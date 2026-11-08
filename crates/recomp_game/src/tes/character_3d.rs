//! The actor classes' model children: QueuedCharacter's vt+0x2C (0043DC00) and what it builds before the skeleton's child: the skin info (00523170, 00478730) at Character +0x104, and 005268D0's race and worn slots.
//! Evidence: `evidence/gameplay-run002-2026-11-07/` run-af (the 11 actors' finishes in the drain). Build 2's first slice: the arms past the race slots are named stops (`CharacterTaskStops`).

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::actor::{Character, MobileObject, TesObjectRefr};
use super::actor_enable::{auto_equip, biped_mask};
use super::actor_process::{BaseProcess, MiddleHighProcess};
use super::ai_package::{actor_is_dead, package_data};
use super::cell_reference_loop::equipped_in_slot;
use super::form_components::{TesAnimation, TesBipedModelForm};
use super::forms_actors::TesRace;
use super::forms_init::RaceStatics;
use super::hud_main_menu::{base_race, npc_sex};
use super::reference_3d::change_flags;
use super::reference_load::{ReferenceLoadServices, queued_file_notify_parent};
use super::tes_file::BsStringT;
use crate::bs::model_loader::{ModelRequest, fixup_model_path, load_model};
use crate::bs::reference_tasks::ReferenceTaskId;
use crate::bs::task_manager::TaskGlobals;

/// IDA: one of the skin info's sixteen 16-byte slots at +0x4C: +0x00 the form (a race for the body parts, or a TESBipedModelForm), +0x04 its TESModel, +0x08 the loaded node, +0x0C.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkinSlot {
    pub form_00: Option<Entity>,
    pub model_04: Option<SkinModel>,
    pub node_08: Option<Entity>,
    /// +0x0C the part is skinned (0047B5B0's 00471B80).
    pub skinned_0c: u32,
}

/// IDA: N/A where a slot's TESModel lives: one of 0052B6A0's static body models (by `RaceStatics` index), a race's tail model (+0xB0 + 0x18 * sex), or a worn form's biped model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkinModel {
    RaceStatic(usize),
    RaceTail {
        race: Entity,
        sex: usize,
    },
    /// A TESBipedModelForm's biped model, +0x08 + 0x18 * sex.
    Biped {
        form: Entity,
        sex: usize,
    },
    /// A form's own TESModel (a weapon's +0x30, by 00478CA0).
    Form(Entity),
}

/// IDA: one of the nine bone entries at +0x04 (00478070 from 00B06550's names): +0x00 bit 0 found, +0x04 the node.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkinBone {
    pub flags_00: u32,
    pub node_04: Option<Entity>,
}

/// IDA: the 0x154 skin info 00478730 builds, Character +0x104 (vt+0x168 004349A0 reads it, vt+0x16C 00611B00 stores it): +0x00 the Bip01 node, +0x04 nine bones, +0x4C sixteen slots, +0x150 the owner reference. Host form: an entity of its own, as the original's heap object.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct SkinInfo {
    pub bip01_00: Option<Entity>,
    pub bones_04: [SkinBone; 9],
    pub slots_4c: [SkinSlot; 16],
    pub owner_150: Entity,
}

/// IDA: 00B33C80, the sixteen slots 0047AB90 moves out of a skin info before it refills the body parts (00478730 zeroes it).
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct SkinSlotScratch {
    pub slots_b33c80: [SkinSlot; 16],
}

/// IDA: N/A the arms an actor task reached that Build 2 hasn't ported yet: the task stays queued with no child, as before this slice.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct CharacterTaskStops {
    pub stops: Vec<(Entity, &'static str)>,
    /// The actors whose main-thread finish (0043B090's Load3D) ran to its end, in finish order.
    pub finished: Vec<Entity>,
}

/// IDA: 00B065C8, the bone entry each biped slot needs (-1 for none).
const SLOT_BONE_B065C8: [i32; 16] = [0, 0, -1, -1, -1, -1, 1, 2, -1, 3, 4, 5, 6, 7, 8, -1];
/// IDA: 00B06574..00B06588, the body-part slots 0047AB90 refills: upper body, lower body, hand, foot, tail.
const BODY_SLOTS_B06574: [usize; 5] = [2, 3, 4, 5, 15];

/// IDA: 0043DC00 QueuedCharacter's vt+0x2C: the base (vt+0x170) and the reference's skin info (vt+0x168, else 00523170); with one, 005268D0, 004788E0 into +0x3C and 0043BA30 (a QueuedHead) into +0x38; then 0043D000 (the KF files), 005E4DD0 (the magic resources) and `QueuedReference_QueueModelChild`. Completion: through 004788E0's plain arm; its helmet arm and what follows are stops.
/// # Errors
/// An arm not ported yet, by its site.
pub fn queue_character_children(
    world: &mut World,
    id: ReferenceTaskId,
    reference: Entity,
    base: Entity,
    services: &mut ReferenceLoadServices<'_, '_>,
) -> Result<(), StreamError> {
    let skin = match skin_info(world, reference) {
        Some(skin) => skin,
        None => create_skin_info(world, reference),
    };
    npc_init_biped(world, base, reference, skin)?;
    queue_slot_models(world, id, skin, services)?;
    // 0043DCE6: 0043BA30's QueuedHead (0x30, a child of this task, +0x38): its Run (00435300 -> 00523220) builds the FaceGen
    // head in the walk (`face_gen_tasks`; FaceGen background agent).
    super::face_gen_tasks::create_queued_head(world, base, id)?;
    queue_animation_files(world, base, reference)?;
    queue_magic_resources(world, reference, services)?;
    // 0043DD68: QueuedReference_QueueModelChild, the skeleton, is the caller's.
    Ok(())
}

/// IDA: 0043DE00 QueuedPlayer_QueueChildren: 00662F60 (the base's skeleton model and the first-person animation's files), then
/// 0043DC00 QueuedCharacter_QueueChildren. Completion: as the two.
/// # Errors
/// As 00662F60 and `queue_character_children`.
pub fn queue_player_children(
    world: &mut World,
    id: ReferenceTaskId,
    reference: Entity,
    base: Entity,
    services: &mut ReferenceLoadServices<'_, '_>,
) -> Result<(), StreamError> {
    queue_player_animation_models(world, id, base, services)?;
    queue_character_children(world, id, reference, base, services)
}

