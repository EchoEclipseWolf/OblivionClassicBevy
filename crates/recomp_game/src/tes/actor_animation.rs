//! An actor's animation data: `00473EB0` ActorAnimData, its +0x9C map of anim-group sequences, and the setup `00475D80` runs over a KF list (`00474070`, `00472030`, `00470BF0`'s variant pick).

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::anim_group::{ANIM_GROUPS, TesAnimGroup};
use crate::bs::kf_model::{KfModelId, kf_model, load_kf_now};
use crate::bs::model_loader::{ModelBuildServices, object_by_name};
use crate::bs::task_manager::TaskGlobals;
use crate::gamebryo::av_object_palette::{DefaultAvObjectPalette, rebuild_from_scene};
use crate::gamebryo::clone::CloningProcess;
use crate::gamebryo::controller_sequence::{
    ControllerManager, ControllerManagerAddStatus, ControllerSequence, activate_sequence,
    add_sequence, deactivate_sequence,
};
use crate::gamebryo::object_net::NiObjectNet;
use crate::gamebryo::scene_node::SceneNodePayload;
use crate::gamebryo::stream_objects::NiClass;
use crate::gamebryo::transform_animation::{
    QuatTransform, TransformUpdateStatus, update_transform_interpolator,
};
use crate::gamebryo::transform_controller::{NodeController, spawn_node_controller};

/// IDA: 00470BF0's draw sites, in order, for a test to compare with a run's (each `Random_Next` at 00470C14): the actor, the group and the draw.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct AnimationVariantDraws {
    pub draws: Vec<(Entity, u16, i32)>,
}

/// IDA: an entry of ActorAnimData +0x9C's map: `AnimSequenceSingle` (vtable A3C72C, 8 bytes: +0x04 the sequence) or `AnimSequenceMultiple` (A3C758: +0x04 an NiTList of sequences, appended at the tail by 00471930).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnimSequence {
    Single(Option<Entity>),
    Multiple(Vec<Entity>),
}

/// IDA: 00473EB0 ActorAnimData (0xDC). Completion: the fields 00475D80, 00474070 and 00472030 use; the rest keep the constructor's values under their offsets.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ActorAnimData {
    /// +0x00 (00476D10 sets 1).
    pub updated_00: u32,
    /// +0x04 the skeleton's root (00475D80).
    pub node_04: Option<Entity>,
    /// +0x08 the accum root (00471600 after the first sequence).
    pub accum_root_08: Option<Entity>,
    /// +0x0C..+0x14 the movement 00476D10 computes.
    pub movement_0c: [f32; 3],
    /// +0x18..+0x20 the accum root's last translate.
    pub position_18: [f32; 3],
    /// +0x24..+0x34 00475D80's bones: Bip01, Bip01 L ForearmTwist, Torch, Weapon, Bip01 Head.
    pub bones_24: [Option<Entity>; 5],
    pub field_38: f32,
    /// +0x3C the five sections' groups (0xFFFF none).
    pub groups_3c: [u16; 5],
    pub actions_48: [i32; 5],
    pub loops_5c: [i32; 5],
    /// +0x70 the five sections' queued groups.
    pub queued_groups_70: [u16; 5],
    pub next_loops_7c: [i32; 5],
    pub byte_90: u8,
    pub time_94: f32,
    /// +0x98 the NiControllerManager (00475D80's 006C5610).
    pub manager_98: Option<Entity>,
    /// +0x9C NiTPointerMap<u16, AnimSequenceBase*> (101 buckets, 00470A40: key % 101; an insert goes to its bucket's head).
    pub sequences_9c: Vec<Vec<(u16, AnimSequence)>>,
    /// +0xA0..+0xB0 the five sections' sequences.
    pub sequences_a0: [Option<Entity>; 5],
    /// +0xB4 the KF files 00474070 deferred (a PointerList, appended).
    pub deferred_b4: Vec<KfModelId>,
    pub speed_bc: f32,
    pub speed_c0: f32,
    pub byte_c4: u8,
    pub field_c8: u32,
    pub field_cc: u32,
    pub field_d0: u32,
    /// +0xD4 and +0xD8, whose +0x10 sequences 00474070 keeps in the manager.
    pub field_d4: Option<Entity>,
    pub field_d8: Option<Entity>,
}

