//! `0047BC40` SkinInfo_AttachBipedModels: each skin slot's model loaded, cloned and put on the actor's skeleton, a skinned part
//! under a wrapper with its bones taken from the skeleton (`0047B5B0`, `0047B090`), a rigid one under its "Prn" bone (`00479140`).

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::actor::{MobileObject, TesObjectRefr};
use super::character_3d::{FaceGenSettings, SkinInfo, SkinSlot, SkinSlotScratch, skin_model_path};
use super::form_components::TesBipedModelForm;
use crate::bs::model_loader::{ModelBuildServices, load_model_now};
use crate::bs::scene_graphs::{AttachChildStatus, attach_scene_child};
use crate::gamebryo::av_object::{NiAvCollision, NiAvParent, NiAvProperties};
use crate::gamebryo::clone::{CloningProcess, clone_object};
use crate::gamebryo::extra_data::StringExtraData;
use crate::gamebryo::mesh_geometry::NiGeometry;
use crate::gamebryo::mesh_skin_owners::MeshSkinInstance;
use crate::gamebryo::object_net::{NiExtraDataName, NiObjectNet};
use crate::gamebryo::scene_node::{
    NiNodeChildren, SceneNodePayload, detach_child, spawn_scene_node,
};
use crate::gamebryo::stream_objects::NiClass;

/// IDA: 00B06588, the slot names 0047BC40 puts first in a part's name.
const SLOT_NAMES_B06588: [&str; 16] = [
    "Head",
    "Hair",
    "UpperBody",
    "LowerBody",
    "Hand",
    "Foot",
    "RightRing",
    "LeftRing",
    "Amulet",
    "Weapon",
    "BackWeapon",
    "SideWeapon",
    "Quiver",
    "Shield",
    "Torch",
    "Tail",
];
/// IDA: 00B065C8, the bone entry each slot's rigid part may hang on (-1 for none).
const SLOT_BONE_B065C8: [i32; 16] = [0, 0, -1, -1, -1, -1, 1, 2, -1, 3, 4, 5, 6, 7, 8, -1];
const IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// IDA: N/A host deferral, an extension of D13: `0047AC20`, the texture a "skin" material part takes: the race's body texture
/// for its part (`0052D2C0` with the NPC's FaceGen coefficients) into its NiTexturingProperty, or a shader property's
/// textures (`BSShaderManager_LoadTexture` for the "_n" map, `004783A0`) with +0x1C |= 0x400. Texturing is renderer state;
/// the geometry and its slot are recorded.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct SkinTextureDeferrals {
    pub parts: Vec<(Entity, usize)>,
}

// IDA: N/A a node's NiObjectNET name.
fn name_of(world: &World, object: Entity) -> Option<Vec<u8>> {
    world
        .get::<NiObjectNet>(object)
        .and_then(|net| net.name.clone())
}

// IDA: N/A the slots of a NiNode's child array (+0xB0 below +0xB6).
fn child_slots(world: &World, node: Entity) -> Vec<Option<Entity>> {
    world
        .get::<NiNodeChildren>(node)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default()
}

// IDA: 004784A0 then 00477F90 on a NiNode's +0xAC NiTArray: a size other than the used count compacts the nonnull slots to the
// front and reallocates to the used count; trailing null slots then drop. The host vector holds the slots alone.
fn compact_children(world: &mut World, node: Entity) {
    if let Some(mut children) = world.get_mut::<NiNodeChildren>(node) {
        children.slots_ac.retain(Option::is_some);
    }
}

/// IDA: 00471B80(node): a NiNode's children in slot order, a NiNode recursed, a geometry (vt+0x0C) with a +0xB8 skin instance
/// answering true. Completion: 100%.
#[must_use]
pub fn has_skin(world: &World, node: Entity) -> bool {
    child_slots(world, node).into_iter().flatten().any(|child| {
        if world.get::<NiNodeChildren>(child).is_some() {
            has_skin(world, child)
        } else {
            world
                .get::<NiGeometry>(child)
                .is_some_and(|geometry| geometry.skin_b8.is_some())
        }
    })
}

// IDA: 006FFAC0 NiObjectNET_RemoveExtraDataByName(name): the sorted list's binary search, the entry removed (0070001A).
fn remove_extra_data(world: &mut World, object: Entity, name: &[u8]) {
    let extras = world
        .get::<NiObjectNet>(object)
        .map(|net| net.extra_data.clone())
        .unwrap_or_default();
    let found = extras.iter().position(|&extra| {
        world
            .get::<NiExtraDataName>(extra)
            .is_some_and(|extra_name| extra_name.0.as_deref() == Some(name))
    });
    if let Some(index) = found
        && let Some(mut net) = world.get_mut::<NiObjectNet>(object)
    {
        net.extra_data.remove(index);
    }
}

