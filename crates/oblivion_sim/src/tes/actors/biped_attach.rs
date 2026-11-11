//! `0047BC40` SkinInfo_AttachBipedModels: each skin slot's model loaded, cloned and put on the actor's skeleton, a skinned part
//! under a wrapper with its bones taken from the skeleton (`0047B5B0`, `0047B090`), a rigid one under its "Prn" bone (`00479140`).

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor::{MobileObject, TesObjectRefr};
use super::character_3d::{
    FaceGenSettings, SkinInfo, SkinSlot, SkinSlotAt, SkinSlotScratch, release_skin_slot,
    skin_model_path,
};
use crate::bs::model_loader::{ModelBuildServices, load_model_now};
use crate::bs::scene_graphs::{AttachChildStatus, attach_scene_child};
use crate::gamebryo::av_object::{
    NiAvCollision, NiAvParent, NiAvProperties, clear_collision, deep_copy_geometry_data,
};
use crate::gamebryo::clone::{CloningProcess, clone_object};
use crate::gamebryo::extra_data::StringExtraData;
use crate::gamebryo::mesh_geometry::NiGeometry;
use crate::gamebryo::mesh_skin_owners::MeshSkinInstance;
use crate::gamebryo::object_net::{NiExtraDataName, NiObjectNet, remove_extra_data};
use crate::gamebryo::scene_node::{
    NiNodeChildren, SceneNodePayload, child_slots, detach_child, has_skin, spawn_scene_node,
};
use crate::gamebryo::stream_objects::NiClass;
use crate::tes::form_components::TesBipedModelForm;
use crate::tes::forms_actors::race_body_texture;

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

/// IDA: N/A the base texture `0047AC20` gives a "skin" material part: `0052D2C0`'s "Textures\%s" of the race's body texture
/// (+0x224 + 0x0C (5 sex + part)), which the original makes through TES_CreateTextureImage (`00442890`, quiet 0) and puts in
/// the part's NiTexturingProperty (type 6, through `00700900`, then the old one detached), a shader property's slot 0 (type 4 with
/// shader type 5..10, vt+0x80), or a new NiTexturingProperty (`0047AFAB`). Host deferral with D13's textures, as
/// `FaceGenPartTexture`: the path is kept on the geometry, and the view draws from it in place of the NIF's. `None` is
/// the null texture a part with no NPC owner takes.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct SkinPartTexture(pub Option<Vec<u8>>);

/// IDA: N/A host deferral (D13): what `0047AC20` and `0052D2C0` do with a skin part's texture beyond its base map. With
/// bFaceGenTexturing (00B10D3C), `0052D2C0` takes the NPC's FaceGen texture (the second out, else g_faceGen's +0x80 grey
/// texture from `00523D80`): the texture palette's "%08XModUpperBodyMale", "%08XModUpperBodyFemale" or "%08XModBody", else
/// one built (`00557E60`) from "Meshes\%s" of the static model (00B364A0, 00B364B8 or 00B364D0) as an `.egt` (FaceGen_MakeEgtPath
/// `005500C0`, FaceGen_GetModel `00553620`) with the NPC's coefficients (`005221C0`) and added to the palette. A shader
/// property then takes the "_n" map (`007B4160`, BSShaderManager_LoadTexture) at slot 0, the FaceGen texture at slot 1 with
/// `004783A0`'s g_faceGen +0x40 texture as its vt+0x84, +0x1C |= 0x400 and +0x24 = 0. The port builds none of these
/// textures; each request is recorded, and its build counted as a success (the original answers 0, with the "Missing race
/// texture" warning, when the `.egt` model or its texture can't be made).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct SkinTextureDeferrals {
    pub parts: Vec<SkinTextureRequest>,
}

/// IDA: N/A one `SkinTextureDeferrals` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinTextureRequest {
    pub geometry: Entity,
    /// 0047AC20's biped slot (2 upper body, 3 lower body, 4 hand, 5 foot, 15 tail).
    pub slot: usize,
    /// The texture palette name (`sprintf` of the NPC's form ID).
    pub palette_name: Vec<u8>,
    /// The `.egt` the FaceGen texture would be built from.
    pub egt_path: Vec<u8>,
}

/// IDA: the six prefixes 0047AC20 puts in a stack array and matches a skin part's name against (`_strnicmp`, in order), with
/// the slot its switch picks for each ("Arms" is the upper body).
const SKIN_PART_PREFIXES: [(&[u8], usize); 6] = [
    (b"UpperBody", 2),
    (b"LowerBody", 3),
    (b"Hand", 4),
    (b"Foot", 5),
    (b"Arms", 2),
    (b"Tail", 15),
];