// IDA: 00662F60(priority, task) on the player: the base's (vt+0x170) TESModel (+0xAC) vt+0x18 (0046E080, BSStringT_Set) takes
// 00B38B68 sNPCModelBeast when the race (+0xE8) has a tail model (0052BDB0 for the male), else 00B38B70 sNPCModel. Then
// IdleAnimRoot_FindKFFiles(00B36BB8 sFirstPersonSkeleton) through 0043BDA0 ModelLoader_QueueKFList (each 0043B840 under the
// task), ModelLoader_LoadModel(sFirstPersonSkeleton, priority, task, 0, 0, 1, 0) (the out reference dropped), and the base
// model's IdleAnimRoot_FindKFFiles list through 0043BDA0. 00435830's second argument is unused, so both lists are
// `root_kf_files`' (its FileFinder arm a stop). Completion: 100%.
fn queue_player_animation_models(
    world: &mut World,
    id: ReferenceTaskId,
    base: Entity,
    services: &mut ReferenceLoadServices<'_, '_>,
) -> Result<(), StreamError> {
    let race = base_race(world, base)?;
    let beast = race.is_some_and(|race| race_tail_model(world, race, 0).is_some());
    let settings = world.resource::<super::game_settings::GameSettingCollection>();
    let model = settings
        .text_at(if beast { 0x00B3_8B68 } else { 0x00B3_8B70 })
        .ok_or(StreamError("00662F8B: sNPCModel has no text"))?;
    let first_person = settings
        .text_at(0x00B3_6BB8)
        .ok_or(StreamError("00662FB6: sFirstPersonSkeleton has no text"))?;
    world
        .get_mut::<super::form_components::TesModel>(base)
        .ok_or(StreamError("00662F95: a player base with no TESModel"))?
        .path_04
        .set(Some(&model), 0);
    let priority = world
        .resource::<TaskGlobals>()
        .model_loader_b33a1c
        .as_ref()
        .and_then(|loader| loader.references.get(id))
        .ok_or(StreamError("00662F60: a released task"))?
        .priority();
    let queue_kf_list = |world: &mut World,
                         files: Option<Vec<String>>,
                         services: &mut ReferenceLoadServices<'_, '_>|
     -> Result<(), StreamError> {
        for kf in files.unwrap_or_default() {
            let mut result = Ok(());
            match services.archives.as_deref_mut() {
                Some(archives) => archives.with_archives(&mut |manager| {
                    result = crate::bs::kf_model::queue_kf(
                        world,
                        manager,
                        kf.as_bytes(),
                        priority,
                        Some(id),
                    );
                }),
                None => {
                    result = crate::bs::kf_model::queue_kf(
                        world,
                        None,
                        kf.as_bytes(),
                        priority,
                        Some(id),
                    );
                }
            }
            result?;
        }
        Ok(())
    };
    let files = super::forms_ai::root_kf_files(world, &first_person)?;
    queue_kf_list(world, files, services)?;
    let request = ModelRequest {
        priority,
        parent: Some(id),
        lod: 0,
        a7: false,
        a8: true,
        a9: false,
    };
    let path = fixup_model_path(&first_person);
    let child = {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        match services.archives.as_deref_mut() {
            Some(archives) => {
                let mut out = None;
                archives.with_archives(&mut |manager| {
                    out = Some(load_model(&mut tasks, manager, &path, request));
                });
                out.expect("with_archives runs its body")?
            }
            None => load_model(&mut tasks, None, &path, request)?,
        }
    };
    if let Some(child) = child {
        let finished = world
            .resource::<TaskGlobals>()
            .io_manager_b33a10
            .as_ref()
            .and_then(|io| io.tasks.store[child.0].as_ref())
            .is_some_and(|task| task.state_0c == 5);
        if finished {
            queued_file_notify_parent(world, child)?;
        }
    }
    let base_model = super::reference_load::model_path_of(world, base).unwrap_or_default();
    let files = super::forms_ai::root_kf_files(world, &base_model)?;
    queue_kf_list(world, files, services)
}

// IDA: 005E4DD0 Actor_QueueMagicResources: `base_and_race_abilities` (the base's, a Character's null birth sign or the player's +0x644, an NPC's race's), each through 0041A610(item, 0); then each of 00B1489C's ten slots (0..8, 13): 00486790's worn item's enchantment (TESEnchantableForm +0x04) through 0041A610 too. A QueuedMagicItem it starts (00438300, no parent) is a parent task of its art and associated models (`magic_selection::queued_magic_item_start`).
pub(crate) fn queue_magic_resources(
    world: &mut World,
    reference: Entity,
    services: &mut ReferenceLoadServices<'_, '_>,
) -> Result<(), StreamError> {
    // 005E4E35: the player's vt+0x268 (0066B0C0 PlayerCharacter_GetBirthSign, +0x644); a new game's player has none yet.
    if world
        .get::<super::player::PlayerCharacter>(reference)
        .is_some_and(|player| player.birthsign_644 != 0)
    {
        return Err(StreamError("005E4E45: the player's birth sign spells"));
    }
    let mut items = super::cell_reference_loop::base_and_race_abilities(world, reference);
    for slot in [0, 1, 2, 3, 4, 5, 6, 7, 8, 13] {
        let Some((form, _)) = equipped_in_slot(world, reference, slot)? else {
            continue;
        };
        if let Some(enchantable) = world.get::<super::form_components::TesEnchantableForm>(form) {
            match enchantable.enchantment_04 {
                super::form_components::FormRef::Resolved(Some(enchantment)) => {
                    items.push(enchantment)
                }
                super::form_components::FormRef::Resolved(None) => {}
                super::form_components::FormRef::Raw(0) => {}
                super::form_components::FormRef::Raw(_) => {
                    return Err(StreamError("005E4F7B: an unresolved enchantment"));
                }
            }
        }
    }
    for item in items {
        if let Some(request) = super::magic_selection::magic_item_queue_resources(world, item)? {
            super::magic_selection::queued_magic_item_start(
                world,
                request,
                services.archives.as_deref_mut(),
            )?;
        }
    }
    Ok(())
}