// IDA: NiObjectNET_GetExtraData(name) for a NiStringExtraData's value (00B3FCC0's cast; another class answers `Err`).
fn string_extra(world: &World, object: Entity, name: &[u8]) -> Option<Result<Vec<u8>, ()>> {
    let extras = world.get::<NiObjectNet>(object)?.extra_data.clone();
    let extra = extras.into_iter().find(|&extra| {
        world
            .get::<NiExtraDataName>(extra)
            .is_some_and(|extra_name| extra_name.0.as_deref() == Some(name))
    })?;
    Some(
        world
            .get::<StringExtraData>(extra)
            .map(|data| data.value.clone().unwrap_or_default())
            .ok_or(()),
    )
}

// IDA: 00897A90 NiAVObject_SetBodyCollisionEnabled(node, 1): the bhkNiCollisionObject's body (+0x10) through 0089F520(1): its
// filter's bit 14 cleared. Host substitution (D2), as `skin_info_find_bones`' arm 0: the loaded cinfo's word 0 holds the filter.
fn set_body_collision_enabled(world: &mut World, node: Entity) {
    let body = world
        .get::<NiAvCollision>(node)
        .and_then(|collision| {
            world.get::<crate::bs::havok_stream::BhkSpCollisionObject>(collision.0)
        })
        .and_then(|collision| collision.body_10);
    if let Some(body) = body
        && let Some(mut rigid) = world.get_mut::<crate::bs::havok_stream::BhkRigidBody>(body)
    {
        let filter = u32::from_le_bytes(rigid.cinfo_0c[..4].try_into().expect("a cinfo")) & !0x4000;
        rigid.cinfo_0c[..4].copy_from_slice(&filter.to_le_bytes());
    }
}

// IDA: 00707E40 NiAVObject_SetCollisionObject(null): +0xA8 released (the host's link and its stream index).
fn clear_collision(world: &mut World, object: Entity) {
    world.entity_mut(object).remove::<NiAvCollision>();
    if let Some(mut payload) = world.get_mut::<SceneNodePayload>(object) {
        payload.collision_index = crate::gamebryo::stream_objects::NULL_REFERENCE;
    }
}

/// IDA: 0088D000(object, 1, 1) with 00B2E310's 0088A9F0: 0088A7D0 walks the tree (the flags arm skipped, the context's +0x08 is
/// 4), and each object's bhkNiCollisionObject (0089E8A0's +0xA8) has its target's +0xA8 released. Completion: 100%.
fn release_collision_objects(world: &mut World, object: Entity) {
    if world.get::<NiAvCollision>(object).is_some_and(|collision| {
        world
            .get::<crate::bs::havok_stream::BhkSpCollisionObject>(collision.0)
            .is_some()
    }) {
        clear_collision(world, object);
    }
    for child in child_slots(world, object).into_iter().flatten() {
        release_collision_objects(world, child);
    }
}

// IDA: 004D9C30 TESObjectREFR_GetBaseFullNameOrEmpty.
fn owner_full_name(world: &World, owner: Entity) -> Vec<u8> {
    world
        .get::<TesObjectRefr>(owner)
        .and_then(|refr| refr.base_form_1c)
        .map(|base| super::reference_3d::base_full_name(world, base))
        .unwrap_or_default()
}

// IDA: N/A DebugMessage_Warning (004A7A60) through the build services.
fn warn(services: &mut ModelBuildServices<'_>, text: String) {
    (services.warning)(
        text.into_bytes(),
        *services.disable_warning_messages_b06b18 != 0,
    );
}

