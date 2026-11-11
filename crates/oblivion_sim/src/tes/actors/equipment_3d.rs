//! An actor's equipment 3D rebuilt in place: a MiddleHigh process's vt+0x318 (`0063CDC0` MiddleHighProcess_UpdateEquipment3D)
//! while its +0x16C is set, through `004E1580` TESObjectREFR_UpdateBodyModels (the NPC arm: `00526DB0` TESNPC_UpdateBiped),
//! as `005C2F20` (the race menu's head rebuild) and a vampire's paths call it. The render-only calls are counted
//! (`Equipment3DBoundaries`).

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor::{Character, MobileObject, TesObjectRefr};
use super::actor_enable::find_skin_property;
use super::actor_process::MiddleHighProcess;
use crate::bs::model_loader::ModelBuildServices;
use crate::gamebryo::blend_accum::reset_blend_accum;
use crate::gamebryo::object_net::{object_by_name_with_palette, palette_object};
use crate::tes::forms_items::weapon_type;
use crate::tes::player::the_player;

/// IDA: N/A the calls the rebuild counts rather than makes: `007C5E70` and `007C5D00` ShadowSceneNode_UpdateObjectLights (the scene's
/// lights), `006575B0` (the process's vt+0x42C, the magic effect shaders), `00481410` (a non-player's reference ID into its
/// shader properties), and `0047D120`/`0047D0F0` (TimeInfo's disable count around `004E1580`: it keeps the call's wall time
/// out of the game clock, which the driver's clock holds; a host substitution).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct Equipment3DBoundaries {
    pub calls: BTreeMap<u32, u32>,
}

// IDA: N/A a helper, with no address of its own.
pub(crate) fn count(world: &mut World, address: u32) {
    *world
        .get_resource_or_init::<Equipment3DBoundaries>()
        .calls
        .entry(address)
        .or_default() += 1;
}

/// IDA: 004728C0 ActorAnimData_ResetAccumRoot(data) (fastcall): with an accum root (+0x08), +0x18 = 0, the root's rotation
/// the identity and its translate 0; then the root's controllers (+0x0C, next +0x34): a NiSingleInterpController (RTTI
/// 00B3CCB0) whose vt+0x80(0) interpolator is a NiBlendAccumTransformInterpolator (00B3CD1C) goes through `00471640` (its
/// transform reset). Completion: 100% of the NiBoneLODController and NiTransformController the player's accum root holds;
/// another controller class is a stop.
/// # Errors
/// Another controller class, or one with no time payload.
pub fn reset_accum_root(world: &mut World, anim: Entity) -> Result<(), GameError> {
    let Some(root) = world
        .get::<super::actor_animation::ActorAnimData>(anim)
        .and_then(|data| data.accum_root_08)
    else {
        return Ok(());
    };
    world
        .get_mut::<super::actor_animation::ActorAnimData>(anim)
        .expect("tested")
        .position_18 = [0.0; 3];
    if let Some(mut payload) = world.get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(root)
    {
        payload.local_transform.rotation = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        payload.local_transform.translation = [0.0; 3];
    }
    // 004728F0..: the root's controllers (+0x0C, then each one's +0x34); a NiSingleInterpController's vt+0x80(0)
    // interpolator, when a NiBlendAccumTransformInterpolator, goes to 00471640. NiBoneLODController isn't one.
    let mut current = world
        .get::<crate::gamebryo::object_net::NiObjectNet>(root)
        .and_then(|net| net.controller_0c);
    while let Some(controller) = current {
        match world.get::<crate::gamebryo::stream_objects::NiClass>(controller) {
            Some(crate::gamebryo::stream_objects::NiClass::BoneLodController) => {}
            Some(crate::gamebryo::stream_objects::NiClass::TransformController) => {
                // vt+0x80(0): its interpolator (+0x3C); a NiBlendAccumTransformInterpolator goes through 00471640.
                if let Some(interpolator) = world
                    .get::<crate::gamebryo::transform_controller::TransformController>(controller)
                    .and_then(|transform| transform.interpolator_3c)
                    && world
                        .get::<crate::gamebryo::controller_sequence::BlendAccumTransformState>(
                            interpolator,
                        )
                        .is_some()
                {
                    reset_blend_accum(world, interpolator)?;
                }
            }
            _ => {
                return Err(GameError::unclassified(
                    0x004728C0,
                    "004728C0: an accum root controller other than NiBoneLODController or NiTransformController",
                ));
            }
        }
        // The next link (+0x34).
        current = crate::gamebryo::particle_controller::controller_time(world, controller)
            .map(|time| time.next_34)
            .ok_or(GameError::unclassified(
                0x004728C0,
                "004728C0: a controller with no time payload",
            ))?;
    }
    Ok(())
}