// IDA: 0043D000 ModelLoader_QueueAnimationFiles(base + 0xAC, priority, task, reference, 0, 0), the base's TESModel (the skeleton). A dead reference (vt+0x198 Actor_IsDead(0)) with a Skeleton model queues its folder's Idle.KF and Death.KF, a stop here. Otherwise the model map's vt+4 lookup of the path (0043D086) feeds only the IdleAnimRoot_FindKFFiles arm, which a6 = 0 skips, so the port doesn't make it (its lock's thread registration aside); a TESAnimation with names (00468830) queues each under the model's SpecialAnims folder, a stop here. Completion: 100% of a living NPC with no special animations.
fn queue_animation_files(
    world: &mut World,
    base: Entity,
    reference: Entity,
) -> Result<(), StreamError> {
    let path = super::reference_load::model_path_of(world, base).unwrap_or_default();
    if actor_is_dead(world, reference, false) {
        let skeleton = path
            .rsplit(|&byte| byte == b'\\')
            .next()
            .is_some_and(|name| name.len() >= 8 && name[..8].eq_ignore_ascii_case(b"Skeleton"));
        if skeleton {
            return Err(StreamError("0043D0C9: a dead actor's Idle.KF and Death.KF"));
        }
        return Ok(());
    }
    if world
        .get::<TesAnimation>(base)
        .is_some_and(|animation| !animation.names_04.is_empty())
    {
        return Err(StreamError("0043D2AE: a TESAnimation's SpecialAnims"));
    }
    Ok(())
}

/// IDA: N/A host deferral D12 (narrowed 2026-11-08): the QueuedHelmet an actor task doesn't make yet (0043BAF0: slot 1's model as its child, the head fit in 0043EB80). Its child QueuedModel's parent is an IO task, which `QueuedModel::parent_18` (a `ReferenceTaskId`) can't hold yet; with it the actor task finishes only after the helmet; the port's actor tasks finish in queue order anyway (D9). QueuedHead is made (`face_gen_tasks`).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct FaceGenDeferrals {
    /// The actor task, its skin info and slot 1's model, per helmet.
    pub helmets: Vec<(ReferenceTaskId, Entity, SkinModel)>,
    /// The NPCs whose 0043B090 finish reached 004353D0 (the head's attach under "Bip01 Head", 00528A10) and 00437B60 (the
    /// helmet's), in finish order.
    pub head_attaches: Vec<Entity>,
}

// IDA: 004788E0 SkinInfo_QueueSlotModels(out, priority, task): `out` null; an owner (+0x150) whose base isn't an NPC returns. Each of the sixteen slots with a model (+0x04, not null or -1): slot 1 with bLoadHelmetsInBackground, bUseFaceGenHeads and 00477ED0 (bUseMultiThreadedFaceGen and bUseFaceGenHeads) takes 004781D0's QueuedHelmet into `out`, deferred here (D12, `FaceGenDeferrals`); any other through ModelLoader_LoadTESModel(model, priority, task, 3, 0, 1, 0), its out reference dropped at once (004789E5). The port's task counts leave out the IO queue's and the parent's children list's references, which keep the child alive there, so nothing is released here.
fn queue_slot_models(
    world: &mut World,
    id: ReferenceTaskId,
    skin: Entity,
    services: &mut ReferenceLoadServices<'_, '_>,
) -> Result<(), StreamError> {
    let info = world.get::<SkinInfo>(skin).expect("a skin info").clone();
    let owner_base = world
        .get::<TesObjectRefr>(info.owner_150)
        .expect("a reference")
        .base_form_1c;
    if owner_base.is_none_or(|base| super::forms_init::form_type(world, base) != 0x23) {
        return Ok(());
    }
    let priority = world
        .resource::<TaskGlobals>()
        .model_loader_b33a1c
        .as_ref()
        .and_then(|loader| loader.references.get(id))
        .ok_or(StreamError("004788E0: a released task"))?
        .priority();
    let settings = *world.get_resource_or_init::<FaceGenSettings>();
    for (index, slot) in info.slots_4c.iter().enumerate() {
        let Some(model) = slot.model_04 else {
            continue;
        };
        if index == 1
            && settings.load_helmets_in_background_b06608
            && settings.use_face_gen_heads_b120b4
            && settings.use_multi_threaded_face_gen_b120dc
        {
            // D12: the QueuedHelmet (0043BAF0) is FaceGen's: its run takes the head node and its check fits the helmet to it.
            world
                .get_resource_or_init::<FaceGenDeferrals>()
                .helmets
                .push((id, skin, model));
            continue;
        }
        let path = skin_model_path(world, Some(model));
        if path.is_empty() {
            return Err(StreamError("004789E0: a slot model with an empty path"));
        }
        let request = ModelRequest {
            priority,
            parent: Some(id),
            lod: 3,
            a7: false,
            a8: true,
            a9: false,
        };
        let path = fixup_model_path(&path);
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let child = match services.archives.as_deref_mut() {
            Some(archives) => {
                let mut out = None;
                archives.with_archives(&mut |manager| {
                    out = Some(load_model(&mut tasks, manager, &path, request));
                });
                out.expect("with_archives runs its body")?
            }
            None => load_model(&mut tasks, None, &path, request)?,
        };
        if let Some(child) = child {
            // A cached model's child is finished already: 004374A0 notifies at once.
            let finished = world
                .resource::<TaskGlobals>()
                .io_manager_b33a10
                .as_ref()
                .and_then(|io| io.tasks.store[child.0].as_ref())
                .is_some_and(|task| task.state_0c == 5);
            if finished {
                queued_file_notify_parent(world, child)?;
            }
        }
    }
    Ok(())
}

/// IDA: 004349A0 Character vt+0x168, +0x104. Completion: 100% of a Character (the player's 006600B0 is QueuedPlayer's).
#[must_use]
pub fn skin_info(world: &World, reference: Entity) -> Option<Entity> {
    world
        .get::<Character>(reference)
        .and_then(|character| character.skin_info_104)
}