impl Default for ActorAnimData {
    /// IDA: 00473EB0's stores.
    fn default() -> Self {
        Self {
            updated_00: 0,
            node_04: None,
            accum_root_08: None,
            movement_0c: [0.0; 3],
            position_18: [0.0; 3],
            bones_24: [None; 5],
            field_38: 0.0,
            groups_3c: [0xFFFF; 5],
            actions_48: [-1; 5],
            loops_5c: [-1; 5],
            queued_groups_70: [0xFFFF; 5],
            next_loops_7c: [-1; 5],
            byte_90: 0xFF,
            time_94: 0.0,
            manager_98: None,
            sequences_9c: vec![Vec::new(); SEQUENCE_BUCKETS],
            sequences_a0: [None; 5],
            deferred_b4: Vec::new(),
            speed_bc: 1.0,
            speed_c0: 1.0,
            byte_c4: 0,
            field_c8: 0,
            field_cc: 0,
            field_d0: 0,
            field_d4: None,
            field_d8: None,
        }
    }
}

const SEQUENCE_BUCKETS: usize = 101;

impl ActorAnimData {
    /// IDA: 00470960 the map's lookup (00470A40's bucket, 00470A50's equality). Completion: 100%.
    #[must_use]
    pub fn lookup(&self, group: u16) -> Option<&AnimSequence> {
        self.sequences_9c[usize::from(group) % SEQUENCE_BUCKETS]
            .iter()
            .find(|(key, _)| *key == group)
            .map(|(_, value)| value)
    }

    // IDA: 00470960 for the entry itself.
    fn lookup_mut(&mut self, group: u16) -> Option<&mut AnimSequence> {
        self.sequences_9c[usize::from(group) % SEQUENCE_BUCKETS]
            .iter_mut()
            .find(|(key, _)| *key == group)
            .map(|(_, value)| value)
    }

    // IDA: 00470820 insert-or-assign: an existing key's value replaced (vt+0x10, vt+0x0C), a new node at its bucket's head.
    fn insert(&mut self, group: u16, value: AnimSequence) {
        let bucket = &mut self.sequences_9c[usize::from(group) % SEQUENCE_BUCKETS];
        if let Some(entry) = bucket.iter_mut().find(|(key, _)| *key == group) {
            entry.1 = value;
            return;
        }
        bucket.insert(0, (group, value));
    }

    // IDA: 004708B0 remove by key (vt+0x10 and vt+0x18 on the node).
    fn remove(&mut self, group: u16) -> Option<AnimSequence> {
        let bucket = &mut self.sequences_9c[usize::from(group) % SEQUENCE_BUCKETS];
        let at = bucket.iter().position(|(key, _)| *key == group)?;
        Some(bucket.remove(at).1)
    }
}