/// IDA: 0047B090(skin, node, wrapper, slot, first-person, helmet model) for a third-person skin: 004784A0/00477F90 on the node's
/// children, its +0x54 and the wrapper's zeroed; each child in slot order: a non-geometry (vt+0x0C) detached (vt+0x88) and
/// released; a geometry: `0047AC20`'s texture (deferred, `SkinTextureDeferrals`), its +0xB8 skin instance's bones (+0x14, the
/// skin data's +0x40 of them) each replaced by the skeleton's node of its name (006F94A0 from +0x00, "Bone '%s' not found ..."
/// otherwise, keeping the old), and with a wrapper its +0x10 root parent the wrapper and the geometry attached to it (vt+0x84,
/// 1). The node's children compact again. Completion: 100% of a third-person non-helmet slot.
fn bind_skinned_node(
    world: &mut World,
    skin: &SkinInfo,
    node: Entity,
    wrapper: Option<Entity>,
    slot: usize,
    first_person: bool,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), StreamError> {
    let skeleton = skin
        .bip01_00
        .ok_or(StreamError("0047B090: a skin info with no Bip01"))?;
    compact_children(world, node);
    if slot == 1 {
        return Err(StreamError(
            "0047B0F4: a helmet part's FaceGen model (SkinInfo_GetHelmetFaceGenModel)",
        ));
    }
    for object in [Some(node), wrapper].into_iter().flatten() {
        if let Some(mut payload) = world.get_mut::<SceneNodePayload>(object) {
            payload.local_transform.translation = [0.0; 3];
        }
    }
    for child in child_slots(world, node).into_iter().flatten() {
        let Some(geometry) = world.get::<NiGeometry>(child).cloned() else {
            detach_child(world, node, child);
            continue;
        };
        // 0047B16E: the first-person skin drops a child whose name starts "UpperBody" (vt+0x88).
        if first_person
            && name_of(world, child)
                .is_some_and(|name| recomp_core::crt::strnicmp(&name, b"UpperBody", 9) == 0)
        {
            detach_child(world, node, child);
            continue;
        }
        if let Some(properties) = world
            .get::<NiAvProperties>(child)
            .map(|list| list.0.clone())
        {
            let skin_material = properties.into_iter().flatten().any(|property| {
                world.get::<NiClass>(property) == Some(&NiClass::MaterialProperty)
                    && name_of(world, property)
                        .is_some_and(|name| recomp_core::crt::stricmp(&name, b"skin") == 0)
            });
            if skin_material {
                world
                    .get_resource_or_init::<SkinTextureDeferrals>()
                    .parts
                    .push((child, slot));
            }
        }
        let Some(instance) = geometry.skin_b8 else {
            continue;
        };
        let bones = world
            .get::<MeshSkinInstance>(instance)
            .map(|instance| instance.bone_owners_14.clone())
            .unwrap_or_default();
        let mut rebound = bones.clone();
        for (index, bone) in bones.iter().enumerate() {
            let Some(name) = bone.and_then(|bone| name_of(world, bone)) else {
                continue;
            };
            match super::actor_enable::object_by_name_with_palette(world, skeleton, &name) {
                Some(found) => rebound[index] = Some(found),
                None => {
                    let text = |bytes: Option<Vec<u8>>| {
                        bytes.map_or_else(
                            || "NULL".to_owned(),
                            |bytes| String::from_utf8_lossy(&bytes).into_owned(),
                        )
                    };
                    let bone_name = String::from_utf8_lossy(&name).into_owned();
                    let part = text(name_of(world, child));
                    let own = name_of(world, child).unwrap_or_default();
                    let message = if recomp_core::crt::strnicmp(&own, &name, name.len()) == 0 {
                        let parent = world
                            .get::<NiAvParent>(child)
                            .and_then(|parent| parent.0)
                            .and_then(|parent| name_of(world, parent));
                        format!(
                            "Bone '{bone_name}' not found for part '{}->{part}'.\r\nMake sure all the vertices are skinned to a bone in Max.",
                            text(parent)
                        )
                    } else {
                        return Err(StreamError(
                            "0047B4C4: the \"Bone not found\" warning's base name and model",
                        ));
                    };
                    warn(services, message);
                }
            }
        }
        if let Some(mut instance) = world.get_mut::<MeshSkinInstance>(instance) {
            instance.bone_owners_14 = rebound;
            if let Some(wrapper) = wrapper {
                instance.root_parent_owner_10 = Some(wrapper);
            }
        }
        if let Some(wrapper) = wrapper
            && attach_scene_child(world, wrapper, Some(child), true) != AttachChildStatus::Attached
        {
            return Err(StreamError(
                "0047B53A: attaching a skinned geometry to its wrapper",
            ));
        }
    }
    compact_children(world, node);
    Ok(())
}