// IDA: 00523170(base, reference): vt+0x168 is null here (the caller tested it), so FormHeap(0x154) 00478730(reference, null) and vt+0x16C 00611B00, which frees no old one. 00478730: the 0x154 bytes and 00B33C80's 0x100 zeroed, +0x150 the reference; a first-person node (null here) would take 00478070.
fn create_skin_info(world: &mut World, reference: Entity) -> Entity {
    world.get_resource_or_init::<SkinSlotScratch>().slots_b33c80 = [SkinSlot::default(); 16];
    let skin = world
        .spawn(SkinInfo {
            bip01_00: None,
            bones_04: [SkinBone::default(); 9],
            slots_4c: [SkinSlot::default(); 16],
            owner_150: reference,
        })
        .id();
    world
        .get_mut::<Character>(reference)
        .expect("a Character's task")
        .skin_info_104 = Some(skin);
    skin
}

// IDA: 005268D0 TESNPC_InitBiped(base, reference, skin): 0047AB90(race +0xE8, sex +0x28 & 1); an actor's process vt+0x31C(1) (HighProcess 006295E0: +0x16C; LowProcess nothing); with no node, the current package's flags 0x100000 and 0x200000, a save's change flag 0x8000000 and the dead test pick 005227A0(base, reference, ..., 0, 0). Then 00525A70 unless bUseMultiThreadedFaceGen and bUseFaceGenHeads (both 1), 00524280, 00524510 for a skin info that isn't the player's first-person one, and vt+0x31C(0).
fn npc_init_biped(
    world: &mut World,
    base: Entity,
    reference: Entity,
    skin: Entity,
) -> Result<(), StreamError> {
    let race = base_race(world, base)?;
    let sex = npc_sex(world, base).unwrap_or(0);
    refill_body_slots(world, skin, race, sex);
    set_process_equip_byte(world, reference, 1);
    let node = world
        .get::<super::reference_3d::Reference3D>(reference)
        .map(|node| node.0);
    if node.is_none() {
        let flags = actor_current_package(world, reference)?
            .map_or(0, |package| package_data(world, package).flags_1c);
        let allocation = flags & 0x10_0000 == 0;
        let weapon = flags & 0x20_0000 == 0;
        let save_flags = super::forms_references::save_load_flags(world)?;
        let allowed =
            save_flags & 2 == 0 || change_flags(world, reference, true) & 0x0800_0000 == 0;
        let dead_kept = actor_is_dead(world, reference, false)
            && change_flags(world, reference, true) & 0x40 != 0;
        if allowed && !dead_kept {
            auto_equip(world, reference, allocation, weapon, false)?;
        }
    }
    // 005269C5: 00525A70 only without bUseMultiThreadedFaceGen or bUseFaceGenHeads, both 1 in the INI the port reads.
    let settings = *world.get_resource_or_init::<FaceGenSettings>();
    if !settings.use_multi_threaded_face_gen_b120dc || !settings.use_face_gen_heads_b120b4 {
        return Err(StreamError("005269D7: 00525A70 TESNPC_CreateHead"));
    }
    init_worn(world, reference, skin, sex)?;
    // 006600D0(the player, 1) is the player's +0x5C8, never this skin info, so 00524510(reference, null).
    update_face_gen_node_flags(world, node)?;
    set_process_equip_byte(world, reference, 0);
    Ok(())
}

// IDA: 00524510 TESNPC_UpdateFaceGenNodeFlags(reference, node): the given node, else the reference's; none does nothing. Its "BSFaceGenNiNodeBiped" and "BSFaceGenNiNodeSkinned" (006F94A0); with both, the hair and head slots' worn items set or clear bit 0 of +0x18 on FaceGenHair, the ears (00B10CAC, else 00B10CB0) and the two FaceGen nodes, a stop here. Completion: 100% of a node without both FaceGen nodes (the skeleton before 0047BC40 attaches the head).
fn update_face_gen_node_flags(world: &World, node: Option<Entity>) -> Result<(), StreamError> {
    let Some(node) = node else {
        return Ok(());
    };
    let biped = crate::bs::model_loader::object_by_name(world, node, b"BSFaceGenNiNodeBiped");
    let skinned = crate::bs::model_loader::object_by_name(world, node, b"BSFaceGenNiNodeSkinned");
    if biped.is_some() && skinned.is_some() {
        return Err(StreamError(
            "0052455F: the FaceGen nodes' hair and head flags",
        ));
    }
    Ok(())
}

// IDA: 005E0380 Actor_GetCurrentPackage: the process's vt+0x184: a HighProcess's 0064B040 (+0xC0, else +0x08), a LowProcess's 00452A60 (+0x08).
fn actor_current_package(world: &World, actor: Entity) -> Result<Option<Entity>, StreamError> {
    let Some(process) = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58
    else {
        return Ok(None);
    };
    if let Some(middle) = world.get::<MiddleHighProcess>(process)
        && middle.current_package_c0 != 0
    {
        return Err(StreamError("0064B043: a running package at +0xC0"));
    }
    Ok(world
        .get::<BaseProcess>(process)
        .expect("a process")
        .editor_package_08)
}

// IDA: the process's vt+0x31C(value): 006295E0 MiddleHighProcess_SetEquipmentUpdatePending on a MiddleHigh or High process stores +0x16C; a LowProcess's is a no-op.
fn set_process_equip_byte(world: &mut World, actor: Entity, value: u8) {
    let Some(process) = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58
    else {
        return;
    };
    if let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process) {
        middle.equipment_update_pending_16c = value;
    }
}

// IDA: 0047AB90(skin, race, sex): with a race, the sixteen slots move to 00B33C80 and are zeroed; then each of 00B06574's slots whose 00B065C8 bone is -1 or found takes 0052BE80(race, sex, slot) through 0047AA00's race arm: 00478780(slot, 1, null) (no node: form and model cleared), then the race and the model, which may be null.
fn refill_body_slots(world: &mut World, skin: Entity, race: Option<Entity>, sex: usize) {
    let Some(race) = race else {
        return;
    };
    let mut info = world.get::<SkinInfo>(skin).expect("a skin info").clone();
    world.get_resource_or_init::<SkinSlotScratch>().slots_b33c80 = info.slots_4c;
    info.slots_4c = [SkinSlot::default(); 16];
    for slot in BODY_SLOTS_B06574 {
        let bone = SLOT_BONE_B065C8[slot];
        if bone != -1 && info.bones_04[bone as usize].flags_00 & 1 == 0 {
            continue;
        }
        let model = race_body_model(world, race, sex, slot);
        info.slots_4c[slot] = SkinSlot {
            form_00: Some(race),
            model_04: model,
            node_08: None,
            skinned_0c: 0,
        };
    }
    *world.get_mut::<SkinInfo>(skin).expect("a skin info") = info;
}