/// IDA: vt+0x10 GetSequence(index) of an entry: a Single's +0x04 (00471710); a Multiple's (00470BF0) `index`th in its list, with -1 or an index past the count drawing `Random_Next(0) % count` (00470C14) first. Completion: 100%.
/// # Errors
/// A Multiple's draw with no main-thread CRT.
pub fn get_sequence(
    world: &mut World,
    actor: Entity,
    group: u16,
    entry: &AnimSequence,
    index: i8,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, StreamError> {
    match entry {
        AnimSequence::Single(sequence) => Ok(*sequence),
        AnimSequence::Multiple(list) => {
            let mut at = index as i32 as u32;
            if index == -1 || at >= list.len() as u32 {
                let draw = services
                    .main_random_next
                    .as_deref_mut()
                    .ok_or(StreamError("00470C0F: Random_Next with no main-thread CRT"))?(
                );
                world
                    .get_resource_or_init::<AnimationVariantDraws>()
                    .draws
                    .push((actor, group, draw));
                at = (draw as u32) % (list.len() as u32);
            }
            Ok(list.get(at as usize).copied())
        }
    }
}

// IDA: 00471600(manager): +0x6C (cumulative) clear answers null; otherwise the first nonnull of the +0x40 slots below +0x46, its +0x60 accum root.
fn manager_accum_root(world: &World, manager: Entity) -> Option<Entity> {
    let state = world.get::<ControllerManager>(manager)?;
    if !state.cumulative_6c {
        return None;
    }
    let size = usize::from(state.sequence_size_46);
    let first = state
        .sequence_slots_40
        .iter()
        .take(size)
        .copied()
        .flatten()
        .next()?;
    world.get::<ControllerSequence>(first)?.accum_root_60
}

/// IDA: 006C5610 NiControllerManager(target, cumulative): 00715990's time state, the sequence array (grow 10), the 37-bucket map, +0x6C the flag, +0x70..+0x7C zero, `NiTimeController_SetTarget(target)` and a new NiDefaultAVObjectPalette over the target (006C5360: the scene stored, 00716690's rebuild) in +0x7C. Completion: 100%; SetTarget's chain insertion is `set_controller_target`'s.
/// # Errors
/// As `set_controller_target`.
pub fn new_controller_manager(
    world: &mut World,
    target: Entity,
    cumulative: bool,
) -> Result<Entity, StreamError> {
    let manager = spawn_node_controller(
        world,
        NiClass::ControllerManager,
        ControllerManager {
            cumulative_6c: cumulative,
            ..ControllerManager::default()
        },
    );
    crate::gamebryo::controller_sequence::set_controller_target(world, manager, Some(target));
    let mut palette = DefaultAvObjectPalette::default();
    palette.set_scene(Some(target));
    let palette = world.spawn((NiClass::DefaultAvObjectPalette, palette)).id();
    rebuild_from_scene(world, palette);
    world
        .get_mut::<ControllerManager>(manager)
        .expect("just spawned")
        .object_palette_7c = Some(palette);
    Ok(manager)
}

// IDA: 0049FD90 BSAnimGroupSequence(group, sequence) (FormHeap 0x6C): 006C7FB0(sequence's +0x08 name, 0, 1, 0), vtable A49964, +0x68 the group (counted), +0x20 the text keys, +0x60 the accum root and +0x5C the accum root's name copied, then a cloning process and 006C9420 (the new arrays freed, 006C70A0 NiControllerSequence_CopyMembers from the KF's); +0x60 isn't one CopyMembers writes, so it stays the KF's.
fn new_anim_group_sequence(
    world: &mut World,
    group: Entity,
    source: Entity,
) -> Result<Entity, StreamError> {
    let state = world
        .get::<ControllerSequence>(source)
        .expect("a KF's sequence")
        .clone();
    let sequence = world
        .spawn((
            NiClass::ControllerSequence,
            ControllerSequence {
                name_08: state.name_08.clone(),
                array_grow_by_10: 1,
                text_keys_20: state.text_keys_20,
                accum_root_name_5c: state.accum_root_name_5c.clone(),
                ..ControllerSequence::default()
            },
            AnimGroupSequence { group_68: group },
        ))
        .id();
    let mut process = CloningProcess::default();
    crate::gamebryo::clone_animation::clone_sequence(world, &mut process, source, sequence)?;
    world
        .get_mut::<ControllerSequence>(sequence)
        .expect("just spawned")
        .accum_root_60 = state.accum_root_60;
    Ok(sequence)
}

/// IDA: BSAnimGroupSequence's own field: +0x68 the TESAnimGroup.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimGroupSequence {
    pub group_68: Entity,
}

fn group_of(world: &World, group: Entity) -> &TesAnimGroup {
    world.get::<TesAnimGroup>(group).expect("a TESAnimGroup")
}