/// IDA: 0047AC20 SkinInfo_ApplySkinTexture(skin, geometry): a geometry (vt+0x0C) whose material property (type 2) is named
/// "skin": the owner's (+0x150) base (vt+0x170) as a TESNPC (`__RTDynamicCast`); the slot from the geometry's name prefix
/// (`SKIN_PART_PREFIXES`, else 3 with "Bad skin name '%s' on '%s'." naming its parent, +0x1C); with an NPC, `0052D2C0` on its
/// race (+0xE8), failing with "Missing '%s' race texture for '%s' in race '%s'." (00B10BC4's sex, 00B06588's slot name, the
/// race's full name +0x1C or ""); then the texture into the part's properties (`SkinPartTexture`). Completion: 100% of the
/// base texture; the texturing and shader properties themselves are renderer state (D13).
// TODO: [ ] the NiTexturingProperty and shader property stores (D13).
fn apply_skin_texture(
    world: &mut World,
    skin: &SkinInfo,
    geometry: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let Some(properties) = world
        .get::<NiAvProperties>(geometry)
        .map(|list| list.0.clone())
    else {
        return Ok(());
    };
    // 0047AC83: NiAVObject_GetPropertyOfType(2), the material property; its name must be "skin".
    let Some(material) = properties
        .into_iter()
        .flatten()
        .find(|&property| world.get::<NiClass>(property) == Some(&NiClass::MaterialProperty))
    else {
        return Ok(());
    };
    if name_of(world, material).is_none_or(|name| oblivion_core::crt::stricmp(&name, b"skin") != 0)
    {
        return Ok(());
    }
    let npc = world
        .get::<TesObjectRefr>(skin.owner_150)
        .and_then(|refr| refr.base_form_1c)
        .filter(|&base| crate::tes::forms_init::form_type(world, base) == 0x23);
    let name = name_of(world, geometry).ok_or(GameError::unclassified(
        0x0047AD6E,
        "0047AD6E: _strnicmp on a skin part with no name",
    ))?;
    let slot = match SKIN_PART_PREFIXES
        .iter()
        .find(|(prefix, _)| oblivion_core::crt::strnicmp(&name, prefix, prefix.len()) == 0)
    {
        Some(&(_, slot)) => slot,
        None => {
            let parent = world
                .get::<NiAvParent>(geometry)
                .and_then(|parent| parent.0)
                .ok_or(GameError::unclassified(
                    0x0047ADC0,
                    "0047ADC0: a bad skin name on a part with no parent",
                ))?;
            let parent_name = name_of(world, parent).unwrap_or_default();
            warn(
                services,
                format!(
                    "Bad skin name '{}' on '{}'.",
                    String::from_utf8_lossy(&name),
                    String::from_utf8_lossy(&parent_name)
                ),
            );
            3
        }
    };
    let texture = match npc {
        None => None,
        Some(npc) => {
            let race = crate::tes::forms_actors::base_race(world, npc)?.ok_or(
                GameError::unclassified(0x0047ADED, "0047ADED: an NPC with no race"),
            )?;
            if let Some(path) = race_body_texture(world, race, Some(npc), slot, geometry) {
                Some(path)
            } else {
                let race_name = world
                    .get::<crate::tes::form_components::TesFullName>(race)
                    .map(|name| crate::tes::face_gen_head::path_text(&name.name_04))
                    .unwrap_or_default();
                let sex = crate::tes::forms_actors::npc_sex(world, npc).unwrap_or(0);
                warn(
                    services,
                    format!(
                        "Missing '{}' race texture for '{}' in race '{}'.",
                        ["Male", "Female"][sex],
                        SLOT_NAMES_B06588[slot],
                        String::from_utf8_lossy(&race_name)
                    ),
                );
                return Ok(());
            }
        }
    };
    world.entity_mut(geometry).insert(SkinPartTexture(texture));
    Ok(())
}

// IDA: N/A a node's NiObjectNET name.
fn name_of(world: &World, object: Entity) -> Option<Vec<u8>> {
    world
        .get::<NiObjectNet>(object)
        .and_then(|net| net.name.clone())
}