/// IDA: 0052BE80 TESRace_GetBodyPartModel(sex, slot): the tail (15) by 0052BDB0; slots 2..5 index 00B36380's static models, 0x18 apart, the female five after the male (each model's path length by 00449190); an empty one falls back to the other sex's (a female's to the male's at 00B36380, a male's to the female's at 00B363F8); anything else null. Completion: 100%.
#[must_use]
pub fn race_body_model(world: &World, race: Entity, sex: usize, slot: usize) -> Option<SkinModel> {
    if slot == 15 {
        return race_tail_model(world, race, sex);
    }
    let part = slot.wrapping_sub(2);
    if sex > 1 || part > 4 {
        return None;
    }
    let statics = world.resource::<RaceStatics>();
    let has_path = |address: u32| {
        static_model_index(address).is_some_and(|index| {
            statics
                .model_paths
                .get(index)
                .is_some_and(|path| path_length(path) != 0)
        })
    };
    let own = 0x00B3_6380 + 0x18 * (5 * sex + part) as u32;
    if has_path(own) {
        return static_model_index(own).map(SkinModel::RaceStatic);
    }
    let other = if sex == 0 { 0x00B3_63F8 } else { 0x00B3_6380 } + 0x18 * part as u32;
    has_path(other)
        .then(|| static_model_index(other).map(SkinModel::RaceStatic))
        .flatten()
}

// IDA: N/A the `RaceStatics` index of one of 0052B6A0's thirteen addresses; 00B363E0 and 00B36458 aren't filled and read as empty.
fn static_model_index(address: u32) -> Option<usize> {
    const ADDRESSES: [u32; 13] = [
        0x00B3_6380,
        0x00B3_6398,
        0x00B3_63B0,
        0x00B3_63C8,
        0x00B3_63F8,
        0x00B3_6410,
        0x00B3_6428,
        0x00B3_6440,
        0x00B3_6470,
        0x00B3_6488,
        0x00B3_64A0,
        0x00B3_64B8,
        0x00B3_64D0,
    ];
    ADDRESSES.iter().position(|&known| known == address)
}

// IDA: 0052BDB0(race, sex): +0xB0 + 0x18 * sex with a path, else the male +0xB0, else the female +0xC8, else null.
fn race_tail_model(world: &World, race: Entity, sex: usize) -> Option<SkinModel> {
    if sex >= 2 {
        return None;
    }
    let race_data = world.get::<TesRace>(race).expect("a race");
    let has_path = |index: usize| path_length(&race_data.body_models_b0[index].path_04) != 0;
    [sex, 0, 1]
        .into_iter()
        .find(|&index| has_path(index))
        .map(|index| SkinModel::RaceTail { race, sex: index })
}

/// IDA: 00449190 TESModel's path length: the BSStringT's +0x04 word, or its `strlen` when that reads 0xFFFF. Completion: 100%.
#[must_use]
pub fn path_length(path: &BsStringT) -> usize {
    if path.length_04 != 0xFFFF {
        return usize::from(path.length_04);
    }
    path.data_00.as_deref().map_or(0, |bytes| {
        bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(bytes.len())
    })
}

/// IDA: the INI bytes the actor tasks read live: 00B120DC bUseMultiThreadedFaceGen:BackgroundLoad, 00B120B4 bUseFaceGenHeads:General, 00B06608 bLoadHelmetsInBackground:BackgroundLoad. The defaults are the exe's image values (all 1), for a world with no INI; `begin_load_selected_files` publishes the live ones.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceGenSettings {
    pub use_multi_threaded_face_gen_b120dc: bool,
    pub use_face_gen_heads_b120b4: bool,
    pub load_helmets_in_background_b06608: bool,
}

impl Default for FaceGenSettings {
    /// IDA: N/A the image values.
    fn default() -> Self {
        Self {
            use_multi_threaded_face_gen_b120dc: true,
            use_face_gen_heads_b120b4: true,
            load_helmets_in_background_b06608: true,
        }
    }
}

/// IDA: 00468FF0 TESBipedModelForm_HasSlot(slot, primary): the +0x04 word has bit `slot`; with `primary` the hair (1) of a form that also covers the head (bit 0 of +0x04's low byte) doesn't count. Completion: 100%.
#[must_use]
pub fn has_slot(mask: u16, slot: usize, primary: bool) -> bool {
    !(primary && slot == 1 && mask & 1 != 0) && u32::from(mask) & (1 << slot) != 0
}

// IDA: 00524280 TESNPC_InitWorn(base, reference, skin), the base's sex (+0x28 & 1) passed as `sex`: per slot 0..15, 00486790's worn form: the head and hair through 004691D0 with -1; the body, ring, amulet and tail slots through 004691D0 with 6, or 7 for a stack worn on the left; the weapon by 00478CA0 (slot 9: the form and its +0x30 TESModel), the shield (with a process below level 2 its entry, vt+0xF8(0), absent or worn) like a body part; the quiver (00478D70) and torch (00478DF0) are stops. The selection is freed.
fn init_worn(
    world: &mut World,
    reference: Entity,
    skin: Entity,
    sex: usize,
) -> Result<(), StreamError> {
    for slot in 0..16_u32 {
        let Some((form, left)) = equipped_in_slot(world, reference, slot)? else {
            continue;
        };
        let index = match slot {
            0 | 1 => -1,
            2..=8 | 15 => {
                if left {
                    7
                } else {
                    6
                }
            }
            9 => {
                // 00478CA0 SkinInfo_SetWeapon(form): a WEAP releases slot 9 (00478780(1, null)) and takes the form and its TESModel (+0x30).
                if super::forms_init::form_type(world, form) == 0x21 {
                    let mut info = world.get_mut::<SkinInfo>(skin).expect("a skin info");
                    release_slot(&mut info.slots_4c[9])?;
                    info.slots_4c[9].form_00 = Some(form);
                    info.slots_4c[9].model_04 = Some(SkinModel::Form(form));
                }
                continue;
            }
            12 => return Err(StreamError("00524305: 00478D70, a worn quiver")),
            13 => {
                // 00524312: with a process below level 2 its shield entry (vt+0xF8(0), +0xF0) must be absent or worn; then the
                // slot is a body part's (LABEL_26). A higher process's (no entry) is the same.
                let process = super::ai_conditions::process_of(world, reference);
                let held = process
                    .filter(|&process| super::actor_process::process_level(world, process) < 2)
                    .and_then(|process| {
                        world.get::<super::actor_process::MiddleHighProcess>(process)
                    })
                    .and_then(|middle| middle.shield_f0);
                if held.is_some_and(|entry| !entry.worn) {
                    continue;
                }
                if left { 7 } else { 6 }
            }
            14 => return Err(StreamError("005243C8: 00478DF0, a worn torch")),
            _ => return Err(StreamError("00524428: a slot InitWorn doesn't support")),
        };
        if world.get::<TesBipedModelForm>(form).is_none() {
            return Err(StreamError("00524403: a bad part in TESNPC::InitWorn"));
        }
        equip_biped_form(world, form, skin, sex, index)?;
    }
    Ok(())
}