/// IDA: 00474070 ActorAnimData_AddKF(kf, defer). A null group answers false; group 255 warns "Animation sequence '%s' not found in TESAnimGroup::GetSequenceType" (none can be). The +0x9C entry for the group, a new Single when none. An entry with a sequence (Single's vt+0x0C true, vt+0x10): a group that may be multiple becomes a Multiple of it (004708B0, 00473D90, 00470820); otherwise the same KF group answers true at once, and the old sequence leaves the manager (006C4A10, unless +0xD4's or +0xD8's) and the sections, and the entry's cleared. A deferred KF of a group but Idle, Death (and TorchIdle in RaceSexMenu) goes to +0xB4. Otherwise a BSAnimGroupSequence joins the entry and the manager (AddSequence with StoreTargets): the accum root from the first, then 00472640. A refused AddSequence warns, removes it, and drops an emptied entry. Completion: 100% of what the startup actors reach; DynamicIdle's KF release (00438540) and a refused AddSequence's per-item warnings are stops.
/// # Errors
/// As above.
pub fn add_kf(
    world: &mut World,
    actor: Entity,
    anim_entity: Entity,
    kf: KfModelId,
    defer: bool,
    services: &mut ModelBuildServices<'_>,
) -> Result<bool, StreamError> {
    let (group, kf_sequence) = {
        let model = kf_model(world, kf).ok_or(StreamError("00474070: a released KFModel"))?;
        (model.anim_group_08, model.sequence_04)
    };
    let Some(group) = group else {
        return Ok(false);
    };
    let (id, kind) = {
        let state = group_of(world, group);
        (state.group_08, state.group_type())
    };
    if anim_mut(world, anim_entity).lookup(id).is_none() {
        anim_mut(world, anim_entity).insert(id, AnimSequence::Single(None));
    }
    let entry = anim_mut(world, anim_entity)
        .lookup(id)
        .cloned()
        .expect("inserted");
    if let AnimSequence::Single(Some(existing)) = entry {
        if ANIM_GROUPS[usize::from(kind)].multiple_04 {
            anim_mut(world, anim_entity).remove(id);
            anim_mut(world, anim_entity).insert(id, AnimSequence::Multiple(vec![existing]));
        } else {
            let existing_group = world
                .get::<AnimGroupSequence>(existing)
                .map(|sequence| sequence.group_68);
            if existing_group == Some(group) {
                if kind == 1 {
                    release_kf(world, kf);
                }
                return Ok(true);
            }
            if existing_group.is_some_and(|other| group_of(world, other).group_type() == 1) {
                return Err(StreamError(
                    "0047415B: 00438540, a DynamicIdle's KF released by name",
                ));
            }
            let kept = {
                let state = anim_mut(world, anim_entity);
                let held = |slot: Option<Entity>| slot.is_some();
                held(state.field_d4) || held(state.field_d8)
            };
            if kept {
                return Err(StreamError("00474180: +0xD4 / +0xD8's sequence test"));
            }
            // 006C4A10 NiControllerManager_RemoveSequence isn't ported: no two KF files of the startup actors share a group that can't be multiple. The sections' clear (+0xA0 null, +0x3C 0) and the entry's vt+4(null) would follow.
            return Err(StreamError(
                "004741A4: 006C4A10, a second KF for a single group",
            ));
        }
    }
    if defer {
        return Err(StreamError(
            "0047429A: a deferred KF (the player's first-person node)",
        ));
    }
    let source = kf_sequence.ok_or(StreamError("0049FD9E: a KF group with no sequence"))?;
    let sequence = new_anim_group_sequence(world, group, source)?;
    match anim_mut(world, anim_entity)
        .lookup_mut(id)
        .expect("present")
    {
        AnimSequence::Single(slot) => *slot = Some(sequence),
        AnimSequence::Multiple(list) => list.push(sequence),
    }
    let manager = anim_mut(world, anim_entity)
        .manager_98
        .ok_or(StreamError("00474340: AddSequence on a null manager"))?;
    match add_sequence(world, manager, Some(sequence), None, true)? {
        ControllerManagerAddStatus::Added => {}
        ControllerManagerAddStatus::AlreadyManaged => {
            return Err(StreamError("00474372: a refused AddSequence's warnings"));
        }
        ControllerManagerAddStatus::UnsupportedStoreTargets => {
            return Err(StreamError("006C9590: StoreTargets on an unported target"));
        }
        _ => {
            return Err(StreamError(
                "006C5760: AddSequence on a null target, name or sequence",
            ));
        }
    }
    if anim_mut(world, anim_entity).accum_root_08.is_none() {
        let root = manager_accum_root(world, manager);
        anim_mut(world, anim_entity).accum_root_08 = root;
    }
    after_add(world, actor, anim_entity, group, services)?;
    Ok(true)
}

// IDA: N/A a helper, with no address of its own.
fn anim_mut(world: &mut World, anim: Entity) -> Mut<'_, ActorAnimData> {
    world
        .get_mut::<ActorAnimData>(anim)
        .expect("an ActorAnimData")
}

// IDA: InterlockedDecrement(KFModel +0x0C).
fn release_kf(world: &mut World, kf: KfModelId) {
    if let Some(model) = world
        .resource_mut::<TaskGlobals>()
        .model_loader_b33a1c
        .as_mut()
        .and_then(|loader| loader.kfs.store.get_mut(kf.0))
    {
        model.references_0c = model.references_0c.wrapping_sub(1);
    }
}

// IDA: 00472640(group): a +0x3C and +0x70 both 0x00FF (a u16 compare) with group 0 store it in +0x70; then 00472030. Completion: 100%.
fn after_add(
    world: &mut World,
    actor: Entity,
    anim_entity: Entity,
    group: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), StreamError> {
    let id = group_of(world, group).group_08;
    {
        let mut anim = world
            .get_mut::<ActorAnimData>(anim_entity)
            .expect("an ActorAnimData");
        if anim.groups_3c[0] == 0xFF && anim.queued_groups_70[0] == 0xFF && id == 0 {
            anim.queued_groups_70[0] = id;
        }
    }
    accum_movement(world, actor, anim_entity, group, services)
}