/// IDA: 0047B5B0(skin, part, slot, first-person, helmet model): the part is kept as its own wrapper when it is a NiBSLODNode
/// (00B35408); slot +0x0C = 00471B80. No skeleton (+0x00) answers the part, no skin null. Otherwise the wrapper (or a new
/// NiNode, 0070A8A0) takes a zero translate and the identity, goes under the skeleton's "SkinAttachment" (006F94A0) or the
/// skeleton's parent (+0x1C) through vt+0x84(wrapper, 1), and 0047B090 runs on each LOD child, or on the part with the new
/// wrapper. Answers the wrapper. Completion: 100% of a third-person skin.
fn attach_skinned_part(
    world: &mut World,
    skin_entity: Entity,
    part: Entity,
    slot: usize,
    first_person: bool,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, StreamError> {
    // NiBSLODNode (00B35408) isn't a registered stream class, so no part loads as one here.
    let lod = false;
    let skinned = has_skin(world, part);
    world
        .get_mut::<SkinInfo>(skin_entity)
        .expect("a skin info")
        .slots_4c[slot]
        .skinned_0c = u32::from(skinned);
    let skin = world
        .get::<SkinInfo>(skin_entity)
        .expect("a skin info")
        .clone();
    let Some(skeleton) = skin.bip01_00 else {
        return Ok(Some(part));
    };
    if !skinned {
        return Ok(None);
    }
    let wrapper = if lod {
        part
    } else {
        spawn_scene_node(world, SceneNodePayload::default())
    };
    if let Some(mut payload) = world.get_mut::<SceneNodePayload>(wrapper) {
        payload.local_transform.translation = [0.0; 3];
        payload.local_transform.rotation = IDENTITY;
    }
    let parent =
        super::actor_enable::object_by_name_with_palette(world, skeleton, b"SkinAttachment")
            .or_else(|| {
                world
                    .get::<NiAvParent>(skeleton)
                    .and_then(|parent| parent.0)
            })
            .ok_or(StreamError("0047B6E3: a skeleton with no parent faults"))?;
    if attach_scene_child(world, parent, Some(wrapper), true) != AttachChildStatus::Attached {
        return Err(StreamError("0047B6EE: attaching the skin's wrapper"));
    }
    if lod {
        for child in child_slots(world, part).into_iter().flatten() {
            bind_skinned_node(world, &skin, child, None, slot, first_person, services)?;
        }
    } else {
        bind_skinned_node(
            world,
            &skin,
            part,
            Some(wrapper),
            slot,
            first_person,
            services,
        )?;
    }
    Ok(Some(wrapper))
}

/// IDA: 00479140(skeleton, part, null, skin, slot): the part's "Prn" (00A3CEAC) NiStringExtraData names its parent (the left
/// ring's with byte 6 'L' for the lookup); found (006F94A0 from the skeleton): vt+0x84(part, 1), and the part's "Scb" child
/// goes under the same bone with 00897A90(bone, 1), its property and effect state refreshed and the part's children compacted.
/// Not found: a creature's part takes NiNode_DetachScabbard, anything else the "Could not find parent node" warning. Each arm
/// ends in 0088D000(part, 1, 1). Completion: 100%.
fn attach_rigid_part(
    world: &mut World,
    skeleton: Entity,
    part: Entity,
    slot: usize,
    owner: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), StreamError> {
    let name = name_of(world, part).map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    match string_extra(world, part, b"Prn") {
        None => warn(
            services,
            format!(
                "Could not find parent node extra data for '{}'.",
                name.unwrap_or_default()
            ),
        ),
        Some(Err(())) => warn(
            services,
            format!(
                "Extra data 'Prn' on '{}' is not an NiStringExtraData.",
                name.unwrap_or_default()
            ),
        ),
        Some(Ok(mut parent_name)) => {
            if slot == 7 && parent_name.len() > 6 {
                parent_name[6] = b'L';
            }
            let bone =
                super::actor_enable::object_by_name_with_palette(world, skeleton, &parent_name);
            if slot == 7 && parent_name.len() > 6 {
                parent_name[6] = b'R';
            }
            match bone {
                Some(bone) => {
                    if attach_scene_child(world, bone, Some(part), true)
                        != AttachChildStatus::Attached
                    {
                        return Err(StreamError("004791C6: attaching a part to its bone"));
                    }
                    if let Some(scabbard) =
                        super::actor_enable::object_by_name_with_palette(world, part, b"Scb")
                    {
                        if attach_scene_child(world, bone, Some(scabbard), true)
                            != AttachChildStatus::Attached
                        {
                            return Err(StreamError("004791E8: attaching a scabbard to its bone"));
                        }
                        set_body_collision_enabled(world, bone);
                        crate::gamebryo::scene_state::refresh_property_state(
                            world,
                            scabbard,
                            &services.prepare.property_defaults,
                        )?;
                        crate::gamebryo::scene_state::refresh_effect_state(world, scabbard)?;
                        compact_children(world, part);
                    }
                }
                None => {
                    let creature = world
                        .get::<TesObjectRefr>(owner)
                        .and_then(|refr| refr.base_form_1c)
                        .is_some_and(|base| super::forms_init::form_type(world, base) == 0x24);
                    if creature {
                        return Err(StreamError(
                            "00479260: NiNode_DetachScabbard on a creature's part",
                        ));
                    }
                    warn(
                        services,
                        format!(
                            "Could not find parent node '{}' for object '{}'.",
                            String::from_utf8_lossy(&parent_name),
                            name.unwrap_or_default()
                        ),
                    );
                }
            }
        }
    }
    release_collision_objects(world, part);
    Ok(())
}

// IDA: 0070FD30 NiMatrix33_MakeXRotation(angle): rows (1, 0, 0), (0, cos, sin), (0, -sin, cos), each `cos`/`sin` stored as a
// float.
fn x_rotation(angle: f32) -> [f32; 9] {
    let (cos, sin) = (f64::from(angle).cos() as f32, f64::from(angle).sin() as f32);
    [1.0, 0.0, 0.0, 0.0, cos, sin, 0.0, -sin, cos]
}

/// IDA: 00656270 MiddleHighProcess vt+0x150(drawn, skin, anim, actor), as 0047BC40 runs it for slot 9 with drawn = vt+0x304
/// (0064AD30, +0x115): 00651DD0 (each of +0xFC..+0x110's nodes takes scale 1.0, 00B33D80 being clear); with a weapon (+0xE4):
/// the hand bone (vt+0x118, the skin's bone 3, or vt+0x11C's bone 8 for a bow, +0xF4) and the sheath (vt+0x124 006508D0: bone
/// 4 for weapon types 1 and 3..5, else 5). With a sheath, the weapon moves from the hand to the sheath (undrawn) or back
/// (drawn): the source's first child, or with none (or a bow's 00478180 arrow) the target's; it takes the identity and a zero
/// translate, a staff on the sheath the x rotation pi, a bow 0.116465814 with y 26.726. No sheath with a weapon undrawn: the
/// hand's child detached and the item unequipped (a stop). Then for an NPC with animation data and no RaceSexMenu (1036):
/// undrawn 00471990 (the power-attack groups 22..26 out of the anim map: none there before 00475D80, checked), drawn
/// 00476410 (a stop). Last 005EF9B0(drawn) on the actor: undrawn, its "Weapon" node's collision object goes (00707E40 with
/// null); drawn is a stop. Completion: 100% of an undrawn weapon with a sheath.
pub(crate) fn place_weapon(
    world: &mut World,
    process: Entity,
    skin_entity: Entity,
    actor: Entity,
) -> Result<(), StreamError> {
    let Some(middle) = world
        .get::<super::actor_process::MiddleHighProcess>(process)
        .cloned()
    else {
        return super::actor_enable::low_process_only(
            world,
            process,
            "00656270: a process's vt+0x150",
        );
    };
    let drawn = middle.byte_115 != 0;
    let nodes = middle.weapon_nodes_fc;
    for node in [
        nodes.weapon_fc,
        nodes.torch_100,
        nodes.forearm_twist_104,
        nodes.sheath_108,
        nodes.quiver_10c,
        nodes.arrow_bone_110,
    ]
    .into_iter()
    .flatten()
    {
        if let Some(mut payload) = world.get_mut::<SceneNodePayload>(node) {
            payload.local_transform.scale = 1.0;
        }
    }
    let skin = world
        .get::<SkinInfo>(skin_entity)
        .expect("a skin info")
        .clone();
    if let Some(entry) = middle.weapon_e4 {
        let kind = super::actor_enable::weapon_type_of(world, entry.form_08);
        let hand = if middle.bow_f4 != 0 {
            skin.bones_04[8].node_04
        } else {
            skin.bones_04[3].node_04
        };
        let sheath = if matches!(kind, 1 | 3..=5) {
            skin.bones_04[4].node_04
        } else {
            skin.bones_04[5].node_04
        };
        if let Some(hand) = hand {
            let Some(sheath) = sheath else {
                if !drawn {
                    return Err(StreamError(
                        "006563F0: no sheath: the weapon's detach and unequip",
                    ));
                }
                return finish_weapon_placement(world, actor, drawn);
            };
            let first = |world: &World, node: Entity| {
                world
                    .get::<NiNodeChildren>(node)
                    .and_then(|children| children.slots_ac.first().copied().flatten())
            };
            let (from, to) = if drawn {
                (sheath, hand)
            } else {
                (hand, sheath)
            };
            if middle.bow_f4 != 0 {
                return Err(StreamError("006562FF: a bow's arrow test (00478180)"));
            }
            let moved = match first(world, from) {
                Some(child) => {
                    if attach_scene_child(world, to, Some(child), true)
                        != AttachChildStatus::Attached
                    {
                        return Err(StreamError("00656330: moving the weapon between bones"));
                    }
                    Some(child)
                }
                None => first(world, to),
            };
            if let Some(child) = moved {
                let mut rotation = IDENTITY;
                let mut translate = [0.0; 3];
                if !drawn && kind == 4 {
                    rotation = x_rotation(std::f32::consts::PI);
                } else if !drawn && kind == 5 {
                    rotation = x_rotation(0.116_465_814);
                    translate[1] = 26.726;
                }
                if let Some(mut payload) = world.get_mut::<SceneNodePayload>(child) {
                    payload.local_transform.rotation = rotation;
                    payload.local_transform.translation = translate;
                }
            }
        }
    }
    finish_weapon_placement(world, actor, drawn)
}

// IDA: 00656270's tail (LABEL_26): 005E32D0, the anim data and 0x40C's TileMenu, then 005EF9B0.
fn finish_weapon_placement(
    world: &mut World,
    actor: Entity,
    drawn: bool,
) -> Result<(), StreamError> {
    let npc = world
        .get::<TesObjectRefr>(actor)
        .and_then(|refr| refr.base_form_1c)
        .is_some_and(|base| super::forms_init::form_type(world, base) == 0x23);
    // 004D8370 TESObjectREFR_GetAnimData: a MiddleHigh process's +0x17C (the NPCs here have one).
    let anim = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
        .and_then(|process| world.get::<super::actor_process::MiddleHighProcess>(process))
        .and_then(|middle| middle.anim_data_17c);
    let race_menu = world
        .get_resource::<super::interface_manager::InterfaceMovieReads>()
        .is_some_and(|reads| reads.tile_menu_present(1036));
    if npc
        && let Some(anim) = anim
        && !race_menu
    {
        if drawn {
            return Err(StreamError("00656436: 00476410 for a drawn weapon"));
        }
        let groups = world
            .get::<super::actor_animation::ActorAnimData>(anim)
            .map(|data| {
                data.sequences_9c
                    .iter()
                    .flatten()
                    .map(|(key, _)| *key)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if groups.iter().any(|&key| (22..=26).contains(&(key & 0xFF))) {
            return Err(StreamError(
                "004719C4: removing the power-attack groups (00471990)",
            ));
        }
    }
    if drawn {
        return Err(StreamError(
            "005EF9D8: a drawn weapon's reach collision (0053A130)",
        ));
    }
    if let Some(node) = world
        .get::<super::reference_3d::Reference3D>(actor)
        .map(|loaded| loaded.0)
        && let Some(weapon) =
            super::actor_enable::object_by_name_with_palette(world, node, b"Weapon")
    {
        clear_collision(world, weapon);
    }
    Ok(())
}

// IDA: 007100A0 NiMatrix33_Multiply(self, out, other) with a diagonal `other`: each entry its row's product with the column's
// diagonal (the zero products add nothing).
fn scale_columns(matrix: &[f32; 9], diagonal: [f32; 3]) -> [f32; 9] {
    let mut out = [0.0; 9];
    for row in 0..3 {
        for column in 0..3 {
            out[row * 3 + column] = diagonal[column] * matrix[row * 3 + column];
        }
    }
    out
}

/// IDA: 004D7C50 TESObjectREFR_ApplyRaceScaleToRotation(ref, out, rotation, inverse): an NPC base with a race (+0xE8) scales
/// the rotation's columns by the race's weight (+0x68 + 4 * sex, columns 0 and 1) and height (+0x60 + 4 * sex, column 2), or
/// by their inverses, 0.0 for a sex past 1. Completion: 100%.
/// # Errors
/// A base whose race can't be read.
pub(crate) fn race_scaled_rotation(
    world: &World,
    reference: Entity,
    rotation: &[f32; 9],
    inverse: bool,
) -> Result<[f32; 9], StreamError> {
    let Some(base) = world
        .get::<TesObjectRefr>(reference)
        .and_then(|refr| refr.base_form_1c)
    else {
        return Ok(*rotation);
    };
    if super::forms_init::form_type(world, base) != 0x23 {
        return Ok(*rotation);
    }
    let Some(race) = super::hud_main_menu::base_race(world, base)? else {
        return Ok(*rotation);
    };
    let (weight, height) = match super::hud_main_menu::npc_sex(world, base) {
        Some(sex) if sex <= 1 => (
            super::hud_main_menu::race_scale(world, race, 0x68 + 4 * sex),
            super::hud_main_menu::race_scale(world, race, 0x60 + 4 * sex),
        ),
        _ => (0.0, 0.0),
    };
    let (across, up) = if inverse {
        (1.0 / weight, 1.0 / height)
    } else {
        (weight, height)
    };
    Ok(scale_columns(rotation, [across, across, up]))
}

/// IDA: 00478290(skin, slot) for slots 6..8: whether a slot's form is a TESBipedModelForm whose +0x06 flags hide the rings (bit
/// 0, slots 6 and 7) or the amulet (bit 1, slot 8). Completion: 100%.
fn slot_hidden(world: &World, skin: &SkinInfo, slot: usize) -> bool {
    let bit = match slot {
        6 | 7 => 1,
        8 => 2,
        _ => return false,
    };
    skin.slots_4c.iter().any(|entry| {
        entry
            .form_00
            .and_then(|form| world.get::<TesBipedModelForm>(form))
            .is_some_and(|biped| biped.flags_06 & bit != 0)
    })
}

/// IDA: 0047BC40 SkinInfo_AttachBipedModels(skin) for a third-person skin with 00B33D80 clear (00526DB0 doesn't set it). Each
/// of the sixteen slots: one whose model equals the 00B33C80 stash's takes the stash's four words back (the stash zeroed) and
/// keeps them, a ring or amulet hidden (00478290) releasing it; otherwise a stashed node is released (a stop here). Then a slot
/// with a model, not hidden: slot 1 with the background helmet settings and no node goes to SkinInfo_QueueHelmet (D12); slot
/// 14 is a stop. ModelLoader_LoadModelNow(path, 1, 3, 1), cloned at scale 1 (a NiGeomMorpherController's 004430C0 is a stop);
/// a BSFadeNode clone takes stage 7, loses its "REF" extra and takes a zero translate and the identity; `attach_skinned_part`
/// (0047B5B0), else `attach_rigid_part` (00479140) with slot 9's process vt+0x150 (00656270, a stop); the part's name
/// "%s %s (%08X)" (the slot, the form's vt+0xD4, which is "" for every form class here, and its ID); a skinned part's rotation
/// through 004D7C50; 00478220 (a BSX flag 0x10 model's flame nodes, a stop); and for an unskinned NiNode part, a slot with no
/// bone entry warns "The biped part '%s' should be skinned for '%s'." and drops it, one with no parent goes under its bone.
/// The slot's +0x08 takes the part. Last, the owner's node's property and effect state. Completion: 100% of those arms.
/// # Errors
/// The stops above.
pub fn attach_biped_models(
    world: &mut World,
    skin_entity: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), StreamError> {
    let owner = world
        .get::<SkinInfo>(skin_entity)
        .expect("a skin info")
        .owner_150;
    // 0047BC56 PlayerCharacter_IsFirstPersonSkinInfo: the player's +0x5C8.
    let first_person = Some(owner) == super::ai_conditions::the_player(world)
        && world
            .get::<super::player::PlayerCharacter>(owner)
            .is_some_and(|player| player.first_person_skin_info_5c8 == Some(skin_entity));
    for slot in 0..16 {
        let skin = world
            .get::<SkinInfo>(skin_entity)
            .expect("a skin info")
            .clone();
        let entry = skin.slots_4c[slot];
        let stash = world.get_resource_or_init::<SkinSlotScratch>().slots_b33c80[slot];
        if entry.model_04.is_some() && entry.model_04 == stash.model_04 {
            world
                .get_mut::<SkinInfo>(skin_entity)
                .expect("a skin info")
                .slots_4c[slot] = stash;
            world.resource_mut::<SkinSlotScratch>().slots_b33c80[slot] = SkinSlot::default();
            if (6..=8).contains(&slot) && slot_hidden(world, &skin, slot) {
                return Err(StreamError(
                    "0047BCF1: releasing a hidden ring's stashed slot (SkinInfo_ReleaseSlot)",
                ));
            }
            if stash.skinned_0c != 0 && stash.node_08.is_some() {
                return Err(StreamError(
                    "0047BD9A: a stashed skinned part's race rotation",
                ));
            }
            let empty = (6..=8).contains(&slot) && stash.node_08.is_none();
            if !empty {
                continue;
            }
        } else if stash.node_08.is_some() {
            return Err(StreamError(
                "0047BDD6: releasing a stashed slot's node (SkinInfo_ReleaseSlot)",
            ));
        }
        let skin = world
            .get::<SkinInfo>(skin_entity)
            .expect("a skin info")
            .clone();
        let entry = skin.slots_4c[slot];
        if (6..=8).contains(&slot) && slot_hidden(world, &skin, slot) {
            continue;
        }
        if entry.model_04.is_none() {
            continue;
        }
        // 0047BD8C: the first-person skin attaches only the slots the first-person view shows.
        if first_person && !matches!(slot, 2 | 4 | 6 | 7 | 9 | 12 | 13 | 14) {
            continue;
        }
        if slot == 1 {
            let settings = *world.get_resource_or_init::<FaceGenSettings>();
            if entry.node_08.is_none()
                && settings.load_helmets_in_background_b06608
                && settings.use_face_gen_heads_b120b4
                && settings.use_multi_threaded_face_gen_b120dc
            {
                // 0047BE40 SkinInfo_QueueHelmet: the QueuedHelmet is host deferral D12 (`FaceGenDeferrals`), as 004788E0's.
                continue;
            }
        }
        if slot == 14 {
            return Err(StreamError("0047BE6E: the torch slot's process vt+0x348"));
        }
        let path = skin_model_path(world, entry.model_04);
        let Some(model) = load_model_now(world, &path, true, 3, true, services)? else {
            continue;
        };
        if super::reference_3d::has_geom_morpher(world, model) {
            return Err(StreamError(
                "0047BEEB: a part's NiGeomMorpherController clone (004430C0)",
            ));
        }
        let mut process = CloningProcess {
            scale_10: [1.0; 3],
            ..CloningProcess::default()
        };
        let part = clone_object(world, &mut process, model, &mut services.prepare)?;
        if let Some(mut fade) = world.get_mut::<crate::bs::fade_node::BsFadeNode>(part) {
            crate::bs::fade_node::fade_node_set_stage(&mut fade, 7);
        }
        remove_extra_data(world, part, b"REF");
        if let Some(mut payload) = world.get_mut::<SceneNodePayload>(part) {
            payload.local_transform.translation = [0.0; 3];
            payload.local_transform.rotation = IDENTITY;
        }
        let skeleton = skin.bip01_00;
        let mut placed =
            match attach_skinned_part(world, skin_entity, part, slot, first_person, services)? {
                Some(wrapper) => wrapper,
                None => {
                    let skeleton =
                        skeleton.ok_or(StreamError("0047C1D8: a rigid part with no skeleton"))?;
                    attach_rigid_part(world, skeleton, part, slot, owner, services)?;
                    if slot == 9
                        && !super::forms_references::save_load_active(world)?
                        && let Some(process) = world
                            .get::<MobileObject>(owner)
                            .and_then(|mobile| mobile.process_58)
                    {
                        place_weapon(world, process, skin_entity, owner)?;
                    }
                    part
                }
            };
        let form_id = entry
            .form_00
            .and_then(|form| world.get::<super::form::FormIdentity>(form))
            .map_or(0, |identity| identity.form_id_0c);
        let name = format!("{}  ({form_id:08X})", SLOT_NAMES_B06588[slot]);
        if let Some(mut net) = world.get_mut::<NiObjectNet>(placed) {
            net.name = Some(name.into_bytes());
        }
        let skinned = world
            .get::<SkinInfo>(skin_entity)
            .expect("a skin info")
            .slots_4c[slot]
            .skinned_0c
            != 0;
        if skinned {
            let rotation = world
                .get::<SceneNodePayload>(placed)
                .map_or(IDENTITY, |payload| payload.local_transform.rotation);
            let scaled = race_scaled_rotation(world, owner, &rotation, true)?;
            if let Some(mut payload) = world.get_mut::<SceneNodePayload>(placed) {
                payload.local_transform.rotation = scaled;
            }
        }
        if super::reference_3d::bsx_flag(world, model, 0x10) {
            return Err(StreamError(
                "00478238: a part's flame nodes (NiNode_AttachFlameNodes)",
            ));
        }
        if !skinned && world.get::<NiNodeChildren>(placed).is_some() {
            let bone = SLOT_BONE_B065C8[slot];
            if bone == -1 {
                warn(
                    services,
                    format!(
                        "The biped part '{}' should be skinned for '{}'.",
                        String::from_utf8_lossy(&path),
                        String::from_utf8_lossy(&owner_full_name(world, owner))
                    ),
                );
                // 0047C43A: the part leaves its parent (vt+0x88), and the slot's +0x08 is null.
                if let Some(parent) = world.get::<NiAvParent>(placed).and_then(|parent| parent.0) {
                    detach_child(world, parent, placed);
                }
                world
                    .get_mut::<SkinInfo>(skin_entity)
                    .expect("a skin info")
                    .slots_4c[slot]
                    .node_08 = None;
                continue;
            }
            let parent = world.get::<NiAvParent>(placed).and_then(|parent| parent.0);
            if parent.is_none()
                && let Some(node) = skin.bones_04[bone as usize].node_04
                && attach_scene_child(world, node, Some(placed), true)
                    != AttachChildStatus::Attached
            {
                return Err(StreamError("0047C4F8: attaching a part under its bone"));
            }
            placed = part;
        }
        world
            .get_mut::<SkinInfo>(skin_entity)
            .expect("a skin info")
            .slots_4c[slot]
            .node_08 = Some(placed);
    }
    if let Some(node) = world
        .get::<super::reference_3d::Reference3D>(owner)
        .map(|loaded| loaded.0)
    {
        crate::gamebryo::scene_state::refresh_property_state(
            world,
            node,
            &services.prepare.property_defaults,
        )?;
        crate::gamebryo::scene_state::refresh_effect_state(world, node)?;
    }
    Ok(())
}