// IDA: 004691D0(biped, skin, sex, index): sex 1 with an empty female biped path (+0x24) takes 0; 0047AA00(skin, form, +0x08 + 0x18 * sex, index).
fn equip_biped_form(
    world: &mut World,
    form: Entity,
    skin: Entity,
    sex: usize,
    index: i32,
) -> Result<(), StreamError> {
    let biped = world.get::<TesBipedModelForm>(form).expect("tested");
    let sex = if sex == 1 && path_length(&biped.biped_models_08[1].path_04) == 0 {
        0
    } else {
        sex.min(1)
    };
    set_slot_form(world, skin, form, SkinModel::Biped { form, sex }, index)
}

// IDA: 0047AA00(skin, form, model, index)'s biped arm: a ring (slot 7 or 6) with an index takes 00478780 on that slot and the form and model there; otherwise every slot the form covers holding another form is freed (a race's by 00478780, another form's everywhere by 00479740), then the first slot it covers as primary takes it.
fn set_slot_form(
    world: &mut World,
    skin: Entity,
    form: Entity,
    model: SkinModel,
    index: i32,
) -> Result<(), StreamError> {
    let mask = biped_mask(world, form).unwrap_or(0);
    let mut info = world.get::<SkinInfo>(skin).expect("a skin info").clone();
    if index != -1 && (has_slot(mask, 7, false) || has_slot(mask, 6, false)) {
        release_slot(&mut info.slots_4c[index as usize])?;
        info.slots_4c[index as usize].form_00 = Some(form);
        info.slots_4c[index as usize].model_04 = Some(model);
    } else {
        for slot in 0..16 {
            if !has_slot(mask, slot, false) {
                continue;
            }
            let Some(held) = info.slots_4c[slot].form_00 else {
                continue;
            };
            if held == form {
                continue;
            }
            if world.get::<TesRace>(held).is_some() {
                release_slot(&mut info.slots_4c[slot])?;
            } else {
                // 00479740: every slot holding `held`.
                for other in &mut info.slots_4c {
                    if other.form_00 == Some(held) {
                        release_slot(other)?;
                    }
                }
            }
        }
        if let Some(slot) = (0..16).find(|&slot| has_slot(mask, slot, true)) {
            info.slots_4c[slot].model_04 = Some(model);
            info.slots_4c[slot].form_00 = Some(form);
        }
    }
    *world.get_mut::<SkinInfo>(skin).expect("a skin info") = info;
    Ok(())
}

// IDA: 00478780(slot, 1, null): a loaded node (+0x08) takes the detach, the shader resource and the model map's release, a stop here; then the form and model cleared.
fn release_slot(slot: &mut SkinSlot) -> Result<(), StreamError> {
    if slot.node_08.is_some() {
        return Err(StreamError("00478791: releasing a slot's loaded node"));
    }
    slot.form_00 = None;
    slot.model_04 = None;
    Ok(())
}

/// IDA: N/A a slot model's path text (its TESModel +0x04), up to the terminator; empty for none.
#[must_use]
pub fn skin_model_path(world: &World, model: Option<SkinModel>) -> Vec<u8> {
    let text = |path: &BsStringT| {
        path.data_00
            .as_deref()
            .map(|bytes| {
                bytes
                    .split(|&byte| byte == 0)
                    .next()
                    .unwrap_or_default()
                    .to_vec()
            })
            .unwrap_or_default()
    };
    match model {
        None => Vec::new(),
        Some(SkinModel::RaceStatic(index)) => world
            .resource::<RaceStatics>()
            .model_paths
            .get(index)
            .map(text)
            .unwrap_or_default(),
        Some(SkinModel::RaceTail { race, sex }) => {
            text(&world.get::<TesRace>(race).expect("a race").body_models_b0[sex].path_04)
        }
        Some(SkinModel::Form(form)) => text(
            &world
                .get::<super::form_components::TesModel>(form)
                .expect("a form with a model")
                .path_04,
        ),
        Some(SkinModel::Biped { form, sex }) => text(
            &world
                .get::<TesBipedModelForm>(form)
                .expect("a biped form")
                .biped_models_08[sex]
                .path_04,
        ),
    }
}

/// IDA: 00B06550, the nine bone names 00478070 looks up, in order.
const SKIN_BONES_B06550: [&[u8]; 9] = [
    b"Bip01 Head",
    b"Bip01 R Finger1",
    b"Bip01 L Finger1",
    b"Weapon",
    b"BackWeapon",
    b"SideWeapon",
    b"Quiver",
    b"Bip01 L ForearmTwist",
    b"Torch",
];