/// IDA: 00472030 ActorAnimData_InitGroupMovement(group): with a manager, the group's entry and a group but DynamicIdle or SpecialIdle: no accum root (00471600) warns "Could not find AccumRoot in animation '%s'."; otherwise the entry's vt+0x10(-1) (a Multiple draws), and for a group that can't be multiple whose kind is 0, 1, 4, 5 or 6, the accum root's item (the one whose controller's +0x30 target is the sequence's +0x60): its interpolator's vt+0x4C at the begin and end key times, and both translates valid, the difference over the time into the group's +0x14 (004707E0, 004BEF50). A movement group (3..14) with no movement warns "... was exported with 'Animate in Place' from MAX.". Completion: 100% for NiTransformInterpolator items; any other interpolator class there is a stop.
fn accum_movement(
    world: &mut World,
    actor: Entity,
    anim_entity: Entity,
    group: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), StreamError> {
    let (id, kind) = {
        let state = group_of(world, group);
        (state.group_08, state.group_type())
    };
    let (manager, entry, node) = {
        let anim = world
            .get::<ActorAnimData>(anim_entity)
            .expect("an ActorAnimData");
        (anim.manager_98, anim.lookup(id).cloned(), anim.node_04)
    };
    let (Some(manager), Some(entry)) = (manager, entry) else {
        return Ok(());
    };
    if kind == 1 || kind == 2 {
        return Ok(());
    }
    let node_name = node
        .and_then(|node| world.get::<NiObjectNet>(node))
        .and_then(|net| net.name.clone())
        .unwrap_or_default();
    if manager_accum_root(world, manager).is_none() {
        (services.warning)(
            [
                b"Could not find AccumRoot in animation '".as_slice(),
                &node_name,
                b"'.",
            ]
            .concat(),
            *services.disable_warning_messages_b06b18 != 0,
        );
        return Ok(());
    }
    let sequence = get_sequence(world, actor, id, &entry, -1, services)?;
    let info = ANIM_GROUPS[usize::from(kind)];
    if let Some(sequence) = sequence
        && !info.multiple_04
        && matches!(info.kind_0c, 0 | 1 | 4 | 5 | 6)
    {
        let state = world
            .get::<ControllerSequence>(sequence)
            .expect("a sequence")
            .clone();
        let found = state
            .items_14
            .iter()
            .take(state.array_size_0c as usize)
            .find(|item| {
                let target = item
                    .controller_04
                    .and_then(|controller| world.get::<NodeController>(controller))
                    .and_then(|controller| controller.time.target_30);
                target == state.accum_root_60
            });
        if let Some(item) = found {
            let (begin, end) = (state.begin_key_time_2c, state.end_key_time_30);
            let interpolator = item.interpolator_00.ok_or(StreamError(
                "004720C9: the accum root's item with no interpolator",
            ))?;
            if world.get::<NiClass>(interpolator) != Some(&NiClass::TransformInterpolator) {
                return Err(StreamError(
                    "004720D7: an accum root's interpolator not NiTransformInterpolator",
                ));
            }
            let mut first = QuatTransform::default();
            let mut last = QuatTransform::default();
            let a = update_transform_interpolator(world, interpolator, begin, &mut first);
            let b = update_transform_interpolator(world, interpolator, end, &mut last);
            if a == TransformUpdateStatus::UnsupportedOutOfRange
                || b == TransformUpdateStatus::UnsupportedOutOfRange
            {
                return Err(StreamError(
                    "004720E4: the accum root's interpolator out of its keys",
                ));
            }
            let invalid = crate::gamebryo::transform_animation::INVALID_TRANSFORM_COMPONENT;
            if first.translate[0] != invalid && last.translate[0] != invalid {
                let reciprocal = 1.0 / (end - begin);
                let movement = [
                    (last.translate[0] - first.translate[0]) * reciprocal,
                    (last.translate[1] - first.translate[1]) * reciprocal,
                    (last.translate[2] - first.translate[2]) * reciprocal,
                ];
                world
                    .get_mut::<TesAnimGroup>(group)
                    .expect("a TESAnimGroup")
                    .movement_14 = movement;
            }
        }
    }
    let movement = group_of(world, group).movement_14;
    let length =
        (movement[1] * movement[1] + movement[0] * movement[0] + movement[2] * movement[2]).sqrt();
    if kind.wrapping_sub(3) <= 0xB && length == 0.0 {
        let hand = super::anim_group::hand_prefix(usize::from((id >> 8) & 0xF));
        let movement_prefix = super::anim_group::movement_prefix(usize::from(id >> 12));
        let text = [
            b"AnimGroup '".as_slice(),
            movement_prefix,
            hand,
            info.name,
            b"' for '",
            &node_name,
            b"' was exported with 'Animate in Place' from MAX.\r\n",
        ]
        .concat();
        (services.warning)(text, *services.disable_warning_messages_b06b18 != 0);
    }
    Ok(())
}

// IDA: 004D96F0(reference, node, name): a creature base (+0x104 type 4) looks the name up with its fifth byte '2' only; anyone else first as given (006F94A0), then that way.
fn find_bone(world: &World, node: Entity, name: &[u8]) -> Option<Entity> {
    object_by_name(world, node, name).or_else(|| {
        let mut other = name.to_vec();
        if other.len() > 4 {
            other[4] = b'2';
        }
        object_by_name(world, node, &other)
    })
}