/// IDA: an actor's vt+0x168: the Character's skin info (+0x104); the player's `006600B0` PlayerCharacter_GetViewSkinInfo,
/// the skin info in the third person, else the first person's (+0x5C8) when there is one. Completion: 100%.
#[must_use]
pub fn view_skin_info(world: &World, actor: Entity) -> Option<Entity> {
    if Some(actor) != the_player(world) {
        return super::character_3d::skin_info(world, actor);
    }
    let body = world
        .get::<super::player::PlayerCharacter>(actor)
        .expect("the player");
    if body.view_flags_588[0] != 0 {
        super::character_3d::skin_info(world, actor)
    } else {
        body.first_person_skin_info_5c8
            .or_else(|| super::character_3d::skin_info(world, actor))
    }
}

/// IDA: 004E1580 TESObjectREFR_UpdateBodyModels(actor), its NPC arm (base type 0x23): vt+0x168 (the view's skin info),
/// TimeInfo_IncrementDisableCount (counted), vt+0x164's animation data through `004728C0`, then with a skin info
/// `00526DB0` TESNPC_UpdateBiped(base, actor); TimeInfo_DecrementDisableCount (counted). Answers 1. The creature arm (type
/// 0x24) is `actor_enable`'s; anything else is a stop here. Completion: 100% of the NPC arm.
/// # Errors
/// Another base type, or a stop in `004728C0` or `00526DB0`.
pub fn update_body_models(
    world: &mut World,
    actor: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<bool, GameError> {
    let base = world
        .get::<TesObjectRefr>(actor)
        .and_then(|refr| refr.base_form_1c)
        .ok_or(GameError::unclassified(
            0x004E1580,
            "004E1580: a reference with no base faults",
        ))?;
    let player = Some(actor) == the_player(world);
    let skin = view_skin_info(world, actor);
    count(world, 0x0047_D120);
    // vt+0x164: the player's 0065D720; a Character's TESObjectREFR_GetAnimData (004D8370), a MiddleHigh process's +0x17C.
    let anim = if player {
        super::actor_movement_update::player_anim_data(world, actor)
    } else {
        world
            .get::<MobileObject>(actor)
            .and_then(|mobile| mobile.process_58)
            .and_then(|process| world.get::<MiddleHighProcess>(process))
            .and_then(|middle| middle.anim_data_17c)
    };
    if let Some(anim) = anim {
        reset_accum_root(world, anim)?;
    }
    if crate::tes::forms_init::form_type(world, base) != 0x23 {
        return Err(GameError::unclassified(
            0x004E1580,
            "004E1580: UpdateBodyModels for a base that isn't an NPC",
        ));
    }
    if skin.is_some() {
        super::character_3d::npc_update_biped(world, base, actor, services)?;
    }
    count(world, 0x0047_D0F0);
    Ok(true)
}

/// IDA: 0063CDC0 MiddleHighProcess_UpdateEquipment3D (process vt+0x318)(actor): with +0x16C set and the actor's node (vt+0x154):
/// `007C5E70` (counted); a vampire (AV 0x45) with 00B33D80 clear sets it around the rest (`EquipmentRebuildFlag`);
/// `004E1580`; `007C5D00` ShadowSceneNode_UpdateObjectLights (counted); the FaceGen animation data (`005E12B0`, process vt+0xE8)
/// vt+0x9C(dead) (`00549590`: a live actor whose +0x1DA is clear has nothing to do; otherwise a stop); `00528550`
/// TESNPC_FixFaceNormals (nothing with bFixFaceNormals 0, else a stop); the process's vt+0x42C (`006575B0`, counted); with a
/// refraction extra (0x51) the image-space arm (a stop), else `005EE1B0`; then the player with a hidden Inventory menu
/// (`inventory_hidden`, 0057A310) takes `00664E60` (a stop), anyone else `005EA1A0` Actor_UpdateSkinProperty on its node (the
/// player's first-person child 0 out of the third person) and, for a non-player, `00481410` (counted); +0x16C = 0.
/// Completion: 100% of the arms a live, unrefracted, non-vampire player reaches.
/// TODO: [ ] The FaceGen data's dead arm, FixFaceNormals, the refraction arm and `00664E60`.
/// # Errors
/// The stops named, or a stop in `004E1580`, `005EE1B0` or `005EA1A0`.
pub fn update_equipment_3d(
    world: &mut World,
    process: Entity,
    actor: Entity,
    inventory_hidden: bool,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let pending = world
        .get::<MiddleHighProcess>(process)
        .ok_or(GameError::unclassified(
            0x0063CDC0,
            "0063CDC0: a process with no MiddleHigh part",
        ))?
        .equipment_update_pending_16c;
    let player = Some(actor) == the_player(world);
    let node = if player {
        super::player_3d::player_ni_node(world, actor)
    } else {
        world
            .get::<crate::tes::reference_3d::Reference3D>(actor)
            .map(|node| node.0)
    };
    let Some(node) = node.filter(|_| pending != 0) else {
        return Ok(());
    };
    count(world, 0x007C_5E70);
    let flag = world
        .get_resource_or_init::<super::biped_attach::EquipmentRebuildFlag>()
        .active_b33d80;
    let vampire = crate::tes::actor::actor_get_actor_value(world, actor, 0x45)? != 0 && !flag;
    if vampire {
        world
            .resource_mut::<super::biped_attach::EquipmentRebuildFlag>()
            .active_b33d80 = true;
    }
    update_body_models(world, actor, services)?;
    if vampire {
        world
            .resource_mut::<super::biped_attach::EquipmentRebuildFlag>()
            .active_b33d80 = false;
    }
    count(world, 0x007C_5D00);
    if let Some(anim) = super::actor_movement_update::actor_face_animation_data(world, actor)
        && world
            .get::<crate::tes::face_gen_nodes::BsFaceGenAnimationData>(anim)
            .is_some_and(|data| data.animation_disabled_1da != 0)
    {
        return Err(GameError::unclassified(
            0x00549590,
            "00549590: a FaceGen pose with its dead flag set",
        ));
    }
    if world
        .get_resource::<crate::tes::face_gen_tasks::FaceGenHeadSettings>()
        .is_some_and(|settings| settings.fix_face_normals_b120bc)
    {
        return Err(GameError::unclassified(
            0x00528550,
            "00528550: TESNPC_FixFaceNormals with bFixFaceNormals",
        ));
    }
    count(world, 0x0065_75B0);
    super::actor_fade::actor_update_alpha(world, actor)?;
    if player && inventory_hidden {
        return Err(GameError::unclassified(
            0x0063CDC0,
            "0063CDC0: 00664E60 with a hidden Inventory menu",
        ));
    }
    let skin_node = if player
        && world
            .get::<super::player::PlayerCharacter>(actor)
            .is_some_and(|body| body.view_flags_588[0] == 0)
    {
        world
            .get::<crate::gamebryo::scene_node::NiNodeChildren>(node)
            .and_then(|children| children.slots_ac.first().copied().flatten())
            .ok_or(GameError::unclassified(
                0x0063CDC0,
                "0063CDC0: the first-person node with no child 0",
            ))?
    } else {
        node
    };
    super::actor_enable::find_skin_property(world, process, skin_node)?;
    if !player {
        count(world, 0x0048_1410);
    }
    world
        .get_mut::<MiddleHighProcess>(process)
        .expect("tested")
        .equipment_update_pending_16c = 0;
    Ok(())
}

// IDA: 006541A0 MiddleHighProcess_FindWeaponNodes(process, manager, node) for a non-player: the sheath index 4 (BackWeapon) for weapon types 1 and 3..5, else 5
// (SideWeapon). A manager looks each name up in its +0x7C object palette (vt+0x4C) alone; with none, 006F94A0 on the node. "Weapon" +0xFC, the sheath
// +0x108, "Torch" +0x100, "Bip01 L ForearmTwist" +0x104, "Quiver" +0x10C, +0x110 = 0. No node stores only +0x110.
pub(super) fn weapon_bone_nodes(
    world: &World,
    manager: Option<Entity>,
    node: Option<Entity>,
    weapon_kind: Option<u8>,
) -> Result<super::actor_process::WeaponNodes, GameError> {
    let mut nodes = super::actor_process::WeaponNodes::default();
    let sheath: &[u8] = match weapon_kind {
        Some(1 | 3..=5) => b"BackWeapon",
        _ => b"SideWeapon",
    };
    if manager.is_none() && node.is_none() {
        return Ok(nodes);
    }
    let find = |name: &[u8]| match (manager, node) {
        (Some(manager), _) => palette_object(world, manager, name),
        (None, Some(node)) => object_by_name_with_palette(world, node, name),
        (None, None) => None,
    };
    nodes.weapon_fc = find(b"Weapon");
    nodes.sheath_108 = find(sheath);
    nodes.torch_100 = find(b"Torch");
    nodes.forearm_twist_104 = find(b"Bip01 L ForearmTwist");
    nodes.quiver_10c = find(b"Quiver");
    Ok(nodes)
}

/// IDA: 004E1900 TESObjectREFR_AttachWeaponModel(reference, weapon) for a non-player with its 3D: the skin info (vt+0x168,
/// the Character's +0x104) takes 00479780 SkinInfo_AttachWeapon (`attach_weapon_part`); with none (a creature) 00479450 under the actor, host
/// deferral D15 (`CreatureBodyDeferrals`, which also names the part and sets the draw request). Then a bow's 005E13D0 and
/// arrow anim data are stops, 005EA1A0 Actor_UpdateSkinProperty on the node and 005EE1B0 Actor_UpdateAlpha. No 3D returns.
/// Completion: a non-player's melee weapon.
/// # Errors
/// The stops above, or one in the part's attach.
pub(super) fn attach_weapon_model(
    world: &mut World,
    actor: Entity,
    process: Entity,
    weapon: Entity,
) -> Result<(), GameError> {
    let Some(node) = world
        .get::<crate::tes::reference_3d::Reference3D>(actor)
        .map(|loaded| loaded.0)
    else {
        return Ok(());
    };
    if the_player(world) == Some(actor) {
        return Err(GameError::stop(0x004E1937, "the player's weapon model"));
    }
    let skin = world
        .get::<Character>(actor)
        .and_then(|character| character.skin_info_104);
    match skin {
        Some(skin) => {
            let services = world
                .get_resource::<crate::bs::model_loader::GameplayModelServices>()
                .ok_or_else(|| GameError::stop(0x00479780, "a weapon part needs model services"))?
                .0;
            services(world, &mut |world, services| {
                super::biped_attach::attach_weapon_part(world, skin, weapon, services)
            })?;
        }
        None => {
            world
                .get_resource_or_init::<crate::tes::reference_3d::CreatureBodyDeferrals>()
                .weapons
                .push((actor, weapon));
            return Ok(());
        }
    }
    if weapon_type(world, weapon) == 5 {
        return Err(GameError::stop(
            0x004E1A9C,
            "a bow's arrow attach (005E13D0)",
        ));
    }
    find_skin_property(world, process, node)?;
    super::actor_fade::actor_update_alpha(world, actor)
}

/// IDA: the process arms 004E3490's LABEL_94 runs for an actor whose process is level 0 or 1 (a MiddleHigh or High one):
/// vt+0xEC(1) (0064B240: +0xE4 when worn) into vt+0x104 (00658550: the same entry keeps its stack; with animation data,
/// 006541A0 on its manager and node), then vt+0x114 (0064F5E0: +0x110 the "ArrowBone" under the anim data's node with a weapon,
/// else null). A lower level, or no process, does nothing. Completion: 100% of a non-player with the same weapon entry.
/// # Errors
/// The player's arms, or a different weapon entry.
pub(crate) fn process_animation_nodes(
    world: &mut World,
    actor: Entity,
    anim: Option<Entity>,
) -> Result<(), GameError> {
    let Some(process) = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
    else {
        return Ok(());
    };
    if super::actor_process::process_level(world, process) > 1 {
        return Ok(());
    }
    // 004E3EAC: the player also takes vt+0x114 with 0065D750(1)'s first-person anim data; with a weapon that sets 00B3BA98
    // (a stop here), with none +0x110 stays null. Its 00658550 with no weapon zeroes 00B3BA84..00B3BA98, already zero.
    if the_player(world) == Some(actor)
        && world
            .get::<MiddleHighProcess>(process)
            .is_some_and(|middle| middle.weapon_e4.is_some())
    {
        return Err(GameError::unclassified(
            0x004E3EAC,
            "004E3EAC: the player's first-person weapon nodes (00B3BA84..00B3BA98)",
        ));
    }
    let middle =
        world
            .get::<MiddleHighProcess>(process)
            .cloned()
            .ok_or(GameError::unclassified(
                0x004E3E78,
                "004E3E78: a level 0 or 1 process with no MiddleHigh part",
            ))?;
    let worn = middle.weapon_e4.filter(|entry| entry.worn);
    if worn.is_none() && middle.weapon_e4.is_some() {
        return Err(GameError::unclassified(
            0x00658563,
            "00658563: replacing the process's weapon entry with null",
        ));
    }
    let data = anim.and_then(|anim| {
        world
            .get::<super::actor_animation::ActorAnimData>(anim)
            .map(|data| (data.manager_98, data.node_04))
    });
    // 00658550: with animation data and a weapon entry, 006541A0; otherwise +0xFC..+0x110 zeroed.
    let mut nodes = super::actor_process::WeaponNodes::default();
    if let (Some(entry), Some((manager, node))) = (worn, data) {
        let kind = Some(weapon_type(world, entry.form_08));
        nodes = if node.is_some() {
            weapon_bone_nodes(world, manager, node, kind)?
        } else {
            let root = world
                .get::<crate::tes::reference_3d::Reference3D>(actor)
                .map(|loaded| loaded.0);
            weapon_bone_nodes(world, None, root, kind)?
        };
        if let Some(weapon) = nodes.weapon_fc {
            crate::gamebryo::scene_node::detach_scabbard(world, weapon);
        }
    }
    nodes.arrow_bone_110 = match (data, middle.weapon_e4) {
        (Some((_, Some(node))), Some(_)) => object_by_name_with_palette(world, node, b"ArrowBone"),
        _ => None,
    };
    world
        .get_mut::<MiddleHighProcess>(process)
        .expect("read above")
        .weapon_nodes_fc = nodes;
    Ok(())
}