/// IDA: 00478070 SkinInfo_FindBones(skin, root): +0x00 the root's "Bip01" (0x6F94A0); each of 00B06550's names found as an object
/// whose vt+8 says a node sets its entry's bit 0 and node, otherwise warns "Missing bone"; then 00897A90 on the BackWeapon and
/// SideWeapon nodes (+0x28, +0x30), below. Completion: 100%.
/// # Errors
/// None yet; the signature keeps the stops' shape.
pub fn skin_info_find_bones(
    world: &mut World,
    skin: Entity,
    root: Entity,
) -> Result<(), StreamError> {
    use crate::bs::model_loader::object_by_name;
    let bip01 = object_by_name(world, root, b"Bip01");
    let mut bones = [SkinBone::default(); 9];
    for (bone, name) in bones.iter_mut().zip(SKIN_BONES_B06550) {
        if let Some(found) = object_by_name(world, root, name)
            && world
                .get::<crate::gamebryo::stream_objects::NiClass>(found)
                .is_some_and(|class| class.is_node())
        {
            bone.flags_00 |= 1;
            bone.node_04 = Some(found);
        }
    }
    // 00897A90(node, 0) on the BackWeapon and SideWeapon bones: a bhkNiCollisionObject with a body takes 0089F520(0) on the
    // body: the hk object's collision filter (+0x08, +0x14 + 0x1C) gains bit 14 (0x4000), then vt+0x80 refreshes it in the
    // world. Host substitution (D2): the loaded cinfo's word 0, which builds that filter, takes the bit; no hk world to refresh.
    for bone in [bones[4], bones[5]] {
        let body = bone
            .node_04
            .and_then(|node| world.get::<crate::gamebryo::av_object::NiAvCollision>(node))
            .and_then(|collision| {
                world.get::<crate::bs::havok_stream::BhkSpCollisionObject>(collision.0)
            })
            .and_then(|collision| collision.body_10);
        if let Some(body) = body
            && let Some(mut rigid) = world.get_mut::<crate::bs::havok_stream::BhkRigidBody>(body)
        {
            let filter =
                u32::from_le_bytes(rigid.cinfo_0c[..4].try_into().expect("a cinfo")) | 0x4000;
            rigid.cinfo_0c[..4].copy_from_slice(&filter.to_le_bytes());
        }
    }
    let mut info = world.get_mut::<SkinInfo>(skin).expect("a skin info");
    info.bip01_00 = bip01;
    info.bones_04 = bones;
    Ok(())
}

/// IDA: 004796F0 SkinInfo_ReleaseAllSlots under 00B33E00's lock: 00478780(slot, 1, null) on each of the sixteen. Completion: 100%, with `release_slot`'s stop on a loaded node.
/// # Errors
/// A slot with a loaded node.
pub fn release_all_slots(world: &mut World, skin: Entity) -> Result<(), StreamError> {
    let mut info = world.get_mut::<SkinInfo>(skin).expect("a skin info");
    for slot in &mut info.slots_4c {
        release_slot(slot)?;
    }
    Ok(())
}

/// IDA: 00526DB0 TESNPC_UpdateBiped(base, reference): one pass for an NPC (its skin info, vt+0x168, and node), two for the player (see below); `005268D0` TESNPC_InitBiped, 0047BC40 (`biped_attach::attach_biped_models`), and with a node NiAVObject_RefreshPropertyState, RefreshEffectState, vt+0x78 (0070A360 UpdateWorldBound) and NiAVObject_Update(0, 0). Completion: 100%, with 0047BC40's.
/// # Errors
/// The player, or a stop in 005268D0 or the update.
pub fn npc_update_biped(
    world: &mut World,
    base: Entity,
    reference: Entity,
    services: &mut crate::bs::model_loader::ModelBuildServices<'_>,
) -> Result<(), StreamError> {
    // 00526DD9: the player takes two passes, its view skin info (vt+0x168 006600B0: +0x104 while +0x588 is set, else +0x5C8
    // when there is one) on +0x3C, then 006600D0(+0x588)'s skin on 00664F40(1)'s +0x5D0.
    let mut passes = vec![(
        skin_info(world, reference),
        world
            .get::<super::reference_3d::Reference3D>(reference)
            .map(|node| node.0),
    )];
    if Some(reference) == super::ai_conditions::the_player(world) {
        let player = world
            .get::<super::player::PlayerCharacter>(reference)
            .expect("the player");
        let third_person = player.view_flags_588[0] != 0;
        if !third_person && player.first_person_skin_info_5c8.is_some() {
            passes[0].0 = player.first_person_skin_info_5c8;
        }
        let second = if third_person {
            player.first_person_skin_info_5c8
        } else {
            skin_info(world, reference)
        };
        passes.push((second, player.first_person_node_5d0));
    }
    for (skin, node) in passes {
        update_biped_pass(world, base, reference, skin, node, services)?;
    }
    Ok(())
}

// IDA: one pass of 00526DB0: 005268D0 TESNPC_InitBiped, 0047BC40, and with a node its property and effect state, vt+0x78
// and NiAVObject_Update(0, 0).
fn update_biped_pass(
    world: &mut World,
    base: Entity,
    reference: Entity,
    skin: Option<Entity>,
    node: Option<Entity>,
    services: &mut crate::bs::model_loader::ModelBuildServices<'_>,
) -> Result<(), StreamError> {
    let skin = skin.ok_or(StreamError("00526DF0: TESNPC_InitBiped with no skin info"))?;
    npc_init_biped(world, base, reference, skin)?;
    super::biped_attach::attach_biped_models(world, skin, services)?;
    if let Some(node) = node {
        crate::gamebryo::scene_state::refresh_property_state(
            world,
            node,
            &services.prepare.property_defaults,
        )?;
        crate::gamebryo::scene_state::refresh_effect_state(world, node)?;
        crate::gamebryo::scene_update::update_node_world_bound(world, node)?;
        let result = crate::gamebryo::scene_update::update_scene_object(
            world,
            node,
            0.0,
            false,
            &mut crate::gamebryo::scene_update::NoSceneControllers,
        )?;
        if result.status != crate::gamebryo::scene_update::SceneUpdateStatus::Updated {
            return Err(StreamError("00526E59: the NPC's NiAVObject_Update stopped"));
        }
    }
    Ok(())
}

/// IDA: 0043DD80 QueuedCreature_QueueChildren (QueuedCreature's vt+0x2C): the base's TESModel (+0xAC, the skeleton) through
/// `0043D000` with a6 = 1 (`queue_creature_animation_files`), `0043CDE0` (`queue_creature_body_models`), `005E4DD0` and the
/// caller's QueuedReference_QueueModelChild. Completion: as the three.
/// # Errors
/// A stop in any of them.
pub fn queue_creature_children(
    world: &mut World,
    id: ReferenceTaskId,
    reference: Entity,
    base: Entity,
    services: &mut ReferenceLoadServices<'_, '_>,
) -> Result<(), StreamError> {
    queue_creature_animation_files(world, id, reference, base, services)?;
    queue_creature_body_models(world, id, reference, base, services)?;
    queue_magic_resources(world, reference, services)
}