/// IDA: 00B06548 iAnimationClonePerLoop:BackgroundLoad, which 00475D80's defer test and 00476D10's deferred adds read live.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimationCloneSettings {
    pub animation_clone_per_loop_b06548: u32,
}

impl Default for AnimationCloneSettings {
    /// IDA: N/A the image value, for a world with no INI.
    fn default() -> Self {
        Self {
            animation_clone_per_loop_b06548: 5,
        }
    }
}

// IDA: 00475EF6..00475F5A the defer argument for 00474070: with a player whose third-person root (00664F40(0)) exists and
// 00B06548 nonzero, a node that is the player's root, +0x5D0 or +0x5D8 defers only while the RaceSexMenu (1036) is visible;
// any other node defers unless a save loads, menu mode holds or the cell test runs. Otherwise nothing defers. The race menu
// is read as present (`InterfaceMovieReads`), and the cell test isn't ported: an actor whose answer turns on it is a stop.
fn kf_defer(world: &World, node: Entity) -> Result<bool, StreamError> {
    let Some(player) = super::ai_conditions::the_player(world) else {
        return Ok(false);
    };
    let root = world
        .get::<super::reference_3d::Reference3D>(player)
        .map(|loaded| loaded.0)
        .filter(|&root| {
            world
                .get::<SceneNodePayload>(root)
                .is_some_and(|payload| payload.effective_av_flags & 1 == 0)
        });
    let clone_per_loop = world
        .get_resource::<AnimationCloneSettings>()
        .copied()
        .unwrap_or_default()
        .animation_clone_per_loop_b06548;
    let Some(root) = root else {
        return Ok(false);
    };
    if clone_per_loop == 0 {
        return Ok(false);
    }
    let data = world
        .get::<super::player::PlayerCharacter>(player)
        .expect("the player");
    if node == root
        || Some(node) == data.first_person_node_5d0
        || data.first_person_biped_root_5d8 != 0
    {
        if data.first_person_biped_root_5d8 != 0 {
            return Err(StreamError("00475F2E: the player's +0x5D8 biped root"));
        }
        return Ok(world
            .get_resource::<super::interface_manager::InterfaceMovieReads>()
            .is_some_and(|reads| reads.tile_menu_present(1036)));
    }
    if super::forms_references::save_load_active(world)? || super::script_vm::is_menu_mode(world) {
        return Ok(false);
    }
    Err(StreamError(
        "00475F4F: TES_IsCellTestMode for an actor's KF defer",
    ))
}

