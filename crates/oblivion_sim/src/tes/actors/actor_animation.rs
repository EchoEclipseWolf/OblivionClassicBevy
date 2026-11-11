//! An actor's animation data: `00473EB0` ActorAnimData, its +0x9C map of anim-group sequences, and the setup `00475D80` runs over a KF list (`00474070`, `00472030`, `00470BF0`'s variant pick).

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::anim_group::{ANIM_GROUPS, TesAnimGroup};
use crate::bs::anim_group_sequence::{
    AnimGroupSequence, new_anim_group_sequence, section_scaled_time,
};
use crate::bs::kf_model::{KfModelId, kf_model, load_kf_now};
use crate::bs::model_loader::ModelBuildServices;
use crate::bs::task_manager::TaskGlobals;
use crate::gamebryo::controller_sequence::{
    ControllerManagerAddStatus, ControllerSequence, activate_sequence, add_sequence,
    deactivate_sequence, manager_accum_root, new_controller_manager,
};
use crate::gamebryo::object_net::NiObjectNet;
use crate::gamebryo::scene_node::SceneNodePayload;
use crate::gamebryo::stream_objects::NiClass;
use crate::gamebryo::transform_animation::{
    QuatTransform, TransformUpdateStatus, update_transform_interpolator,
};
use crate::gamebryo::transform_controller::NodeController;

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
    pub current_idle_cc: Option<Entity>,
    pub queued_idle_d0: Option<Entity>,
    /// +0xD4 and +0xD8, whose +0x10 sequences 00474070 keeps in the manager.
    pub retired_idle_d4: Option<Entity>,
    pub second_retired_idle_d8: Option<Entity>,
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
            current_idle_cc: None,
            queued_idle_d0: None,
            retired_idle_d4: None,
            second_retired_idle_d8: None,
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
) -> Result<Option<Entity>, GameError> {
    match entry {
        AnimSequence::Single(sequence) => Ok(*sequence),
        AnimSequence::Multiple(list) => {
            let mut at = index as i32 as u32;
            if index == -1 || at >= list.len() as u32 {
                let draw =
                    services
                        .main_random_next
                        .as_deref_mut()
                        .ok_or(GameError::unclassified(
                            0x00470C0F,
                            "00470C0F: Random_Next with no main-thread CRT",
                        ))?();
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

fn group_of(world: &World, group: Entity) -> &TesAnimGroup {
    world.get::<TesAnimGroup>(group).expect("a TESAnimGroup")
}

/// IDA: 00474070 ActorAnimData_AddKF(kf, defer). A null group answers false; group 255 warns "Animation sequence '%s' not found in TESAnimGroup::GetSequenceType" (none can be). The +0x9C entry for the group, a new Single when none. An entry with a sequence (Single's vt+0x0C true, vt+0x10): a group that may be multiple becomes a Multiple of it (004708B0, 00473D90, 00470820); otherwise the same KF group answers true at once, and the old sequence leaves the manager (006C4A10, unless +0xD4's or +0xD8's) and the sections, and the entry's cleared. A deferred KF of a group but Idle, Death (and TorchIdle in RaceSexMenu) goes to +0xB4 (`deferred_b4`) and answers 1. Otherwise a BSAnimGroupSequence joins the entry and the manager (AddSequence with StoreTargets): the accum root from the first, then 00472640. A refused AddSequence warns, removes it, and drops an emptied entry. Completion: 100% of what the startup actors reach; DynamicIdle's KF release (00438540) and a refused AddSequence's per-item warnings are stops.
/// # Errors
/// As above.
pub fn add_kf(
    world: &mut World,
    actor: Entity,
    anim_entity: Entity,
    kf: KfModelId,
    defer: bool,
    services: &mut ModelBuildServices<'_>,
) -> Result<bool, GameError> {
    let (group, kf_sequence) = {
        let model = kf_model(world, kf).ok_or(GameError::unclassified(
            0x00474070,
            "00474070: a released KFModel",
        ))?;
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
                return Err(GameError::unclassified(
                    0x0047415B,
                    "0047415B: 00438540, a DynamicIdle's KF released by name",
                ));
            }
            let kept = {
                let state = anim_mut(world, anim_entity);
                let held = |slot: Option<Entity>| slot.is_some();
                held(state.retired_idle_d4) || held(state.second_retired_idle_d8)
            };
            if kept {
                return Err(GameError::unclassified(
                    0x00474180,
                    "00474180: +0xD4 / +0xD8's sequence test",
                ));
            }
            // 006C4A10 NiControllerManager_RemoveSequence isn't ported: no two KF files of the startup actors share a group that can't be multiple. The sections' clear (+0xA0 null, +0x3C 0) and the entry's vt+4(null) would follow.
            return Err(GameError::unclassified(
                0x004741A4,
                "004741A4: 006C4A10, a second KF for a single group",
            ));
        }
    }
    // 004742D9..: a deferred KF of a group other than Idle (0) and Death (32), and not TorchIdle (33) while the RaceSexMenu is
    // visible (Menu_IsVisibleWithStateMask(1036, 0)), is appended to +0xB4 (PointerList_AppendNonNull) and answers 1.
    let race_menu = world
        .get_resource::<crate::tes::interface_manager::InterfaceMovieReads>()
        .is_some_and(|reads| reads.tile_menu_present(1036));
    if defer && kind != 0 && !(race_menu && kind == 33) && kind != 32 {
        anim_mut(world, anim_entity).deferred_b4.push(kf);
        return Ok(true);
    }
    let source = kf_sequence.ok_or(GameError::unclassified(
        0x0049FD9E,
        "0049FD9E: a KF group with no sequence",
    ))?;
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
        .ok_or(GameError::unclassified(
            0x00474340,
            "00474340: AddSequence on a null manager",
        ))?;
    match add_sequence(world, manager, Some(sequence), None, true)? {
        ControllerManagerAddStatus::Added => {}
        ControllerManagerAddStatus::AlreadyManaged => {
            return Err(GameError::unclassified(
                0x00474372,
                "00474372: a refused AddSequence's warnings",
            ));
        }
        ControllerManagerAddStatus::UnsupportedStoreTargets => {
            return Err(GameError::unclassified(
                0x006C9590,
                "006C9590: StoreTargets on an unported target",
            ));
        }
        _ => {
            return Err(GameError::unclassified(
                0x006C5760,
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
) -> Result<(), GameError> {
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
) -> Result<(), GameError> {
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
            let interpolator = item.interpolator_00.ok_or(GameError::unclassified(
                0x004720C9,
                "004720C9: the accum root's item with no interpolator",
            ))?;
            if world.get::<NiClass>(interpolator) != Some(&NiClass::TransformInterpolator) {
                return Err(GameError::unclassified(
                    0x004720D7,
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
                return Err(GameError::unclassified(
                    0x004720E4,
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

/// IDA: 00B33C28, the deferred KF adds 00476D10 has made since `00440400` last zeroed it; that runs from the idle frame only
/// outside menu mode (0040DBA5), which the port doesn't run yet, so in the race menu it only counts up.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnimationCloneCount {
    pub count_b33c28: u32,
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
fn kf_defer(world: &World, node: Entity) -> Result<bool, GameError> {
    let Some(player) = crate::tes::player::the_player(world) else {
        return Ok(false);
    };
    let root = world
        .get::<crate::tes::reference_3d::Reference3D>(player)
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
            return Err(GameError::unclassified(
                0x00475F2E,
                "00475F2E: the player's +0x5D8 biped root",
            ));
        }
        return Ok(world
            .get_resource::<crate::tes::interface_manager::InterfaceMovieReads>()
            .is_some_and(|reads| reads.tile_menu_present(1036)));
    }
    if crate::tes::forms_references::save_load_active(world)?
        || crate::tes::interface_manager::is_menu_mode(world)
    {
        return Ok(false);
    }
    Err(GameError::unclassified(
        0x00475F4F,
        "00475F4F: TES_IsCellTestMode for an actor's KF defer",
    ))
}

/// IDA: 00475D80 ActorAnimData_InitFromKFList(anim, list, node, reference): a null list, node or reference, or an Idle (group 0) already present, answers false. +0x04 the node; the five bones (004D96F0); the node's +0x18 bit 1 cleared; +0x98 a new NiControllerManager(node, 1). Each KF file in list order: 00439FF0's KFModel; one with a group, unless an NPC's power attack (0051AAA0: groups 22..26, whose KF is released), goes to 00474070 with `kf_defer`'s argument (00B06548 iAnimationClonePerLoop and the player's nodes). Then the accum root's rotation is the identity, and the Idle (its vt+0x10(-1)) is activated (100, start over, 1.0), the node updated at 0.0 with its controllers, and deactivated. Answers whether the Idle's there. Completion: 100%.
/// # Errors
/// As above.
pub fn init_from_kf_list(
    world: &mut World,
    anim_entity: Entity,
    files: Option<Vec<String>>,
    node: Option<Entity>,
    reference: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<bool, GameError> {
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
    let is_npc = base.is_some_and(|base| crate::tes::forms_init::form_type(world, base) == 0x23);
    const BONES: [&[u8]; 5] = [
        b"Bip01",
        b"Bip01 L ForearmTwist",
        b"Torch",
        b"Weapon",
        b"Bip01 Head",
    ];
    let bones = BONES.map(|name| crate::tes::reference_3d::find_bone(world, reference, node, name));
    {
        let mut anim = world
            .get_mut::<ActorAnimData>(anim_entity)
            .expect("an ActorAnimData");
        anim.node_04 = Some(node);
        anim.bones_24 = bones;
    }
    world
        .get_mut::<SceneNodePayload>(node)
        .ok_or(GameError::unclassified(
            0x00475E1B,
            "00475E1B: the skeleton's root isn't a NiNode",
        ))?
        .effective_av_flags &= !2;
    let manager = new_controller_manager(world, node, true)?;
    world
        .get_mut::<ActorAnimData>(anim_entity)
        .expect("an ActorAnimData")
        .manager_98 = Some(manager);
    let defer = kf_defer(world, node);
    for path in files {
        let kf = load_kf_now(world, path.as_bytes(), services)?.ok_or(GameError::unclassified(
            0x00475EDB,
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
        add_kf(world, reference, anim_entity, kf, defer?, services)?;
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
        crate::tes::reference_3d::update_with_controllers(world, node, 0.0).map_err(|_| {
            GameError::unclassified(
                0x00475FE9,
                "00475FE9: the Idle's update (a controller or interpolator not ported)",
            )
        })?;
        deactivate_sequence(world, sequence, 0.0, false);
    }
    Ok(true)
}

// IDA: an actor's vt+0xEC: Character 00611D00 Actor_GetHeightScale (the race height), Creature 004D7260 TESObjectREFR_GetScale
// (+0x38 times the TESCreature's +0x114).
pub(in crate::tes) fn actor_scale(world: &World, actor: Entity) -> Result<f32, GameError> {
    if world.get::<super::actor::Character>(actor).is_some() {
        return crate::tes::actor::actor_height_scale(world, actor);
    }
    let refr = world
        .get::<super::actor::TesObjectRefr>(actor)
        .expect("a reference");
    let creature = refr
        .base_form_1c
        .and_then(|base| world.get::<crate::tes::forms_actors::TesCreature>(base));
    Ok(creature.map_or(refr.scale_38, |creature| creature.scale_114 * refr.scale_38))
}

// IDA: N/A a node's translate (+0x54..+0x5C).
fn translate_of(world: &World, node: Entity) -> Option<[f32; 3]> {
    world
        .get::<SceneNodePayload>(node)
        .map(|payload| payload.local_transform.translation)
}

/// IDA: 00476D10's first arm (00476D2A..00476D92): with deferred KF files (+0xB4/+0xB8), while 00B33C28 is below 00B06548
/// iAnimationClonePerLoop, 00474070(the list's head, 0), the head popped, and with the list then empty (00471730) the loop ends
/// before 00B33C28 counts up. With an add to make and no model services (a caller that runs none) it's a stop. Completion: 100%.
/// # Errors
/// An add with no services, or a stop in 00474070.
pub fn run_deferred_kf_adds(
    world: &mut World,
    anim: Entity,
    actor: Entity,
    mut services: Option<&mut ModelBuildServices<'_>>,
) -> Result<(), GameError> {
    let limit = world
        .get_resource::<AnimationCloneSettings>()
        .copied()
        .unwrap_or_default()
        .animation_clone_per_loop_b06548;
    loop {
        let head = world
            .get::<ActorAnimData>(anim)
            .ok_or(GameError::unclassified(
                0x00476D10,
                "00476D10: not an ActorAnimData",
            ))?
            .deferred_b4
            .first()
            .copied();
        let Some(head) = head else {
            return Ok(());
        };
        if world
            .get_resource_or_init::<AnimationCloneCount>()
            .count_b33c28
            >= limit
        {
            return Ok(());
        }
        let services = services.as_deref_mut().ok_or(GameError::unclassified(
            0x00476D35,
            "00476D35: a deferred KF add with no model services",
        ))?;
        add_kf(world, actor, anim, head, false, services)?;
        let mut state = anim_mut(world, anim);
        state.deferred_b4.remove(0);
        if state.deferred_b4.is_empty() {
            return Ok(());
        }
        world.resource_mut::<AnimationCloneCount>().count_b33c28 += 1;
    }
}

/// IDA: 00476D10 ActorAnimData_Update(anim, actor, delta, time) for an actor whose sections hold no sequence, as 004E3490's
/// arms leave it after 00475D80: deferred KF files (+0xB4/+0xB8) are added up to iAnimationClonePerLoop (`AnimationCloneCount`);
/// +0xC8's idle and the +0xD4 / +0xD8 slots are stops; no
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
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    run_deferred_kf_adds(world, anim, actor, Some(services))?;
    let data = world
        .get::<ActorAnimData>(anim)
        .expect("an ActorAnimData")
        .clone();
    if data.field_c8 != 0 {
        return Err(GameError::unclassified(
            0x00476D6B,
            "00476D6B: +0xC8's idle object",
        ));
    }
    if data.retired_idle_d4.is_some() || data.second_retired_idle_d8.is_some() {
        return Err(GameError::unclassified(
            0x00476DB0,
            "00476DB0: 00472ED0 on +0xD4 / +0xD8",
        ));
    }
    let Some(node) = data.node_04 else {
        return Ok(());
    };
    if data.byte_90 == 5 {
        return Err(GameError::unclassified(
            0x00476DE4,
            "00476DE4: +0x90 == 5, every section held",
        ));
    }
    let busy = (0..5).any(|section| {
        data.groups_3c[section] as u8 != 0xFF
            || data.queued_groups_70[section] as u8 != 0xFF
            || data.sequences_a0[section].is_some()
    });
    if busy {
        return Err(GameError::unclassified(
            0x00476E20,
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
        crate::tes::reference_3d::update_with_controllers(world, node, time)?;
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
            return Err(GameError::unclassified(
                0x00477500,
                "00477500: a mounted actor's sleep state (004728C0)",
            ));
        }
        let process = crate::tes::actor::process_of(world, actor);
        if process
            .and_then(|process| world.get::<super::actor_process::HighProcess>(process))
            .is_some_and(|high| high.movement_flags_1fc & 0xF != 0)
        {
            return Err(GameError::unclassified(
                0x0047756A,
                "0047756A: directional movement's root motion",
            ));
        }
        let before = data.position_18;
        let first = world
            .get::<crate::gamebryo::scene_node::NiNodeChildren>(accum)
            .and_then(|children| children.slots_ac.first().copied().flatten())
            .ok_or(GameError::unclassified(
                0x0047753F,
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
            &mut crate::tes::reference_3d::LoadUpdateControllers,
        )?;
        if result.status != crate::gamebryo::scene_update::SceneUpdateStatus::Updated {
            return Err(GameError::unclassified(
                0x00477869,
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
        .current_idle_cc
        .is_some()
    {
        return Err(GameError::unclassified(
            0x004779A8,
            "004779A8: +0xCC's idle state",
        ));
    }
    world
        .get_mut::<ActorAnimData>(anim)
        .expect("read above")
        .byte_90 = 0xFF;
    Ok(())
}

// IDA: 00476F96..004774B0 (LABEL_46) one section's step in 00476D10 with +0x90 = 0xFF: an active (state 1) sequence of a
// group from 3 to 26 (the movement speeds +0xBC/+0xC0) is a stop; kinds 0 and 1 with a looping sequence (+0x24 = 0): while
// the section's action (+0x48) is below 1 and the group's action time (0051AE20) is below +0x48 + the clock, the action
// becomes 1; then with no queued group, an action-1 time at or before it with loops left (+0x5C not 0) wraps: action 0,
// +0x48 -= end - begin (+0x30 - +0x2C, each sum stored as a float), +0x5C counted down unless -1; an action-1 time at or
// before it with no loops left runs `004733A0` (a stop). A queued group, another kind or a non-looping cycle are stops.
fn section_advance(
    world: &mut World,
    anim: Entity,
    section: usize,
    sequence: Entity,
    group: u16,
    _delta: f32,
) -> Result<(), GameError> {
    let seq = world
        .get::<ControllerSequence>(sequence)
        .expect("a sequence")
        .clone();
    // 00476FAE: only an animating (state 1) sequence takes the speeds and the kinds' switch.
    if seq.state_44 != 1 {
        return Ok(());
    }
    let kind_row = usize::from(group & 0xFF);
    if (3..=26).contains(&kind_row) {
        return Err(GameError::unclassified(
            0x00476FC6,
            "00476FC6: 00476D10's movement speed arms",
        ));
    }
    let kind = super::anim_group::ANIM_GROUPS
        .get(kind_row)
        .ok_or(GameError::unclassified(
            0x00476FDE,
            "00476FDE: a group past the table",
        ))?
        .kind_0c;
    if kind > 1 {
        return Err(GameError::unclassified(
            0x00476FE8,
            "00476FE8: 00476D10's kinds 2..7",
        ));
    }
    if seq.cycle_type_24 != 0 {
        return Err(GameError::unclassified(
            0x00477000,
            "00477000: 00476D10's non-looping kind 0/1",
        ));
    }
    let group_entity = world
        .get::<AnimGroupSequence>(sequence)
        .ok_or(GameError::unclassified(
            0x00477388,
            "00477388: a section sequence with no TESAnimGroup",
        ))?
        .group_68;
    let anim_group = world
        .get::<super::anim_group::TesAnimGroup>(group_entity)
        .expect("the sequence's group")
        .clone();
    let mut warnings = Vec::new();
    let mut time_of = |action: i32| {
        super::anim_group::action_time(&anim_group, action, &mut |line| warnings.push(line))
    };
    let state = world
        .get::<ActorAnimData>(anim)
        .expect("an ActorAnimData")
        .clone();
    let clock = state.time_94;
    let now = |offset: f32| oblivion_core::x87::stored_float(f64::from(offset) + f64::from(clock));
    let mut action = state.actions_48[section];
    while action < 1 {
        if time_of(action) >= now(seq.offset_48) {
            break;
        }
        if (action as u32) <= 1 {
            action = 1;
        }
    }
    if state.queued_groups_70[section] & 0xFF != 0xFF {
        return Err(GameError::unclassified(
            0x00477420,
            "00477420: 00476D10's queued group after a loop",
        ));
    }
    let end = time_of(1);
    let mut offset = seq.offset_48;
    let mut loops = state.loops_5c[section];
    if end > now(offset) || loops == 0 {
        if time_of(1) <= now(offset) {
            return Err(GameError::unclassified(
                0x004774A6,
                "004774A6: 004733A0, a group's end with no loops left",
            ));
        }
    } else {
        action = 0;
        let span = -oblivion_core::x87::stored_float(
            f64::from(seq.end_key_time_30) - f64::from(seq.begin_key_time_2c),
        );
        let local =
            oblivion_core::x87::stored_float(f64::from(offset) - f64::from(seq.begin_key_time_2c));
        let local = oblivion_core::x87::stored_float(f64::from(span) + f64::from(local));
        offset =
            oblivion_core::x87::stored_float(f64::from(local) + f64::from(seq.begin_key_time_2c));
        if loops != -1 {
            loops -= 1;
        }
    }
    if !warnings.is_empty() {
        return Err(GameError::unclassified(
            0x0051AE20,
            "0051AE20: an action time's warning in 00476D10",
        ));
    }
    world
        .get_mut::<ControllerSequence>(sequence)
        .expect("a sequence")
        .offset_48 = offset;
    let mut state = world
        .get_mut::<ActorAnimData>(anim)
        .expect("an ActorAnimData");
    state.actions_48[section] = action;
    state.loops_5c[section] = loops;
    Ok(())
}

/// IDA: 00476D10 ActorAnimData_Update(actor, time, override) with `override` -1.0, the arms the player's data takes under the
/// RaceSexMenu (005CE60B): deferred KFs (+0xB4/+0xB8) through `run_deferred_kf_adds` (none once 00B33C28 reaches the limit, as
/// it does in menu mode, where nothing zeroes it) and no +0xC8 actor; +0xD4/+0xD8 null; +0x04 null returns; +0x90 not
/// 5; +0x00 = 1 and +0x0C..+0x14 = 0; the five sections with no group (low byte 0xFF at +0x3C and +0x70) and so no 0049F4A0
/// time; +0x94 += time (a float, 24-bit); the per-section walk skips every section. With an accum root (+0x08): the actor's
/// directional movement (`Actor_HasDirectionalMovement`, the process's GetMovementFlags & 0xF: the port's player has none),
/// the actor's scale read as +0x38 (vt GetScale, the race's height not applied: the player's is 1.0),
/// the accum root's +0x54 = +0x18, `0047C990` NiAVObject_UpdateRootAndAccumRoot (`update_root_and_accum`: the root's
/// controllers but the multi-target ones, and the UpdateWorldData calls), +0x18 = the accum root's +0x54, the accum root's +0x54 = 0, the movement = (+0x18 - the old +0x18) x the actor's scale, and +0x14 = the
/// first child's +0x5C less its value before (0). The tail's walk finds no section sequence; +0xCC null; +0x90 = 0xFF.
/// Completion: 100% of these arms; any other is a stop.
/// TODO: [ ] 00476D10's section arms (a group set, a queued group, 0049F4A0's times, 0051AF70's notes), the deferred KF load, +0xC8, +0xD4/+0xD8, +0xCC and an override time.
/// # Errors
/// An unported arm.
pub fn anim_data_update_idle(
    world: &mut World,
    anim: Entity,
    actor: Entity,
    time: f32,
) -> Result<(), GameError> {
    let data = world
        .get::<ActorAnimData>(anim)
        .ok_or(GameError::unclassified(
            0x00476D10,
            "00476D10: no animation data",
        ))?
        .clone();
    run_deferred_kf_adds(world, anim, actor, None)?;
    if data.field_c8 != 0 {
        return Err(GameError::unclassified(
            0x00476D8C,
            "00476D8C: 00476D10's +0xC8 actor",
        ));
    }
    if data.retired_idle_d4.is_some() || data.second_retired_idle_d8.is_some() {
        return Err(GameError::unclassified(
            0x00476DBE,
            "00476DBE: 00476D10's +0xD4/+0xD8 sequences",
        ));
    }
    if data.node_04.is_none() {
        return Ok(());
    }
    if data.byte_90 == 5 {
        return Err(GameError::unclassified(
            0x00476E0A,
            "00476E0A: 00476D10 with +0x90 = 5",
        ));
    }
    {
        let mut data = world.get_mut::<ActorAnimData>(anim).expect("read above");
        data.updated_00 = 1;
        data.movement_0c = [0.0; 3];
    }
    // 00476E2E..00476F5A: each section's scaled time before the step (0049F4A0, which stores it) and its sequence.
    let mut before = [(0.0_f32, None::<Entity>); 5];
    for (section, slot) in before.iter_mut().enumerate() {
        let group = data.groups_3c[section] & 0xFF;
        let queued = data.queued_groups_70[section] & 0xFF;
        if group == 0xFF && queued != 0xFF {
            return Err(GameError::unclassified(
                0x00476E40,
                "00476E40: 00476D10's queued group",
            ));
        }
        let sequence = data.sequences_a0[section];
        *slot = (0.0, sequence);
        if group != 0xFF
            && let Some(sequence) = sequence
        {
            slot.0 = section_scaled_time(world, sequence, data.time_94);
        }
    }
    // 00476F0A: +0x94 + time, stored as a float.
    let clock = oblivion_core::x87::stored_float(f64::from(data.time_94) + f64::from(time));
    world
        .get_mut::<ActorAnimData>(anim)
        .expect("read above")
        .time_94 = clock;
    for section in 0..5 {
        let group = data.groups_3c[section];
        let Some(sequence) = data.sequences_a0[section] else {
            continue;
        };
        if group & 0xFF == 0xFF {
            continue;
        }
        section_advance(world, anim, section, sequence, group, time)?;
    }
    if let Some(accum) = data.accum_root_08 {
        // 00476FED Actor_HasDirectionalMovement: the process's GetMovementFlags & 0xF, which nothing the port runs sets for
        // the player (a host reading: the port's process holds no movement flags), so the movement comes from the translate.
        let child_z = |world: &World| {
            world
                .get::<crate::gamebryo::scene_node::NiNodeChildren>(accum)
                .and_then(|children| children.slots_ac.first().copied().flatten())
                .and_then(|child| world.get::<SceneNodePayload>(child))
                .map_or(0.0, |payload| payload.local_transform.translation[2])
        };
        let before_z = child_z(world);
        let old = data.position_18;
        if let Some(mut payload) = world.get_mut::<SceneNodePayload>(accum) {
            payload.local_transform.translation = old;
        }
        let node = data.node_04.expect("checked above");
        let result = crate::gamebryo::scene_update::update_root_and_accum(
            world,
            node,
            clock,
            Some(accum),
            &mut crate::tes::reference_3d::LoadUpdateControllers,
        )?;
        if result.status != crate::gamebryo::scene_update::SceneUpdateStatus::Updated {
            return Err(GameError::unclassified(
                0x00477869,
                "00477869: 0047C990's controller update stopped",
            ));
        }
        let after = world
            .get::<SceneNodePayload>(accum)
            .map_or(old, |payload| payload.local_transform.translation);
        if let Some(mut payload) = world.get_mut::<SceneNodePayload>(accum) {
            payload.local_transform.translation = [0.0; 3];
        }
        let scale = world
            .get::<super::actor::TesObjectRefr>(actor)
            .map_or(1.0, |refr| refr.scale_38);
        let delta = [after[0] - old[0], after[1] - old[1], after[2] - old[2]];
        let after_z = child_z(world);
        let mut data = world.get_mut::<ActorAnimData>(anim).expect("read above");
        data.position_18 = after;
        data.movement_0c = [
            oblivion_core::x87::stored_float(f64::from(scale) * f64::from(delta[0])),
            oblivion_core::x87::stored_float(f64::from(delta[1]) * f64::from(scale)),
            oblivion_core::x87::stored_float(f64::from(after_z) - f64::from(before_z)),
        ];
    }
    // 004779C2..00477AC1: with the root not culled (+0x18 bit 0), each section whose sequence is still the one it had and is
    // animating (state 1, 2 or 5) and holds the highest priority for the section's bone (00475D80's +0x24..+0x34; 0049FD20)
    // takes its scaled time again (0049F4A0, which stores the same values: the clock hasn't moved since) and, when it
    // changed, the group's notes between the two (0051AF70). A group with notes is a stop.
    let data = world
        .get::<ActorAnimData>(anim)
        .expect("read above")
        .clone();
    for (section, (_, sequence)) in before.iter().enumerate() {
        let Some(sequence) = *sequence else {
            continue;
        };
        if data.sequences_a0[section] != Some(sequence) {
            continue;
        }
        let state = world
            .get::<ControllerSequence>(sequence)
            .map_or(0, |seq| seq.state_44);
        if !matches!(state, 1 | 2 | 5) {
            continue;
        }
        let notes = world
            .get::<AnimGroupSequence>(sequence)
            .and_then(|seq| world.get::<super::anim_group::TesAnimGroup>(seq.group_68))
            .is_some_and(|group| !group.notes_28.is_empty());
        if notes {
            return Err(GameError::unclassified(
                0x00477AB1,
                "00477AB1: 0051AF70's notes for a playing group",
            ));
        }
    }
    if data.current_idle_cc.is_some() {
        return Err(GameError::unclassified(
            0x004779A0,
            "004779A0: 00476D10's +0xCC idle",
        ));
    }
    world
        .get_mut::<ActorAnimData>(anim)
        .expect("read above")
        .byte_90 = 0xFF;
    Ok(())
}

/// IDA: 00475B60 ActorAnimData's destructor, then FormHeap_Free: the +0xCC..+0xD8 sequence entries released (`00472ED0`,
/// held by the port as current, queued and retired idle entries), the +0x98 controller manager (`release_actor_manager`) released, the +0x9C
/// map's entries destroyed (`004709C0`) and emptied, the +0xB8 list freed, +0xB4 = 0, +0x08 = 0. Completion: 100% of the
/// port's fields; the entry objects and the manager have no host destructor beyond their removal.
pub fn destroy_actor_anim_data(world: &mut World, anim: Entity) {
    let manager = world
        .get::<ActorAnimData>(anim)
        .and_then(|data| data.manager_98);
    if let Some(manager) = manager {
        crate::gamebryo::controller_sequence::release_actor_manager(world, manager);
    }
    world.entity_mut(anim).despawn();
}