// IDA: 0043D000(model, priority, task, reference, 1, 0) for a creature: a dead reference (vt+0x198(0)) whose model's last
// component starts "Skeleton" queues "Data\" + ("Meshes\" unless the path has it) + the path, its last component replaced by
// "\Idle.KF" and then "\Death.KF", each through 00431970(name, path, 1) and 0043BDA0 (0043B840 per file, the task the
// parent). A living one's arm (the map's vt+4, 00435830(path, 1)) and a TESAnimation's SpecialAnims are stops here.
fn queue_creature_animation_files(
    world: &mut World,
    id: ReferenceTaskId,
    reference: Entity,
    base: Entity,
    services: &mut ReferenceLoadServices<'_, '_>,
) -> Result<(), StreamError> {
    let path = super::reference_load::model_path_of(world, base).unwrap_or_default();
    if !actor_is_dead(world, reference, false) {
        return Err(StreamError(
            "0043D086: a living creature's KF files (00435830)",
        ));
    }
    let Some(slash) = path.iter().rposition(|&byte| byte == b'\\') else {
        return Ok(());
    };
    if recomp_core::crt::strnicmp(&path[slash + 1..], b"Skeleton", 8) != 0 {
        return Ok(());
    }
    let text: String = path.iter().map(|&byte| char::from(byte)).collect();
    let mut name = String::from("Data\\");
    if recomp_core::crt::strnicmp(&path, b"Meshes\\", 7) != 0 {
        name.push_str("Meshes\\");
    }
    name.push_str(&text);
    let cut = name.rfind('\\').expect("the prefix has one");
    let priority = world
        .resource::<TaskGlobals>()
        .model_loader_b33a1c
        .as_ref()
        .and_then(|loader| loader.references.get(id))
        .ok_or(StreamError("0043D000: a released task"))?
        .priority();
    for file in ["\\Idle.KF", "\\Death.KF"] {
        let pattern = format!("{}{file}", &name[..cut]);
        let mut files = Vec::new();
        if let Some(archives) = services.archives.as_deref_mut() {
            archives.with_archives(&mut |manager| {
                if let Some(manager) = manager {
                    files = manager.find_matching_files(&pattern, &text, 1);
                }
            });
        }
        for kf in files {
            let mut result = Ok(());
            match services.archives.as_deref_mut() {
                Some(archives) => archives.with_archives(&mut |manager| {
                    result = crate::bs::kf_model::queue_kf(
                        world,
                        manager,
                        kf.as_bytes(),
                        priority,
                        Some(id),
                    );
                }),
                None => {
                    result = crate::bs::kf_model::queue_kf(
                        world,
                        None,
                        kf.as_bytes(),
                        priority,
                        Some(id),
                    )
                }
            }
            result?;
        }
    }
    Ok(())
}

// IDA: 0043CDE0 QueuedCreature_QueueBodyModels(model, priority, task, reference): the TESCreature's NIF list (TESModelList
// +0xEC) under the model's folder (the path through its last '\') through 0043BC20: each name by 0043B5E0 (ModelLoader_LoadModel
// with a parent, lod 0, a8 1; its +0x7C hash goes to vt+0x30, which ignores it), a cached one notifying at once. Then with a
// reference the changes' best slot-13 armor (0048C870(.., 13, 0)), weapon (0048BDA0(.., -1, 0)) and ammo (0048B9C0(.., 0)),
// each through 0043B990 ModelLoader_QueueModelForReference.
fn queue_creature_body_models(
    world: &mut World,
    id: ReferenceTaskId,
    reference: Entity,
    base: Entity,
    services: &mut ReferenceLoadServices<'_, '_>,
) -> Result<(), StreamError> {
    if world
        .get::<super::forms_actors::TesCreature>(base)
        .is_none()
    {
        return Ok(());
    }
    let path = super::reference_load::model_path_of(world, base).unwrap_or_default();
    let folder = match path.iter().rposition(|&byte| byte == b'\\') {
        Some(slash) => path[..=slash].to_vec(),
        None => path.clone(),
    };
    let names = world
        .get::<super::form_components::TesModelList>(base)
        .map(|list| list.names_04.clone())
        .unwrap_or_default();
    let priority = world
        .resource::<TaskGlobals>()
        .model_loader_b33a1c
        .as_ref()
        .and_then(|loader| loader.references.get(id))
        .ok_or(StreamError("0043CDE0: a released task"))?
        .priority();
    for name in names {
        let name: Vec<u8> = name
            .split(|&byte| byte == 0)
            .next()
            .unwrap_or_default()
            .to_vec();
        let full = fixup_model_path(&[folder.as_slice(), &name].concat());
        let request = ModelRequest {
            priority,
            parent: Some(id),
            lod: 0,
            a7: false,
            a8: true,
            a9: false,
        };
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let child = match services.archives.as_deref_mut() {
            Some(archives) => {
                let mut out = None;
                archives.with_archives(&mut |manager| {
                    out = Some(load_model(&mut tasks, manager, &full, request));
                });
                out.expect("with_archives runs its body")?
            }
            None => load_model(&mut tasks, None, &full, request)?,
        };
        if let Some(child) = child {
            let finished = world
                .resource::<TaskGlobals>()
                .io_manager_b33a10
                .as_ref()
                .and_then(|io| io.tasks.store[child.0].as_ref())
                .is_some_and(|task| task.state_0c == 5);
            if finished {
                queued_file_notify_parent(world, child)?;
            }
        }
    }
    let changes = super::inventory::ensure_container_changes(world, reference)?;
    let picks = (|| -> Result<[Option<Entity>; 3], StreamError> {
        let armor = super::actor_enable::best_armor(world, &changes, base, reference, 13, false)?
            .map(|(form, _)| form);
        let weapon = super::actor_enable::best_weapon(world, &changes, base, reference, false)?
            .map(|(form, _)| form);
        let ammo = super::actor_enable::best_ammo(world, &changes, reference, false)?;
        Ok([armor, weapon, ammo])
    })();
    super::inventory::put_container_changes(world, reference, changes);
    for form in picks?.into_iter().flatten() {
        super::reference_load::queue_model_for_reference(world, id, form, reference, services)?;
    }
    Ok(())
}