/// IDA: 00475D80 ActorAnimData_InitFromKFList(anim, list, node, reference): a null list, node or reference, or an Idle (group 0) already present, answers false. +0x04 the node; the five bones (004D96F0); the node's +0x18 bit 1 cleared; +0x98 a new NiControllerManager(node, 1). Each KF file in list order: 00439FF0's KFModel; one with a group, unless an NPC's power attack (0051AAA0: groups 22..26, whose KF is released), goes to 00474070 with `kf_defer`'s argument (00B06548 iAnimationClonePerLoop and the player's nodes). Then the accum root's rotation is the identity, and the Idle (its vt+0x10(-1)) is activated (100, start over, 1.0), the node updated at 0.0 with its controllers, and deactivated. Answers whether the Idle's there. Completion: 100%; a deferred add is a stop.
/// # Errors
/// As above.
pub fn init_from_kf_list(
    world: &mut World,
    anim_entity: Entity,
    files: Option<Vec<String>>,
    node: Option<Entity>,
    reference: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<bool, StreamError> {
    let (Some(files), Some(node)) = (files, node) else {
        return Ok(false);
    };
    if world
        .get::<ActorAnimData>(anim_entity)
        .expect("an ActorAnimData")
        .lookup(0)
        .is_some()
    {
        return Ok(false);
    }
    let base = world
        .get::<super::actor::TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c;
    let is_npc = base.is_some_and(|base| super::forms_init::form_type(world, base) == 0x23);
    const BONES: [&[u8]; 5] = [
        b"Bip01",
        b"Bip01 L ForearmTwist",
        b"Torch",
        b"Weapon",
        b"Bip01 Head",
    ];
    let bones = BONES.map(|name| find_bone(world, node, name));
    {
        let mut anim = world
            .get_mut::<ActorAnimData>(anim_entity)
            .expect("an ActorAnimData");
        anim.node_04 = Some(node);
        anim.bones_24 = bones;
    }
    world
        .get_mut::<SceneNodePayload>(node)
        .ok_or(StreamError("00475E1B: the skeleton's root isn't a NiNode"))?
        .effective_av_flags &= !2;
    let manager = new_controller_manager(world, node, true)?;
    world
        .get_mut::<ActorAnimData>(anim_entity)
        .expect("an ActorAnimData")
        .manager_98 = Some(manager);
    let defer = kf_defer(world, node);
    for path in files {
        let kf = load_kf_now(world, path.as_bytes(), services)?.ok_or(StreamError(
            "00475EDB: a KF that didn't load (its +0x08 read on null)",
        ))?;
        let group = kf_model(world, kf).and_then(|model| model.anim_group_08);
        let Some(group) = group else {
            continue;
        };
        let kind = group_of(world, group).group_type();
        if kind.wrapping_sub(22) <= 4 && is_npc {
            release_kf(world, kf);
            continue;
        }
        if defer? {
            return Err(StreamError(
                "00475F5A: a deferred KF add (00474070's +0xB4)",
            ));
        }
        add_kf(world, reference, anim_entity, kf, false, services)?;
    }
    if let Some(accum) = world
        .get::<ActorAnimData>(anim_entity)
        .expect("an ActorAnimData")
        .accum_root_08
        && let Some(mut payload) = world.get_mut::<SceneNodePayload>(accum)
    {
        payload.local_transform.rotation = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    }
    let idle = world
        .get::<ActorAnimData>(anim_entity)
        .expect("an ActorAnimData")
        .lookup(0)
        .cloned();
    let Some(idle) = idle else {
        return Ok(false);
    };
    if let Some(sequence) = get_sequence(world, reference, 0, &idle, -1, services)? {
        activate_sequence(world, sequence, 100, true, 1.0, 0.0, None, false);
        // 00475FE9 NiAVObject_Update(node, 0.0, 1): the bones' blends over the KF's B-splines (006E5990), the accum root's
        // NiBlendAccumTransformInterpolator (006CF780), and the skeleton's bone LOD and bhkBlendController (008AAD60).
        super::reference_3d::update_with_controllers(world, node, 0.0).map_err(|_| {
            StreamError("00475FE9: the Idle's update (a controller or interpolator not ported)")
        })?;
        deactivate_sequence(world, sequence, 0.0, false);
    }
    Ok(true)
}

// IDA: an actor's vt+0xEC: Character 00611D00 Actor_GetHeightScale (the race height), Creature 004D7260 TESObjectREFR_GetScale
// (+0x38 times the TESCreature's +0x114).
fn actor_scale(world: &World, actor: Entity) -> Result<f32, StreamError> {
    if world.get::<super::actor::Character>(actor).is_some() {
        return super::hud_main_menu::actor_height_scale(world, actor);
    }
    let refr = world
        .get::<super::actor::TesObjectRefr>(actor)
        .expect("a reference");
    let creature = refr
        .base_form_1c
        .and_then(|base| world.get::<super::forms_actors::TesCreature>(base));
    Ok(creature.map_or(refr.scale_38, |creature| creature.scale_114 * refr.scale_38))
}

// IDA: N/A a node's translate (+0x54..+0x5C).
fn translate_of(world: &World, node: Entity) -> Option<[f32; 3]> {
    world
        .get::<SceneNodePayload>(node)
        .map(|payload| payload.local_transform.translation)
}

/// IDA: 00476D10 ActorAnimData_Update(anim, actor, delta, time) for an actor whose sections hold no sequence, as 004E3490's
/// arms leave it after 00475D80: deferred KF files (+0xB4/+0xB8), +0xC8's idle and the +0xD4 / +0xD8 slots are stops; no
/// node returns. +0x00 = 1, +0x0C = 0; +0x90 == 5 is a stop. A section with a group or a sequence is a stop. With a time other
/// than -1.0 the node updates at it (NiAVObject_Update, +0x94 = time); otherwise +0x94 += delta and, with an accum root: a
/// Character with a horse (vt+0x380 Character_GetMountedHorse; the original resets the accum root, 004728C0, only in sleep
/// states 2..5) and directional movement (005E05B0: a High process's +0x1FC & 0xF; a MiddleHigh one's is 0) are stops; the accum root takes +0x18 as its translate, 0047C990(node, +0x94, accum), +0x18
/// takes the result and the accum root's translate zeroes; +0x0C = (+0x18 - before) * vt+0xEC's scale, and +0x14 = the accum
/// root's first child's z after less before. The text-key walk finds no section sequence, +0xCC isn't 2, and +0x90 = -1.
/// Completion: 100% of that state.
/// # Errors
/// The arms above, or a stop in the update.
pub fn update_anim_data(
    world: &mut World,
    anim: Entity,
    actor: Entity,
    delta: f32,
    time: f32,
) -> Result<(), StreamError> {
    let data = world
        .get::<ActorAnimData>(anim)
        .ok_or(StreamError("00476D10: not an ActorAnimData"))?
        .clone();
    if !data.deferred_b4.is_empty() {
        return Err(StreamError(
            "00476D2A: the deferred KF adds (00B33C28 against 00B06548 iAnimationClonePerLoop)",
        ));
    }
    if data.field_c8 != 0 {
        return Err(StreamError("00476D6B: +0xC8's idle object"));
    }
    if data.field_d4.is_some() || data.field_d8.is_some() {
        return Err(StreamError("00476DB0: 00472ED0 on +0xD4 / +0xD8"));
    }
    let Some(node) = data.node_04 else {
        return Ok(());
    };
    if data.byte_90 == 5 {
        return Err(StreamError("00476DE4: +0x90 == 5, every section held"));
    }
    let busy = (0..5).any(|section| {
        data.groups_3c[section] as u8 != 0xFF
            || data.queued_groups_70[section] as u8 != 0xFF
            || data.sequences_a0[section].is_some()
    });
    if busy {
        return Err(StreamError(
            "00476E20: a section with a group or a sequence",
        ));
    }
    {
        let mut data = world.get_mut::<ActorAnimData>(anim).expect("read above");
        data.updated_00 = 1;
        data.movement_0c = [0.0; 3];
    }
    if time != -1.0 {
        world
            .get_mut::<ActorAnimData>(anim)
            .expect("read above")
            .time_94 = time;
        super::reference_3d::update_with_controllers(world, node, time)?;
        world
            .get_mut::<ActorAnimData>(anim)
            .expect("read above")
            .byte_90 = 0xFF;
        return Ok(());
    }
    let clock = data.time_94 + delta;
    world
        .get_mut::<ActorAnimData>(anim)
        .expect("read above")
        .time_94 = clock;
    if let Some(accum) = data.accum_root_08 {
        if world.get::<super::actor::Character>(actor).is_some()
            && world
                .get::<super::actor::Actor>(actor)
                .is_some_and(|actor| actor.horse_or_rider_d4 != 0)
        {
            return Err(StreamError(
                "00477500: a mounted actor's sleep state (004728C0)",
            ));
        }
        let process = super::ai_conditions::process_of(world, actor);
        if process
            .and_then(|process| world.get::<super::actor_process::HighProcess>(process))
            .is_some_and(|high| high.movement_flags_1fc & 0xF != 0)
        {
            return Err(StreamError("0047756A: directional movement's root motion"));
        }
        let before = data.position_18;
        let first = world
            .get::<crate::gamebryo::scene_node::NiNodeChildren>(accum)
            .and_then(|children| children.slots_ac.first().copied().flatten())
            .ok_or(StreamError(
                "0047753F: an accum root with no first child faults",
            ))?;
        let child_z = translate_of(world, first).map_or(0.0, |translate| translate[2]);
        if let Some(mut payload) = world.get_mut::<SceneNodePayload>(accum) {
            payload.local_transform.translation = before;
        }
        let result = crate::gamebryo::scene_update::update_root_and_accum(
            world,
            node,
            clock,
            Some(accum),
            &mut super::reference_3d::LoadUpdateControllers,
        )?;
        if result.status != crate::gamebryo::scene_update::SceneUpdateStatus::Updated {
            return Err(StreamError(
                "00477869: 0047C990's controller update stopped",
            ));
        }
        let after = translate_of(world, accum).unwrap_or(before);
        if let Some(mut payload) = world.get_mut::<SceneNodePayload>(accum) {
            payload.local_transform.translation = [0.0; 3];
        }
        let scale = actor_scale(world, actor)?;
        let child_after = translate_of(world, first).map_or(0.0, |translate| translate[2]);
        let mut data = world.get_mut::<ActorAnimData>(anim).expect("read above");
        data.position_18 = after;
        let movement = [
            after[0] - before[0],
            after[1] - before[1],
            after[2] - before[2],
        ];
        data.movement_0c = [
            scale * movement[0],
            movement[1] * scale,
            scale * movement[2],
        ];
        data.movement_0c[2] = child_after - child_z;
    }
    if world
        .get::<ActorAnimData>(anim)
        .expect("read above")
        .field_cc
        != 0
    {
        return Err(StreamError("004779A8: +0xCC's idle state"));
    }
    world
        .get_mut::<ActorAnimData>(anim)
        .expect("read above")
        .byte_90 = 0xFF;
    Ok(())
}