// IDA: 004784A0 then 00477F90 on a NiNode's +0xAC NiTArray: a size other than the used count compacts the nonnull slots to the
// front and reallocates to the used count; trailing null slots then drop. The host vector holds the slots alone.
fn compact_children(world: &mut World, node: Entity) {
    if let Some(mut children) = world.get_mut::<NiNodeChildren>(node) {
        children.slots_ac.retain(Option::is_some);
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
        .map(|base| crate::tes::reference_3d::base_full_name(world, base))
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
/// released; a geometry: `0047AC20`'s race texture (`apply_skin_texture`), its +0xB8 skin instance's bones (+0x14, the
/// skin data's +0x40 of them) each replaced by the skeleton's node of its name (006F94A0 from +0x00, "Bone '%s' not found ..."
/// otherwise, keeping the old), and with a wrapper its +0x10 root parent the wrapper and the geometry attached to it (vt+0x84,
/// 1); a helmet (slot 1) geometry first takes `fit_skinned_helmet_part`. The node's children compact again. Completion: 100% of a third-person slot.
#[allow(
    clippy::too_many_arguments,
    reason = "0047B090's six arguments and the build services"
)]
fn bind_skinned_node(
    world: &mut World,
    skin: &SkinInfo,
    node: Entity,
    wrapper: Option<Entity>,
    slot: usize,
    first_person: bool,
    helmet_model: Option<usize>,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let skeleton = skin.bip01_00.ok_or(GameError::unclassified(
        0x0047B090,
        "0047B090: a skin info with no Bip01",
    ))?;
    compact_children(world, node);
    // 0047B101: slot 1 takes the given FaceGen model, else SkinInfo_GetHelmetFaceGenModel's (only 0047BC40's own slot-1 arm
    // passes none, which the background helmet settings skip).
    let helmet_model = if slot == 1 {
        Some(helmet_model.ok_or(GameError::unclassified(
            0x0047B0F4,
            "0047B0F4: a helmet part's FaceGen model (SkinInfo_GetHelmetFaceGenModel)",
        ))?)
    } else {
        None
    };
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
                .is_some_and(|name| oblivion_core::crt::strnicmp(&name, b"UpperBody", 9) == 0)
        {
            detach_child(world, node, child);
            continue;
        }
        if let (Some(model), Some(data), Some(instance)) =
            (helmet_model, geometry.data_b4, geometry.skin_b8)
        {
            fit_skinned_helmet_part(world, skin, child, data, instance, model, services)?;
        }
        apply_skin_texture(world, skin, child, services)?;
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
            match crate::gamebryo::object_net::object_by_name_with_palette(world, skeleton, &name) {
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
                    let message = if oblivion_core::crt::strnicmp(&own, &name, name.len()) == 0 {
                        let parent = world
                            .get::<NiAvParent>(child)
                            .and_then(|parent| parent.0)
                            .and_then(|parent| name_of(world, parent));
                        format!(
                            "Bone '{bone_name}' not found for part '{}->{part}'.\r\nMake sure all the vertices are skinned to a bone in Max.",
                            text(parent)
                        )
                    } else {
                        return Err(GameError::unclassified(
                            0x0047B4C4,
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
            return Err(GameError::unclassified(
                0x0047B53A,
                "0047B53A: attaching a skinned geometry to its wrapper",
            ));
        }
    }
    compact_children(world, node);
    Ok(())
}

// IDA: 0047B25C..0047B404, 0047B090's helmet arm for a skinned geometry with data (+0xB4), a skin instance (+0xB8) and the
// FaceGen model: for an owner (+0x150) whose base (vt+0x170) is an NPC (type 0x23), the data deep-copied (`00700790`) into the
// geometry (vt+0x8C); a skin instance with a partition (+0x0C) takes a deep copy of it (00478300) after 00700900's clone of
// the instance, which with a fresh cloning process is the instance itself (NiSkinInstance_CreateClone 0072BFA0 answers `this`
// when the map holds none of its bones), so 00478350 changes nothing; then 005221C0's coefficients and, with
// bUseFaceGenHeads, `00558840`(model, coefficients, geometry, 1.0, null).
fn fit_skinned_helmet_part(
    world: &mut World,
    skin: &SkinInfo,
    geometry: Entity,
    data: Entity,
    instance: Entity,
    model: usize,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let Some(base) = world
        .get::<TesObjectRefr>(skin.owner_150)
        .and_then(|refr| refr.base_form_1c)
        .filter(|&base| crate::tes::forms_init::form_type(world, base) == 0x23)
    else {
        return Ok(());
    };
    let copy = world.entity_mut(data).clone_and_spawn();
    world
        .get_mut::<NiGeometry>(geometry)
        .expect("a geometry")
        .data_b4 = Some(copy);
    if let Some(partition) = world
        .get::<MeshSkinInstance>(instance)
        .and_then(|instance| instance.partition_0c)
    {
        let partition = world.entity_mut(partition).clone_and_spawn();
        world
            .get_mut::<MeshSkinInstance>(instance)
            .expect("read above")
            .partition_0c = Some(partition);
    }
    let coefficients = crate::tes::face_gen_head::npc_face_gen_coefficients(world, base)?;
    if world
        .get_resource_or_init::<super::character_3d::FaceGenSettings>()
        .use_face_gen_heads_b120b4
    {
        super::helmet_tasks::with_face_gen_files(world, services, |world, files, _| {
            crate::bs::face_gen_model::apply_helmet_coefficients(
                world,
                model,
                &coefficients,
                copy,
                files,
            )
        })?;
    }
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
    helmet_model: Option<usize>,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, GameError> {
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
    let parent = crate::gamebryo::object_net::object_by_name_with_palette(
        world,
        skeleton,
        b"SkinAttachment",
    )
    .or_else(|| {
        world
            .get::<NiAvParent>(skeleton)
            .and_then(|parent| parent.0)
    })
    .ok_or(GameError::unclassified(
        0x0047B6E3,
        "0047B6E3: a skeleton with no parent faults",
    ))?;
    if attach_scene_child(world, parent, Some(wrapper), true) != AttachChildStatus::Attached {
        return Err(GameError::unclassified(
            0x0047B6EE,
            "0047B6EE: attaching the skin's wrapper",
        ));
    }
    if lod {
        for child in child_slots(world, part).into_iter().flatten() {
            bind_skinned_node(
                world,
                &skin,
                child,
                None,
                slot,
                first_person,
                helmet_model,
                services,
            )?;
        }
    } else {
        bind_skinned_node(
            world,
            &skin,
            part,
            Some(wrapper),
            slot,
            first_person,
            helmet_model,
            services,
        )?;
    }
    Ok(Some(wrapper))
}

// IDA: 00478EC0 NiNode_RebindBodyPartSkin, the ordinary NiNode arm with no destination wrapper.
// Completion: compaction, skeleton removal and in-place bone replacement in child order.
// TODO: [ ] NiBSLODNode recursion when its stream class can be loaded.
fn rebind_creature_body_skin(
    world: &mut World,
    root: Entity,
    part: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let text = |world: &World, node| {
        name_of(world, node).map_or_else(
            || "NULL".to_owned(),
            |s| String::from_utf8_lossy(&s).into_owned(),
        )
    };
    compact_children(world, part);
    let mut warned = false;
    for child in child_slots(world, part).into_iter().flatten() {
        let Some(geometry) = world.get::<NiGeometry>(child) else {
            if !warned
                && world
                    .get::<NiNodeChildren>(child)
                    .is_some_and(|n| n.slots_ac.iter().any(Option::is_some))
            {
                let parent = world
                    .get::<NiAvParent>(part)
                    .and_then(|p| p.0)
                    .expect("attached body model");
                warn(
                    services,
                    format!(
                        "Body part '{}->{}' for skeleton '{}' was exported incorectly.\r\nHide the skeleton before you export body parts.",
                        text(world, parent),
                        text(world, part),
                        text(world, root)
                    ),
                );
                warned = true;
            }
            detach_child(world, part, child);
            continue;
        };
        let Some(skin) = geometry.skin_b8 else {
            continue;
        };
        let instance = world
            .get::<MeshSkinInstance>(skin)
            .expect("geometry skin instance");
        let count = instance
            .data_08
            .and_then(|d| world.get::<crate::gamebryo::mesh_skin_owners::MeshSkinData>(d))
            .ok_or(GameError::unclassified(
                0x00478F77,
                "missing creature skin data",
            ))?
            .bones
            .len();
        let bones = instance.bone_owners_14.clone();
        let bones = bones.get(..count).ok_or(GameError::unclassified(
            0x00478F77,
            "creature skin bone array is truncated",
        ))?;
        for (index, bone) in bones.iter().enumerate() {
            let Some(name) = bone.and_then(|bone| name_of(world, bone)) else {
                continue;
            };
            if let Some(found) =
                crate::gamebryo::object_net::object_by_name_with_palette(world, root, &name)
            {
                world
                    .get_mut::<MeshSkinInstance>(skin)
                    .unwrap()
                    .bone_owners_14[index] = Some(found);
            } else {
                let bone_name = String::from_utf8_lossy(&name);
                let child_name = name_of(world, child).unwrap_or_default();
                let parent_name = |world: &World, node| {
                    world
                        .get::<NiAvParent>(node)
                        .and_then(|p| p.0)
                        .map_or_else(|| "NULL".to_owned(), |p| text(world, p))
                };
                let message = if oblivion_core::crt::strnicmp(&child_name, &name, name.len()) == 0 {
                    format!(
                        "Bone '{bone_name}' not found for part '{}->{}'.\r\nMake sure all the verticies are skinned to a bone in Max.",
                        parent_name(world, part),
                        text(world, part)
                    )
                } else {
                    format!(
                        "Bone '{bone_name}' not found for part '{}'.\r\nRequested by model '{}'.",
                        text(world, part),
                        parent_name(world, root)
                    )
                };
                warn(services, message);
            }
        }
    }
    compact_children(world, part);
    Ok(())
}

// IDA: 004E172F..004E1831 TESObjectREFR_UpdateBodyModels, one TESCreature model-list entry.
// Completion: load, clone, attach at SkinAttachment or Bip01's parent, or attach a rigid part.
// 004430C0 TES_CloneModelWithPrivateGeometryData clones first, then copies each geometry data object.
pub(in crate::tes) fn attach_creature_body_model(
    world: &mut World,
    reference: Entity,
    root: Entity,
    path: &[u8],
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let Some(model) = load_model_now(world, path, false, 3, true, services)? else {
        return Ok(());
    };
    let scale = super::actor_animation::actor_scale(world, reference)?;
    let mut process = CloningProcess {
        scale_10: [scale; 3],
        ..Default::default()
    };
    let morpher = crate::gamebryo::av_object::has_geom_morpher(world, model);
    let part = clone_object(world, &mut process, model, &mut services.prepare)?;
    if morpher {
        deep_copy_geometry_data(world, part);
    }
    let skinned = has_skin(world, part);
    let bone = crate::tes::reference_3d::find_bone(world, reference, root, b"Bip01");
    if let Some(bone) = bone.filter(|_| skinned) {
        let parent = crate::bs::model_loader::object_by_name(world, bone, b"SkinAttachment")
            .or_else(|| world.get::<NiAvParent>(bone).and_then(|p| p.0))
            .ok_or(GameError::unclassified(
                0x004E17FD,
                "creature skeleton bone has no parent",
            ))?;
        if attach_scene_child(world, parent, Some(part), true) != AttachChildStatus::Attached {
            return Err(GameError::unclassified(
                0x004E1807,
                "attach the creature body part",
            ));
        }
        rebind_creature_body_skin(world, root, part, services)
    } else {
        attach_rigid_part(world, root, part, usize::MAX, reference, services)
    }
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
) -> Result<(), GameError> {
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
            let bone = crate::gamebryo::object_net::object_by_name_with_palette(
                world,
                skeleton,
                &parent_name,
            );
            if slot == 7 && parent_name.len() > 6 {
                parent_name[6] = b'R';
            }
            match bone {
                Some(bone) => {
                    if attach_scene_child(world, bone, Some(part), true)
                        != AttachChildStatus::Attached
                    {
                        return Err(GameError::unclassified(
                            0x004791C6,
                            "004791C6: attaching a part to its bone",
                        ));
                    }
                    if let Some(scabbard) = crate::gamebryo::object_net::object_by_name_with_palette(
                        world, part, b"Scb",
                    ) {
                        if attach_scene_child(world, bone, Some(scabbard), true)
                            != AttachChildStatus::Attached
                        {
                            return Err(GameError::unclassified(
                                0x004791E8,
                                "004791E8: attaching a scabbard to its bone",
                            ));
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
                        .is_some_and(|base| crate::tes::forms_init::form_type(world, base) == 0x24);
                    if creature {
                        crate::gamebryo::scene_node::detach_scabbard(world, part);
                        release_collision_objects(world, part);
                        return Ok(());
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
) -> Result<(), GameError> {
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
    let drawn = middle.weapon_drawn_115 != 0;
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
        let kind = crate::tes::forms_items::weapon_type_of(world, entry.form_08);
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
                    return Err(GameError::unclassified(
                        0x006563F0,
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
                return Err(GameError::unclassified(
                    0x006562FF,
                    "006562FF: a bow's arrow test (00478180)",
                ));
            }
            let moved = match first(world, from) {
                Some(child) => {
                    if attach_scene_child(world, to, Some(child), true)
                        != AttachChildStatus::Attached
                    {
                        return Err(GameError::unclassified(
                            0x00656330,
                            "00656330: moving the weapon between bones",
                        ));
                    }
                    Some(child)
                }
                None => first(world, to),
            };
            if let Some(child) = moved {
                let mut rotation = IDENTITY;
                let mut translate = [0.0; 3];
                if !drawn && kind == 4 {
                    rotation = crate::gamebryo::transform::rotation_x(std::f32::consts::PI);
                } else if !drawn && kind == 5 {
                    rotation = crate::gamebryo::transform::rotation_x(0.116_465_814);
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
fn finish_weapon_placement(world: &mut World, actor: Entity, drawn: bool) -> Result<(), GameError> {
    let npc = world
        .get::<TesObjectRefr>(actor)
        .and_then(|refr| refr.base_form_1c)
        .is_some_and(|base| crate::tes::forms_init::form_type(world, base) == 0x23);
    // 004D8370 TESObjectREFR_GetAnimData: a MiddleHigh process's +0x17C (the NPCs here have one).
    let anim = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
        .and_then(|process| world.get::<super::actor_process::MiddleHighProcess>(process))
        .and_then(|middle| middle.anim_data_17c);
    let race_menu = world
        .get_resource::<crate::tes::interface_manager::InterfaceMovieReads>()
        .is_some_and(|reads| reads.tile_menu_present(1036));
    if npc
        && let Some(anim) = anim
        && !race_menu
    {
        if drawn {
            return Err(GameError::unclassified(
                0x00656436,
                "00656436: 00476410 for a drawn weapon",
            ));
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
            return Err(GameError::unclassified(
                0x004719C4,
                "004719C4: removing the power-attack groups (00471990)",
            ));
        }
    }
    if drawn {
        return Err(GameError::unclassified(
            0x005EF9D8,
            "005EF9D8: a drawn weapon's reach collision (0053A130)",
        ));
    }
    if let Some(node) = world
        .get::<crate::tes::reference_3d::Reference3D>(actor)
        .map(|loaded| loaded.0)
        && let Some(weapon) =
            crate::gamebryo::object_net::object_by_name_with_palette(world, node, b"Weapon")
    {
        clear_collision(world, weapon);
    }
    Ok(())
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
) -> Result<[f32; 9], GameError> {
    let Some(base) = world
        .get::<TesObjectRefr>(reference)
        .and_then(|refr| refr.base_form_1c)
    else {
        return Ok(*rotation);
    };
    if crate::tes::forms_init::form_type(world, base) != 0x23 {
        return Ok(*rotation);
    }
    let Some(race) = crate::tes::forms_actors::base_race(world, base)? else {
        return Ok(*rotation);
    };
    let (weight, height) = match crate::tes::forms_actors::npc_sex(world, base) {
        Some(sex) if sex <= 1 => (
            crate::tes::forms_actors::race_scale(world, race, 0x68 + 4 * sex),
            crate::tes::forms_actors::race_scale(world, race, 0x60 + 4 * sex),
        ),
        _ => (0.0, 0.0),
    };
    let (across, up) = if inverse {
        (1.0 / weight, 1.0 / height)
    } else {
        (weight, height)
    };
    // 007100A0 with the diagonal matrix.
    let scale = [across, 0.0, 0.0, 0.0, across, 0.0, 0.0, 0.0, up];
    Ok(crate::gamebryo::transform::multiply(rotation, &scale))
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

/// IDA: 00B33D80, set around an equipment rebuild (`005C2F20`'s head rebuild, a vampire's 0063CDC0): 0047BC40 then evicts
/// the owner's body textures from the palette (`00550240`, counted) and re-applies the race's skin texture to the parts it
/// keeps; 00651DD0 skips its weapon-node resets. `palette_evictions_550240` counts 0047BC40's `00550240` calls.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EquipmentRebuildFlag {
    pub active_b33d80: bool,
    pub palette_evictions_550240: u32,
}

/// IDA: 0047BC40 SkinInfo_AttachBipedModels(skin), with 00B33D80 (`EquipmentRebuildFlag`) as it stands. With it set,
/// `00550240` on the owner's base form ID first (counted: a texture palette eviction). Each
/// of the sixteen slots: one whose model equals the 00B33C80 stash's takes the stash's four words back (the stash zeroed) and
/// keeps them, a ring or amulet hidden (00478290) releasing it; otherwise a stashed node is released (a stop here). Then a slot
/// with a model, not hidden: slot 1 with the background helmet settings and no node goes to SkinInfo_QueueHelmet (`helmet_tasks`); slot
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
) -> Result<(), GameError> {
    let owner = world
        .get::<SkinInfo>(skin_entity)
        .expect("a skin info")
        .owner_150;
    // 0047BC56 PlayerCharacter_IsFirstPersonSkinInfo: the player's +0x5C8.
    let first_person = Some(owner) == crate::tes::player::the_player(world)
        && world
            .get::<super::player::PlayerCharacter>(owner)
            .is_some_and(|player| player.first_person_skin_info_5c8 == Some(skin_entity));
    // 0047BC74: with 00B33D80, 00550240(the owner's base form ID).
    let rebuilding = world
        .get_resource::<EquipmentRebuildFlag>()
        .is_some_and(|flag| flag.active_b33d80);
    if rebuilding {
        world
            .resource_mut::<EquipmentRebuildFlag>()
            .palette_evictions_550240 += 1;
    }
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
            // 0047BD45: a hidden ring's kept slot is released in place (0047BD93: 00478780(slot, 0, 0)).
            if (6..=8).contains(&slot) && slot_hidden(world, &skin, slot) {
                release_skin_slot(world, skin_entity, SkinSlotAt::Skin(slot), false, None)?;
                continue;
            }
            // 0047BD2A: with 00B33D80, a kept NiNode part's children take the race's skin texture again (0047AC20 each).
            if rebuilding
                && let Some(node) = stash.node_08
                && world.get::<NiNodeChildren>(node).is_some()
            {
                let children: Vec<Entity> = world
                    .get::<NiNodeChildren>(node)
                    .map(|children| children.slots_ac.iter().flatten().copied().collect())
                    .unwrap_or_default();
                let skin = world
                    .get::<SkinInfo>(skin_entity)
                    .expect("a skin info")
                    .clone();
                for child in children {
                    apply_skin_texture(world, &skin, child, services)?;
                }
            }
            // 0047BDC3: a kept skinned part takes the race-scaled identity (004D7C50 with the inverse).
            if stash.skinned_0c != 0
                && let Some(node) = stash.node_08
            {
                let scaled = race_scaled_rotation(world, owner, &IDENTITY, true)?;
                if let Some(mut payload) = world.get_mut::<SceneNodePayload>(node) {
                    payload.local_transform.rotation = scaled;
                }
            }
            let empty = (6..=8).contains(&slot) && stash.node_08.is_none();
            if !empty {
                continue;
            }
        } else if stash.node_08.is_some() {
            // 0047BE24: a stashed node the slot no longer wants goes (0047BE43: 00478780(stash, 1, 0)).
            release_skin_slot(world, skin_entity, SkinSlotAt::Stash(slot), true, None)?;
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
                // 0047BE40 SkinInfo_QueueHelmet(skin, out, 0, null), the local out dropped (004BDDC0). An actor task's helmet
                // (004788E0) is still in the loader's helmet map until its finish (00437B60), so this one is freed at once.
                let helmet = super::helmet_tasks::create_queued_helmet(
                    world,
                    skin_entity,
                    0,
                    None,
                    services.archives.as_deref_mut(),
                )?;
                super::helmet_tasks::release_helmet_reference(world, helmet)?;
                continue;
            }
        }
        if slot == 14 {
            return Err(GameError::unclassified(
                0x0047BE6E,
                "0047BE6E: the torch slot's process vt+0x348",
            ));
        }
        let path = skin_model_path(world, entry.model_04);
        let Some(model) = load_model_now(world, &path, true, 3, true, services)? else {
            continue;
        };
        if crate::gamebryo::av_object::has_geom_morpher(world, model) {
            return Err(GameError::unclassified(
                0x0047BEEB,
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
        let mut placed = match attach_skinned_part(
            world,
            skin_entity,
            part,
            slot,
            first_person,
            None,
            services,
        )? {
            Some(wrapper) => wrapper,
            None => {
                let skeleton = skeleton.ok_or(GameError::unclassified(
                    0x0047C1D8,
                    "0047C1D8: a rigid part with no skeleton",
                ))?;
                attach_rigid_part(world, skeleton, part, slot, owner, services)?;
                if slot == 9
                    && !crate::tes::forms_references::save_load_active(world)?
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
            .and_then(|form| world.get::<crate::tes::form::FormIdentity>(form))
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
        if crate::tes::reference_3d::bsx_flag(world, model, 0x10) {
            return Err(GameError::unclassified(
                0x00478238,
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
                return Err(GameError::unclassified(
                    0x0047C4F8,
                    "0047C4F8: attaching a part under its bone",
                ));
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
        .get::<crate::tes::reference_3d::Reference3D>(owner)
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

/// IDA: 004792F0 SkinInfo_CloneHelmet(skin, FaceGen model, root): null when slot 1 already has a node (+0x64), or for a null
/// FaceGen model or root; otherwise the root cloned with a fresh cloning process at scale 1 (a NiGeomMorpherController's
/// 004430C0 is a stop), a BSFadeNode clone at stage 7, its "REF" extra data removed, and a zero translate with the identity.
/// Completion: 100% but the morpher arm.
/// # Errors
/// The morpher arm, or a stop in the clone.
pub fn clone_helmet(
    world: &mut World,
    skin_entity: Entity,
    face_gen_model: Option<usize>,
    root: Option<Entity>,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, GameError> {
    let skin = world.get::<SkinInfo>(skin_entity).expect("a skin info");
    if skin.slots_4c[1].node_08.is_some() {
        return Ok(None);
    }
    let (Some(_), Some(root)) = (face_gen_model, root) else {
        return Ok(None);
    };
    if crate::gamebryo::av_object::has_geom_morpher(world, root) {
        return Err(GameError::unclassified(
            0x00479375,
            "00479375: a helmet's NiGeomMorpherController clone (004430C0)",
        ));
    }
    let mut process = CloningProcess {
        scale_10: [1.0; 3],
        ..CloningProcess::default()
    };
    let clone = clone_object(world, &mut process, root, &mut services.prepare)?;
    if let Some(mut fade) = world.get_mut::<crate::bs::fade_node::BsFadeNode>(clone) {
        crate::bs::fade_node::fade_node_set_stage(&mut fade, 7);
    }
    remove_extra_data(world, clone, b"REF");
    if let Some(mut payload) = world.get_mut::<SceneNodePayload>(clone) {
        payload.local_transform.translation = [0.0; 3];
        payload.local_transform.rotation = IDENTITY;
    }
    Ok(Some(clone))
}

/// IDA: 0047B7D0 SkinInfo_AttachHelmet(skin, FaceGen model, part) for a third-person skin, from QueuedHelmet_Finish
/// (`00437B60`): with both, `attach_skinned_part` (0047B5B0, slot 1, the FaceGen model) or else `attach_rigid_part`
/// (00479140, slot 1); the part named "%s %s (%08X)" with 00B0658C ("Hair"), slot 1's form's vt+0xD4 ("") and its ID; a
/// skinned slot's (+0x68) rotation through 004D7C50; a BSX flag 0x10 part's flame nodes (00478220, a stop); for an unskinned
/// NiNode part (vt+0x08), the bone entry 00B065CC (0, the head) takes it through vt+0x84(part, 1) when it has a node, else
/// Bip01 (+0x00) does when the part has a parent (+0x1C); slot 1's +0x08 takes the part. Last, the owner's node's
/// (+0x150's +0x3C) property and effect state. Completion: 100% of a third-person skin's rigid helmet; a skinned helmet's
/// FaceGen fit in `bind_skinned_node` is a stop.
/// # Errors
/// The stops above, or a failed attach.
pub fn attach_helmet(
    world: &mut World,
    skin_entity: Entity,
    face_gen_model: Option<usize>,
    part: Option<Entity>,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    const SLOT: usize = 1;
    let owner = world
        .get::<SkinInfo>(skin_entity)
        .expect("a skin info")
        .owner_150;
    if let (Some(_), Some(part)) = (face_gen_model, part) {
        // 0047B82E PlayerCharacter_IsFirstPersonSkinInfo, only for the player.
        let first_person = Some(owner) == crate::tes::player::the_player(world)
            && world
                .get::<super::player::PlayerCharacter>(owner)
                .is_some_and(|player| player.first_person_skin_info_5c8 == Some(skin_entity));
        let placed = match attach_skinned_part(
            world,
            skin_entity,
            part,
            SLOT,
            first_person,
            face_gen_model,
            services,
        )? {
            Some(wrapper) => wrapper,
            None => {
                let skeleton = world
                    .get::<SkinInfo>(skin_entity)
                    .expect("a skin info")
                    .bip01_00
                    .ok_or(GameError::unclassified(
                        0x0047B860,
                        "0047B860: a rigid helmet with no skeleton",
                    ))?;
                attach_rigid_part(world, skeleton, part, SLOT, owner, services)?;
                part
            }
        };
        let skin = world
            .get::<SkinInfo>(skin_entity)
            .expect("a skin info")
            .clone();
        let form_id = skin.slots_4c[SLOT]
            .form_00
            .and_then(|form| world.get::<crate::tes::form::FormIdentity>(form))
            .map_or(0, |identity| identity.form_id_0c);
        let name = format!("{}  ({form_id:08X})", SLOT_NAMES_B06588[SLOT]);
        if let Some(mut net) = world.get_mut::<NiObjectNet>(placed) {
            net.name = Some(name.into_bytes());
        }
        let skinned = skin.slots_4c[SLOT].skinned_0c != 0;
        if skinned {
            let rotation = world
                .get::<SceneNodePayload>(placed)
                .map_or(IDENTITY, |payload| payload.local_transform.rotation);
            let scaled = race_scaled_rotation(world, owner, &rotation, true)?;
            if let Some(mut payload) = world.get_mut::<SceneNodePayload>(placed) {
                payload.local_transform.rotation = scaled;
            }
        }
        if crate::tes::reference_3d::bsx_flag(world, part, 0x10) {
            return Err(GameError::unclassified(
                0x0047B909,
                "0047B909: a helmet's flame nodes (NiNode_AttachFlameNodes)",
            ));
        }
        if !skinned && world.get::<NiNodeChildren>(placed).is_some() {
            let bone = SLOT_BONE_B065C8[SLOT] as usize;
            let target = match skin.bones_04[bone].node_04 {
                Some(node) => Some(node),
                None => world
                    .get::<NiAvParent>(placed)
                    .and_then(|parent| parent.0)
                    .and(skin.bip01_00),
            };
            if let Some(target) = target
                && attach_scene_child(world, target, Some(placed), true)
                    != AttachChildStatus::Attached
            {
                return Err(GameError::unclassified(
                    0x0047B949,
                    "0047B949: attaching a helmet under its bone",
                ));
            }
        }
        world
            .get_mut::<SkinInfo>(skin_entity)
            .expect("a skin info")
            .slots_4c[SLOT]
            .node_08 = Some(placed);
    }
    if let Some(node) = world
        .get::<crate::tes::reference_3d::Reference3D>(owner)
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

/// IDA: 00479780 SkinInfo_AttachWeapon(skin, weapon) for a non-player's skin: only a weapon (type 0x21). Slot 9's entry is
/// released (00478780(slot, 1, 0)) and takes the weapon (+0xDC) and its TESModel (+0xE0, the weapon's +0x30); a bow's part
/// slot is 14. With a model path and an owner whose 3D is loaded: ModelLoader_LoadModelNow(path, 1, 3, 1), cloned at scale
/// 1 (a NiGeomMorpherController's 004430C0 is a stop); 00478220 (a BSX flag 0x10 model's flame nodes, a stop);
/// NiObjectNET_RemoveAllExtraData on the clone; a BSFadeNode clone at stage 7; a zero translate and the identity. A clone
/// with skinned geometry (00471B80) only warns "Tyring to add skinned object '%s' as an add on to skeleton." Otherwise
/// 00479140(skeleton, part, model, null, -1), which finds no "Prn" now; an unparented node goes under its slot's bone
/// (00B065C8, by name from the skeleton, 006F94A0), else the skeleton (vt+0x84(part, 1)). Its effect and property state
/// refresh. +0xE4 takes the part (null with no path or no 3D), named "Weapon %s (%08X)" (the form's vt+0xD4 is "" here).
/// Last, with the owner's process, vt+0x150 (00656270) as 0047BC40 runs it. Completion: a non-player's melee weapon.
/// # Errors
/// The stops above, or one in the load, clone or placement.
pub fn attach_weapon_part(
    world: &mut World,
    skin_entity: Entity,
    weapon: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    if crate::tes::forms_init::form_type(world, weapon) != 0x21 {
        return Ok(());
    }
    release_skin_slot(world, skin_entity, SkinSlotAt::Skin(9), true, None)?;
    {
        let mut skin = world.get_mut::<SkinInfo>(skin_entity).expect("a skin info");
        skin.slots_4c[9].form_00 = Some(weapon);
        skin.slots_4c[9].model_04 = Some(super::character_3d::SkinModel::Form(weapon));
    }
    let slot = if crate::tes::forms_items::weapon_type_of(world, weapon) == 5 {
        14
    } else {
        9
    };
    let owner = world
        .get::<SkinInfo>(skin_entity)
        .expect("a skin info")
        .owner_150;
    if Some(owner) == crate::tes::player::the_player(world) {
        return Err(GameError::stop(0x004797C8, "the player's weapon part"));
    }
    let path = skin_model_path(world, Some(super::character_3d::SkinModel::Form(weapon)));
    let skeleton = world
        .get::<crate::tes::reference_3d::Reference3D>(owner)
        .map(|loaded| loaded.0);
    let mut placed = None;
    if !path.is_empty()
        && let Some(skeleton) = skeleton
        && let Some(model) = load_model_now(world, &path, true, 3, true, services)?
    {
        if crate::gamebryo::av_object::has_geom_morpher(world, model) {
            return Err(GameError::stop(
                0x00479875,
                "a weapon part's NiGeomMorpherController clone (004430C0)",
            ));
        }
        let mut process = CloningProcess {
            scale_10: [1.0; 3],
            ..CloningProcess::default()
        };
        let part = clone_object(world, &mut process, model, &mut services.prepare)?;
        if crate::tes::reference_3d::bsx_flag(world, model, 0x10) {
            return Err(GameError::stop(
                0x00478238,
                "a weapon part's flame nodes (NiNode_AttachFlameNodes)",
            ));
        }
        // 006FFC60 NiObjectNET_RemoveAllExtraData.
        if let Some(mut net) = world.get_mut::<NiObjectNet>(part) {
            net.extra_data.clear();
        }
        if let Some(mut fade) = world.get_mut::<crate::bs::fade_node::BsFadeNode>(part) {
            crate::bs::fade_node::fade_node_set_stage(&mut fade, 7);
        }
        if let Some(mut payload) = world.get_mut::<SceneNodePayload>(part) {
            payload.local_transform.translation = [0.0; 3];
            payload.local_transform.rotation = IDENTITY;
        }
        if has_skin(world, part) {
            let name = name_of(world, part).unwrap_or_default();
            warn(
                services,
                format!(
                    "Tyring to add skinned object '{}' as an add on to skeleton.",
                    String::from_utf8_lossy(&name)
                ),
            );
        } else {
            // 00479140 with slot -1: no slot is the left ring's.
            attach_rigid_part(world, skeleton, part, usize::MAX, owner, services)?;
            if world.get::<NiNodeChildren>(part).is_some()
                && world
                    .get::<NiAvParent>(part)
                    .and_then(|parent| parent.0)
                    .is_none()
            {
                let bone = SLOT_BONE_B065C8[slot];
                let target = (bone != -1)
                    .then(|| {
                        crate::gamebryo::object_net::object_by_name_with_palette(
                            world,
                            skeleton,
                            super::character_3d::SKIN_BONES_B06550[bone as usize],
                        )
                    })
                    .flatten()
                    .unwrap_or(skeleton);
                if attach_scene_child(world, target, Some(part), true)
                    != AttachChildStatus::Attached
                {
                    return Err(GameError::stop(0x00479A09, "attaching a weapon part"));
                }
            }
        }
        crate::gamebryo::scene_state::refresh_effect_state(world, part)?;
        crate::gamebryo::scene_state::refresh_property_state(
            world,
            part,
            &services.prepare.property_defaults,
        )?;
        placed = Some(part);
    }
    world
        .get_mut::<SkinInfo>(skin_entity)
        .expect("a skin info")
        .slots_4c[9]
        .node_08 = placed;
    let form_id = world
        .get::<crate::tes::form::FormIdentity>(weapon)
        .map_or(0, |identity| identity.form_id_0c);
    if let Some(part) = placed
        && let Some(mut net) = world.get_mut::<NiObjectNet>(part)
    {
        net.name = Some(format!("{}  ({form_id:08X})", SLOT_NAMES_B06588[9]).into_bytes());
    }
    if let Some(process) = world
        .get::<MobileObject>(owner)
        .and_then(|mobile| mobile.process_58)
    {
        place_weapon(world, process, skin_entity, owner)?;
    }
    Ok(())
}
