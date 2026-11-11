//! A reference's 3D: the base's vt+0x114 (`004B2320` → `004B3750`), which takes the Model's root or clones it.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::bs::havok::HavokGlobals;
use crate::bs::model_loader::ModelBuildServices;
use crate::bs::model_loader::fixup_model_path;
use crate::bs::ragdoll::apply_node_poses;
use crate::bs::task_manager::TaskGlobals;
use crate::gamebryo::av_object::{NiAvParent, has_geom_morpher, set_controller_phases};
use crate::gamebryo::clone::{CloningProcess, clone_object};
use crate::gamebryo::controller_sequence::{activate_from_start, deactivate_all};
use crate::gamebryo::object_net::{NiExtraDataName, NiObjectNet};
use crate::gamebryo::scene_node::{SceneNodePayload, detach_scabbard};
use crate::gamebryo::transform::NiTransform;
use crate::tes::actor::{Character, TesObjectRefr};
use crate::tes::actor_animation::destroy_actor_anim_data;
use crate::tes::cell_havok::{cell_bhk_world, tree_has_moving_body};
use crate::tes::character_3d::{SkinInfo, SkinSlotAt, npc_create_3d, release_skin_slot};
use crate::tes::default_forms::DefaultFormStatics;
use crate::tes::form::FormIdentity;
use crate::tes::forms_init::{form_id, form_type};
use crate::tes::forms_world::{TesObjectCell, cell_is_interior};
use crate::tes::player::player_check_reference_overlap;
use crate::tes::player_reload::{player_remove_from_world, release_weapon};
use crate::tes::reference_load::model_path;
use crate::tes::save_load_game::change_flags;
use crate::tes::tile::InterfaceCx;
use oblivion_core::crt::stricmp;

/// IDA: TESObjectREFR +0x3C, the reference's NiNode (`TesObjectRefr::loaded_3d_3c` holds 1 while it is set).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference3D(pub Entity);

// IDA: 004D96F0 TESObjectREFR_FindBone. A horse goes directly to the alternate
// bone name. The other references try the original name first, using its palette.
// Completion: the two lookups for the animation and body-part bone names.
pub(in crate::tes) fn find_bone(
    world: &World,
    reference: Entity,
    root: Entity,
    name: &[u8],
) -> Option<Entity> {
    use crate::gamebryo::object_net::object_by_name_with_palette;
    let horse = world
        .get::<TesObjectRefr>(reference)
        .and_then(|r| r.base_form_1c)
        .and_then(|base| world.get::<crate::tes::forms_actors::TesCreature>(base))
        .is_some_and(|creature| creature.data_104[0] == 4);
    if !horse && let Some(bone) = object_by_name_with_palette(world, root, name) {
        return Some(bone);
    }
    let mut alternate = name.to_vec();
    if let Some(fifth) = alternate.get_mut(4) {
        *fifth = b'2';
    }
    object_by_name_with_palette(world, root, &alternate)
}

/// IDA: the scaled-model cache 00434820 looks in (a string map of `004B2B00`'s keys to ModelMapEntry, `00434A70` holding the
/// node with a reference).
#[derive(Resource, Clone, Debug, Default)]
pub struct ScaledModelCache {
    pub entries: std::collections::BTreeMap<Vec<u8>, Entity>,
}

/// IDA: 004B2B00(ref, buffer): `sprintf("%s%i", path, n)` with the path as 004B3310 takes it and n = `fistp` (round to
/// nearest) of the float GetScale(ref) * 100.0 (`dbl_A309F0`). Completion: 100%.
pub fn scaled_model_key(
    world: &World,
    base: Entity,
    reference: Entity,
) -> Result<Vec<u8>, GameError> {
    let mut key = crate::tes::reference_load::model_for_reference(world, base, reference)?
        .unwrap_or_default();
    let scale = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .scale_38;
    let product = (f64::from(scale) * 100.0) as f32;
    key.extend_from_slice(format!("{}", f64::from(product).round_ties_even() as i32).as_bytes());
    Ok(key)
}

/// IDA: 004B3310, the bound objects' vt+0x110: the model path (00435580's biped arm or the TESModel); a reference with a scale
/// other than 1 first asks the scaled-model cache (`004B2B00`, `00434820`: a hit answers the entry's node, with the entry's
/// count up and `00B35464[409]` counted); otherwise 00439EB0's model map entry, the Model's +0x08 root. A model not in the
/// map is loaded at once by 00439EB0 (`load_model_now`). Completion: 100%.
pub fn model_root_for(
    world: &mut World,
    base: Entity,
    reference: Option<Entity>,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, GameError> {
    if let Some(reference) = reference
        && world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .scale_38
            != 1.0
    {
        let key = scaled_model_key(world, base, reference)?;
        if let Some(&node) = world
            .get_resource::<ScaledModelCache>()
            .and_then(|cache| cache.entries.get(&key))
        {
            return Ok(Some(node));
        }
    }
    let path = match reference {
        Some(reference) => crate::tes::reference_load::model_for_reference(world, base, reference)?,
        None => crate::tes::reference_load::model_path_of(world, base),
    };
    let Some(path) = path.filter(|path| !path.is_empty()) else {
        return Ok(None);
    };
    let path = fixup_model_path(&path);
    // 004B334B..004B3369: with a reference, its +0x1C base's vt+0xF4 (004B2A00 TESBoundObject_IsMarker on every bound class)
    // is a3; 004B33C5 TESForm_GetIOStage the a4; a5 = 1.
    let marker = reference
        .and_then(|reference| {
            world
                .get::<TesObjectRefr>(reference)
                .expect("a reference")
                .base_form_1c
        })
        .is_some_and(|base| bound_object_is_marker(world, base));
    let stage = crate::tes::form::model_stage(form_type(world, base));
    crate::bs::model_loader::load_model_now(world, &path, marker, stage, true, services)
}

// IDA: N/A the root's references: the Model's +0x08 and each reference whose +0x3C holds it.
fn root_reference_count(world: &mut World, root: Entity) -> usize {
    let mut query = world.query::<&Reference3D>();
    1 + query.iter(world).filter(|node| node.0 == root).count()
}

// IDA: NiObjectNET_GetExtraData(root, "BSX") (00A7D0EC) & 0x10, the BSXFlags word at +0x0C.
pub(crate) fn bsx_flag(world: &World, root: Entity, bit: u32) -> bool {
    world
        .get::<NiObjectNet>(root)
        .map(|net| net.extra_data.as_slice())
        .unwrap_or_default()
        .iter()
        .find(|&&extra| {
            world
                .get::<NiExtraDataName>(extra)
                .is_some_and(|name| name.0.as_deref() == Some(b"BSX".as_slice()))
        })
        .and_then(|&extra| world.get::<crate::bs::nif_objects::BsxFlagsExtraData>(extra))
        .is_some_and(|flags| flags.flags_0c & bit != 0)
}

/// IDA: 004B2320 (vt+0x114 of STAT, ACTI, MISC, FURN, WEAP, ARMO, CLOT, TREE and the item types) → vt+0xEC 004B3750(ref, 0):
/// door and travel markers take no reference; the root by vt+0x110; the base's +0x08 bit 0x10 sets the reference's
/// (0046A9C0). With the root's refcount 1 (the Model's alone), the type's rule: ACTI 0x12 and creature 0x24 always clone
/// (`00480820`'s answer is the clone's a3, a stop when true), ARMO 0x14 clones with biped slot 13 (`00468FF0`'s +0x04 word), DOOR 0x18 clones
/// with a NiControllerManager and a sequence on its first child, 0x21..0x23 always clone; the others clone with BSX bit 0x10
/// or a reference scale other than the root's +0x60. Not cloning, the reference **takes the root itself**. Then
/// (`004B3BEC`, both ways) +0x18 bit 0 clears, |1.0| scale, identity rotation, zero translation, parent null. A clone takes
/// 00478B90's process at the reference's scale and 00700610, and a scaled clone of a root at scale 1 goes into the scaled-model
/// cache. Completion: 100% for the arms above.
pub fn bound_object_create_3d(
    world: &mut World,
    base: Entity,
    reference: Option<Entity>,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, GameError> {
    let statics = world.resource::<DefaultFormStatics>();
    let marker =
        Some(base) == statics.door_marker_b35ea4 || Some(base) == statics.travel_marker_b35eb4;
    let reference = if marker { None } else { reference };
    let Some(root) = model_root_for(world, base, reference, services)? else {
        return Ok(None);
    };
    if world.get::<FormIdentity>(base).expect("a form").flags_08 & 0x10 != 0
        && let Some(reference) = reference
    {
        world
            .get_mut::<FormIdentity>(reference)
            .expect("a form")
            .flags_08 |= 0x10;
    }
    let root_scale = world
        .get::<SceneNodePayload>(root)
        .ok_or(GameError::unclassified(
            0x004B37BB,
            "004B37BB: a model root that isn't a NiNode",
        ))?
        .local_transform
        .scale;
    let reference_scale = reference.map_or(1.0, |reference| {
        world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .scale_38
    });
    let clone = if root_reference_count(world, root) != 1 {
        true
    } else {
        match form_type(world, base) {
            // 004B380B 00480820: a NiGeomMorpherController anywhere under the root makes a3 1, the 004430C0 arm (a stop);
            // otherwise the clone.
            0x12 | 0x24 => {
                if has_geom_morpher(world, root) {
                    return Err(GameError::unclassified(
                        0x004B39E0,
                        "004B39E0: the a3 clone arm (004430C0)",
                    ));
                }
                true
            }
            // 004B38E2 00468FF0(slot 13, 0): +0x04's bit 13 on the armor's TESBipedModelForm (+0x64).
            0x14 if world
                .get::<crate::tes::form_components::TesBipedModelForm>(base)
                .is_some_and(|biped| biped.slot_mask_04 & (1 << 13) != 0) =>
            {
                true
            }
            // 004B3872..004B38B5: the root's first child with a +0x0C controller that is a NiControllerManager (00B3CAC0)
            // holding the sequence "Unequip" clones; otherwise the default rule.
            0x18 => {
                let first = world
                    .get::<crate::gamebryo::scene_node::NiNodeChildren>(root)
                    .and_then(|children| children.slots_ac.first().copied().flatten());
                let manager = first
                    .and_then(|first| world.get::<NiObjectNet>(first))
                    .and_then(|net| net.controller_0c)
                    .is_some_and(|head| {
                        world.get::<crate::gamebryo::stream_objects::NiClass>(head)
                            == Some(&crate::gamebryo::stream_objects::NiClass::ControllerManager)
                    });
                // 004B38A7: the A43C10 sequence name, "Unequip".
                let unequip = first
                    .and_then(|first| world.get::<NiObjectNet>(first))
                    .and_then(|net| net.controller_0c)
                    .filter(|_| manager)
                    .and_then(|head| {
                        world.get::<crate::gamebryo::controller_sequence::ControllerManager>(head)
                    })
                    .and_then(|state| state.find_sequence(b"Unequip"));
                unequip.is_some()
                    || bsx_flag(world, root, 0x10)
                    || (reference.is_some() && reference_scale != root_scale)
            }
            0x21..=0x23 => true,
            _ => {
                bsx_flag(world, root, 0x10)
                    || (reference.is_some() && reference_scale != root_scale)
            }
        }
    };
    let node = if clone {
        // 004B3821..004B3963: the clone's scale is the reference's; a zero scale warns and takes 1.
        let mut scale = reference_scale;
        if scale == root_scale || scale == 0.0 {
            scale = 1.0;
        }
        let mut process = CloningProcess {
            scale_10: [scale; 3],
            ..CloningProcess::default()
        };
        let node = clone_object(world, &mut process, root, &mut services.prepare)?;
        // 004B3A7B..004B3ACF: a scale other than 1 on a root at 1: the clone's +0x60 = |scale|, the cache's entry (00434A70,
        // 00434800, `00B35464[408]` counted) unless the key is there, then vt+0xEC again, whose answer is the 3D (the clone,
        // when it differs, is released through 004B24F0).
        if scale != 1.0 && root_scale == 1.0 {
            world
                .get_mut::<SceneNodePayload>(node)
                .expect("a NiNode")
                .local_transform
                .scale = scale.abs();
            let reference = reference.ok_or(GameError::unclassified(
                0x004B3ABF,
                "004B3ABF: a scaled clone with no reference",
            ))?;
            let key = scaled_model_key(world, base, reference)?;
            world
                .get_resource_or_init::<ScaledModelCache>()
                .entries
                .entry(key)
                .or_insert(node);
            return bound_object_create_3d(world, base, Some(reference), services);
        }
        node
    } else {
        root
    };
    // 004B3BEC..004B3C0D.
    let mut payload = world.get_mut::<SceneNodePayload>(node).expect("a NiNode");
    payload.effective_av_flags &= !1;
    payload.local_transform = NiTransform {
        scale: 1.0_f32.abs(),
        ..NiTransform::default()
    };
    world.entity_mut(node).insert(NiAvParent(None));
    Ok(Some(node))
}

/// IDA: 004D67C0 TESObjectExtraData (0x10, named "REF"): +0x0C the reference whose 3D holds it.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TesObjectExtraData {
    pub reference_0c: Entity,
}

// IDA: 00897A20(node, 1): 004A05E0's bhkNiCollisionObject (+0xA8 with RTTI 00BA7A04 in its chain) takes +0x0C |= 0x40, then
// each child from the last slot down. Completion: 100%.
fn mark_collision_for_sync(world: &mut World, object: Entity) {
    if let Some(collision) = world
        .get::<crate::gamebryo::av_object::NiAvCollision>(object)
        .map(|collision| collision.0)
        && let Some(mut flags) =
            world.get_mut::<crate::bs::havok_stream::BhkSpCollisionObject>(collision)
    {
        flags.flags_0c |= 0x40;
    }
    let children = world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(object)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default();
    for child in children.into_iter().rev().flatten() {
        mark_collision_for_sync(world, child);
    }
}

// IDA: 007B7F00(object, value): a geometry (vt+0x0C) with a type-4 property whose vt+0x54 isn't -1 and a +0xBC shader: the
// shader's vt+0x1C (GetShaderId, its answer dropped), then the property's +0x68 = value; a node (vt+0x08) recurses into each
// nonnull child. Completion: 100% for the bound property classes.
fn set_geometry_classifier(world: &mut World, object: Entity, value: i32) -> Result<(), GameError> {
    if matches!(
        world.get::<crate::gamebryo::stream_objects::NiClass>(object),
        Some(
            crate::gamebryo::stream_objects::NiClass::TriShape
                | crate::gamebryo::stream_objects::NiClass::TriStrips
        )
    ) {
        let property = crate::gamebryo::av_object::find_property_of_type(world, object, 4)?;
        let property = match property {
            Some(property) if crate::bs::sky_shader::shader_property_id(world, property)? != -1 => {
                Some(property)
            }
            _ => None,
        };
        if property.is_some()
            && world
                .get::<crate::bs::sky_shader::NiGeometryShader>(object)
                .is_some()
        {
            crate::bs::sky_shader::set_shader_property_classifier(world, property, value)?;
        }
        return Ok(());
    }
    let children = world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(object)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default();
    for child in children.into_iter().flatten() {
        set_geometry_classifier(world, child, value)?;
    }
    Ok(())
}

/// IDA: 004E4080 TESObjectREFR_Load3D (vt+0x14C) for a reference that isn't an actor: none with +0x08 bits 0x20 or 0x800; a
/// +0x3C answers; otherwise the base's vt+0x114, then `004D7D10` (vt+0x144 `004E48B0`: with no old node and no Havok world it
/// does nothing; the +0x3C store), `004D9800` (an absent node in `00B082F0`'s array: nothing), BSX bit 0x10's FlameNode walks
/// (`004E26F0`; `004DE1C0` for +0x08 bit 0x80, a stop) or `004D6700` (`00B35E50`'s 21 slots, which `TES_PreloadDefaultAssets` leaves zero: false), a weapon's
/// `00480770` (the scabbard detached), +0x54 the position and +0x30 `004D7AF0`, `004D6F20`'s bit 0x100000 on a non-actor (a stop), `00897A20`, the "REF"
/// extra (`004D67C0`, AddExtraDataWithGeneratedName), `007B8910` with `004E0CF0`'s classifier, door and travel markers' +0x18
/// bit 0, a teleport's `004CCA60`, `004E3490` (no first child's controller: the node's vt+0x60 Update(0, 0), else a
/// stop), |scale| at +0x60 and vt+0x50, `NiAVObject_RefreshPropertyState`, `00424E60` (extras 0x4B and 0x4A: a stop).
/// Completion: 100% of a static's path; the arms named are stops.
pub fn reference_load_3d(
    world: &mut World,
    reference: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, GameError> {
    let flags = world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08;
    if flags & 0x20 != 0 || flags & 0x800 != 0 {
        return Ok(None);
    }
    if let Some(node) = world.get::<Reference3D>(reference) {
        return Ok(Some(node.0));
    }
    // PlayerCharacter's vt+0x14C is its own (00667BE0), which calls Character_Load3D (00659F30) directly for the third person.
    if Some(reference) == crate::tes::player::the_player(world) {
        return crate::tes::player_3d::player_load_3d(world, reference, services);
    }
    load_3d_body(world, reference, services)
}

/// IDA: 004E4080's body past the flags and +0x3C tests, and the Character / Creature Load3D (00659F30) it shares: what
/// `reference_load_3d` runs, and 00667BE0 calls for the player's third person.
/// # Errors
/// As `reference_load_3d`.
pub(crate) fn load_3d_body(
    world: &mut World,
    reference: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, GameError> {
    let flags = world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08;
    let Some(base) = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
    else {
        return Ok(None);
    };
    let actor = world.get::<crate::tes::actor::Actor>(reference).is_some();
    let base_type = form_type(world, base);
    if actor && !matches!(base_type, 0x23 | 0x24) {
        return Err(GameError::unclassified(
            0x004E40B4,
            "004E40B4: an actor base that isn't an NPC or a creature",
        ));
    }
    match base_type {
        // 00521A40, TESNPC's vt+0x114 (00A53DD4): the skin info, the take-or-clone, the bones and the refreshes below.
        0x23 => {}
        // 004B2320, checked on each of these vtables (BOOK, FLOR, AMMO, SGST and CONT on their primary vtables 00A44454,
        // 00A41B1C, 00A4068C, 00A45534, 00A44824).
        0x12..=0x17 | 0x19 | 0x1B..=0x22 | 0x26..=0x28 | 0x2A => {}
        // 0051C640 TESCreature_Create3D (TESCreature's 00A5324C +0x114): TESBoundObject_TakeOrCloneModel3D(ref, 0), which always
        // clones a creature.
        0x24 => {}
        // 004B7F70 (TESObjectDOOR's 00A44A54 +0x114): 004B2320, then the door's +0x64 flags below.
        0x18 => {}
        // 004B20D0 (TESObjectLIGH's 00A4319C +0x114).
        0x1A => {}
        _ => {
            return Err(GameError::unclassified(
                0x004E40B4,
                "004E40B4: a base type whose vt+0x114 isn't read",
            ));
        }
    }
    let built = if base_type == 0x1A {
        super::light_3d::light_create_3d(world, base, reference, services)
    } else if base_type == 0x23 {
        npc_create_3d(world, base, reference, services)
    } else {
        bound_object_create_3d(world, base, Some(reference), services)
    };
    let created = match built {
        Ok(created) => created,
        Err(error) if is_clone_gap(&error) => {
            world
                .get_resource_or_init::<Load3DHostGates>()
                .gated
                .push((reference, error.message));
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let Some(node) = created else {
        return Ok(None);
    };
    // 004B7F70: a door with +0x64 bit 2 and the reference's +0x08 bit 0x2000 clear adds the 00B3B900 controller and BSX bit 0;
    // otherwise that controller is removed when there.
    if base_type == 0x18 {
        let automatic = world
            .get::<crate::tes::forms_items::TesDoor>(base)
            .is_some_and(|door| door.flags_64 & 2 != 0);
        if automatic && flags & 0x2000 == 0 {
            return Err(GameError::unclassified(
                0x004B7FB5,
                "004B7FB5: an automatic door's controller (0060E0A0)",
            ));
        }
        if world
            .get::<NiObjectNet>(node)
            .is_some_and(|net| net.controller_0c.is_some())
        {
            return Err(GameError::unclassified(
                0x004B807C,
                "004B807C: a door node's controller head (00B3B900's removal)",
            ));
        }
    }
    // 004D7D10.
    world.entity_mut(reference).insert(Reference3D(node));
    world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .loaded_3d_3c = 1;
    // 004E40F0..004E413A: an actor's node is named "(%08X) -> %s (%08X)": its form ID, the base's full name or "", the base's
    // form ID (BSStringT_Format, NiObjectNET_SetName); a creature's 00625850 follows, which an NPC skips.
    if actor {
        let id = world
            .get::<FormIdentity>(reference)
            .expect("a form")
            .form_id_0c;
        let base_id = world.get::<FormIdentity>(base).expect("a form").form_id_0c;
        let mut name = format!("({id:08X}) -> ").into_bytes();
        name.extend_from_slice(&base_full_name(world, base));
        name.extend_from_slice(format!(" ({base_id:08X})").as_bytes());
        world
            .get_mut::<NiObjectNet>(node)
            .ok_or(GameError::unclassified(
                0x004E412E,
                "004E412E: an actor node with no NiObjectNET",
            ))?
            .name = Some(name);
    }
    // 004E41C0..004E41F8: BSX bit 0x10 walks the FlameNodes, off (004DE1C0) for +0x08 bit 0x80, else on (004E26F0).
    if bsx_flag(world, node, 0x10) {
        if flags & 0x80 != 0 {
            return Err(GameError::unclassified(
                0x004E41F0,
                "004E41F0: 004DE1C0, the FlameNodes' off walk",
            ));
        }
        match flame_nodes_on(world, node, services) {
            Ok(_) => {}
            Err(error) if is_clone_gap(&error) => {
                world
                    .get_resource_or_init::<Load3DHostGates>()
                    .gated
                    .push((reference, error.message));
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
    }
    if base_type == 0x21 {
        detach_scabbard(world, node);
    }
    let (position, scale) = {
        let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
        (refr.position_2c, refr.scale_38)
    };
    let rotation = crate::tes::forms_references::reference_initial_rotation(world, reference)?;
    {
        let mut payload = world.get_mut::<SceneNodePayload>(node).expect("a NiNode");
        payload.local_transform.translation = position;
        payload.local_transform.rotation = rotation;
    }
    // 004D6F20: not an actor (vt+0x190) and +0x08 bit 0x100000.
    if !actor
        && world
            .get::<FormIdentity>(reference)
            .expect("a form")
            .flags_08
            & 0x100000
            != 0
    {
        return Err(GameError::unclassified(
            0x004E41F7,
            "004E41F7: 004D6F20's Havok arm (0088CEB0)",
        ));
    }
    mark_collision_for_sync(world, node);
    attach_reference_extra(world, node, reference)?;
    let classifier = classify_reference(world, node)?;
    set_geometry_classifier(world, node, classifier)?;
    let statics = world.resource::<DefaultFormStatics>();
    if Some(base) == statics.door_marker_b35ea4 || Some(base) == statics.travel_marker_b35eb4 {
        world
            .get_mut::<SceneNodePayload>(node)
            .expect("a NiNode")
            .effective_av_flags |= 1;
    }
    // 004E438D: a teleport whose linked door (0042B460) has a parent cell counts that cell up (004CCA60 with 1).
    if let Some(cell) = crate::tes::base_extra_list::teleport_door(world, reference)
        .and_then(|door| crate::tes::forms_references::parent_cell(world, door))
    {
        cell_change_process_middle_low_count(world, cell, true)?;
    }
    let extras = &world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44;
    if extras.get(0x4B).is_some() || extras.get(0x4A).is_some() {
        return Err(GameError::unclassified(
            0x00424E60,
            "00424E60: saved animation data (extras 0x4B, 0x4A)",
        ));
    }
    if actor {
        // 004E436E TESObjectREFR_InitAnimation: an NPC's or a creature's arm, through 00475D80's animation setup.
        if base_type == 0x24 {
            creature_init_animation(world, reference, base, node, services)?;
        } else {
            npc_init_animation(world, reference, base, node, services)?;
        }
    }
    // 004E3490: the node's first child with a +0x0C controller takes the NiControllerManager arm.
    let first = world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(node)
        .and_then(|children| children.slots_ac.first().copied().flatten());
    // 004E34E9 004D83B0(ref, 0): a non-actor's extra 0x15 (an animation) is removed when present.
    if world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44
        .get(0x15)
        .is_some()
    {
        return Err(GameError::unclassified(
            0x004D83F6,
            "004D83F6: a reference's extra 0x15 removed",
        ));
    }
    if let Some(head) = first
        .and_then(|first| world.get::<NiObjectNet>(first))
        .and_then(|net| net.controller_0c)
    {
        run_manager_idle(world, reference, base_type, node, head)?;
    } else {
        let result = crate::gamebryo::scene_update::update_scene_object(
            world,
            node,
            0.0,
            false,
            &mut crate::gamebryo::scene_update::NoSceneControllers,
        )?;
        if result.status != crate::gamebryo::scene_update::SceneUpdateStatus::Updated {
            return Err(GameError::unclassified(
                0x004E3A4C,
                "004E3A4C: the node's vt+0x60 update stopped",
            ));
        }
    }
    world
        .get_mut::<SceneNodePayload>(node)
        .expect("a NiNode")
        .local_transform
        .scale = scale.abs();
    crate::gamebryo::scene_update::update_scene_node_model_bound(world, node);
    crate::gamebryo::scene_state::refresh_property_state(
        world,
        node,
        &services.prepare.property_defaults,
    )?;
    if actor && let Some(process) = crate::tes::actor::process_of(world, reference) {
        // 004E4467 reads the reference's raw +3C after refreshing its property state.
        crate::tes::actor_enable::find_skin_property(world, process, node)?;
        if world
            .get::<crate::tes::actor_process::HighProcess>(process)
            .is_some()
        {
            crate::tes::actors::actor_process_high::find_bone_lod_controller(
                world, process, reference,
            );
            world
                .get_mut::<crate::tes::actor_process::MiddleHighProcess>(process)
                .unwrap()
                .face_animation_data_15c = None;
        }
    }
    // 004E4605..004E462D: use the reference's saved XRGD after the animation setup.
    if actor
        && (crate::tes::actor::actor_is_dead(world, reference, false)
            || crate::tes::actor::process_of(world, reference)
                .and_then(|p| world.get::<crate::tes::actor_process::MiddleHighProcess>(p))
                .is_some_and(|p| p.knocked_state_11c != 0))
    {
        restore_reference_pose(world, reference, node)?;
    }
    Ok(Some(node))
}

/// IDA: 004D83B0 TESObjectREFR_SetAnimData(reference, anim) for an actor (vt+0x188): vt+0x48 (AddChangeFlags) or, for null, vt+0x44 (ClearChangeFlags) with 0x2000000; a process of level 0 or 1 takes it (0064FFF0: an older one, unless the same, is destroyed by 00475B60 (`destroy_actor_anim_data`) and freed; then +0x17C). A higher level's extra 0x15 (0041E630, 0041E9C0) is a stop. Answers `anim`. Completion: 100% of an actor with a high process.
/// # Errors
/// As above.
pub fn set_actor_anim_data(
    world: &mut World,
    reference: Entity,
    anim: Option<Entity>,
) -> Result<Option<Entity>, GameError> {
    if anim.is_some() {
        crate::tes::forms_init::record_form_change(world, reference, 0x0200_0000)?;
    } else {
        crate::tes::forms_items::clear_change_flags(world, reference, 0x0200_0000)?;
    }
    let process = crate::tes::actor::process_of(world, reference);
    if let Some(process) = process
        && crate::tes::actor_process::process_level(world, process) <= 1
    {
        let state = world
            .get_mut::<crate::tes::actor_process::MiddleHighProcess>(process)
            .ok_or(GameError::unclassified(
                0x0064FFF0,
                "0064FFF0: a level 0 or 1 process with no MiddleHigh part",
            ))?;
        if let Some(old) = state.anim_data_17c {
            if Some(old) == anim {
                return Ok(anim);
            }
            destroy_actor_anim_data(world, old);
        }
        world
            .get_mut::<crate::tes::actor_process::MiddleHighProcess>(process)
            .expect("tested above")
            .anim_data_17c = anim;
        return Ok(anim);
    }
    Err(GameError::unclassified(
        0x004D83F6,
        "004D83F6: an actor's animation in extra 0x15 (no high process)",
    ))
}

// IDA: 004E3490 TESObjectREFR_InitAnimation's NPC arm (an actor's node is a NiNode here): 004D83B0(ref, 0); the base's
// model must be a "Skeleton*" (a warning otherwise); with a skin info (vt+0x168): 004796F0, a new ActorAnimData (FormHeap 0xDC,
// 00473EB0) through 004D83B0, the node again (gone: "SetAnimation cleared 3D because %s (%08X) was moved"), 004E1580 (TimeInfo's
// disable count around vt+0x164's 004728C0 on the new data, a no-op with no accum root, and 00526DB0), the health (vt+0x284(8)):
// a living NPC's KF list by IdleAnimRoot_FindKFFiles(model, 0), 00475D80 ("Bad InitAnimation for NPC '%s' (%08X). Missing 'Idle'
// animation." when it fails), then the tail: 00476080's special animations (a stop), the process's vt+0x2FC / vt+0x308 and
// vt+0x150 (00656270), 00476D10 and LABEL_94 (`actor_animation_tail`).
fn npc_init_animation(
    world: &mut World,
    reference: Entity,
    base: Entity,
    node: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    set_actor_anim_data(world, reference, None)?;
    let path = crate::tes::reference_load::model_path_of(world, base).unwrap_or_default();
    if let Some(slash) = path.iter().rposition(|&byte| byte == b'\\')
        && oblivion_core::crt::strnicmp(&path[slash + 1..], b"Skeleton", 8) != 0
    {
        return Err(GameError::unclassified(
            0x004E3539,
            "004E3539: the \"needs to have a 'skeleton*.nif'\" warning's editor name",
        ));
    }
    let Some(skin) = crate::tes::character_3d::skin_info(world, reference) else {
        return Err(GameError::unclassified(
            0x004E3A38,
            "004E3A38: an NPC with no skin info (LABEL_94's tail)",
        ));
    };
    crate::tes::character_3d::release_all_slots(world, skin)?;
    let anim = world
        .spawn(crate::tes::actor_animation::ActorAnimData::default())
        .id();
    set_actor_anim_data(world, reference, Some(anim))?;
    crate::tes::character_3d::npc_update_biped(world, base, reference, services)?;
    let health = crate::tes::actor::actor_get_actor_value(world, reference, 8)?;
    if f64::from(health) <= 0.0 {
        return Err(GameError::unclassified(
            0x004E3AC2,
            "004E3AC2: a dead NPC's Idle.KF and Death.KF",
        ));
    }
    let files = crate::tes::forms_ai::root_kf_files(world, &path)?;
    if !crate::tes::actor_animation::init_from_kf_list(
        world,
        anim,
        files,
        Some(node),
        reference,
        services,
    )? {
        return Err(GameError::unclassified(
            0x004E3B8A,
            "004E3B8A: \"Bad InitAnimation for NPC\" (its editor name)",
        ));
    }
    // 004E3BB2: a living NPC whose base's TESAnimation lists special animations (TESAnimation_HasNames) takes
    // 00476080, a stop; none of the nine has one.
    if world
        .get::<crate::tes::form_components::TesAnimation>(base)
        .is_some_and(|animation| !animation.names_04.is_empty())
    {
        return Err(GameError::unclassified(
            0x004E3C1C,
            "004E3C1C: 00476080, the NPC's special animations",
        ));
    }
    // 004E3D9E: with a process, vt+0x2FC (0064AD10, +0x114) takes vt+0x308(1) (0064AD40, +0x115); the player's arm is a stop;
    // then vt+0x150 (00656270) with vt+0x304's +0x115, the skin info, the anim data (004D8370) and the actor.
    if let Some(process) = world
        .get::<crate::tes::actor::MobileObject>(reference)
        .and_then(|mobile| mobile.process_58)
    {
        match world.get_mut::<crate::tes::actor_process::MiddleHighProcess>(process) {
            Some(mut middle) => {
                if middle.weapon_draw_requested_114 != 0 {
                    middle.weapon_drawn_115 = 1;
                }
            }
            None => crate::tes::actor_enable::low_process_only(
                world,
                process,
                "004E3DAC: a process's vt+0x2FC",
            )?,
        }
        // 004E3DD7: the player whose view skin (vt+0x168: +0x104 while +0x588 is set) is its first-person one (006600D0(1),
        // +0x5C8) passes 0065D750(1)'s first-person anim data, a stop here; otherwise the anim data, as an NPC's.
        if Some(reference) == crate::tes::player::the_player(world) {
            let data = world
                .get::<crate::tes::player::PlayerCharacter>(reference)
                .expect("the player");
            let view = if data.view_flags_588[0] != 0 {
                Some(skin)
            } else {
                data.first_person_skin_info_5c8.or(Some(skin))
            };
            if view.is_some() && view == data.first_person_skin_info_5c8 {
                return Err(GameError::unclassified(
                    0x004E3DE2,
                    "004E3DE2: the player's vt+0x150 with its first-person skin",
                ));
            }
        }
        crate::tes::biped_attach::place_weapon(world, process, skin, reference)?;
    }
    // LABEL_92 / LABEL_93: 00476D10(0.0, -1.0), then LABEL_94.
    crate::tes::actor_animation::update_anim_data(world, anim, reference, 0.0, -1.0, services)?;
    actor_animation_tail(world, reference, node, Some(anim))
}

/// IDA: N/A host deferral D15 for 004E1900's arm for an actor with a node and no skin info: 00479450(model, slot 9 or 14 for a bow, ref, null), named
/// "Weapon (%08X)" (a bow's by 00A3CF00), then outside a load 005E0DA0 and the process's vt+0x124 / vt+0x308 / vt+0x150, a bow's
/// 005E13D0 and vt+0x114, 005EA1A0 and 005EE1B0. The weapon remains deferred. The equip call makes no
/// draws in run-af.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct CreatureBodyDeferrals {
    /// The actor and the weapon, per 004E1900 arm not run.
    pub weapons: Vec<(Entity, Entity)>,
}

// IDA: 004E3490 TESObjectREFR_InitAnimation's creature arm (004E3836): 004D83B0(ref, 0) and the "Skeleton*" warning (as the NPC
// arm's); a new ActorAnimData through 004D83B0, the node again (gone: "SetAnimation cleared 3D because %s (%08X) was moved"),
// 004E1580 (004728C0 on the new data, a no-op with no accum root; `creature_body_equip`, then the body models), the health
// (004D7570, vt+0x284(8) on an actor): a dead creature's list is "Data\Meshes\" + the model path with its last component
// "\Idle.KF", through 00434850 (00431970 with mask 1), then "\Death.KF" into the same list (prepended); a living one's
// IdleAnimRoot_FindKFFiles(model, 0). 00475D80, returning at 004E3A26 ("Bad InitAnimation for Creature '%s' (%08X). Missing
// 'Idle' animation." when it fails, a stop for the editor name), then 00476D10 ActorAnimData_Update(0.0, -1.0) and LABEL_94
// (`actor_animation_tail`). A living creature's KF files and special animations (00476080) are stops.
fn creature_init_animation(
    world: &mut World,
    reference: Entity,
    base: Entity,
    node: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    set_actor_anim_data(world, reference, None)?;
    let path = crate::tes::reference_load::model_path_of(world, base).unwrap_or_default();
    if let Some(slash) = path.iter().rposition(|&byte| byte == b'\\')
        && oblivion_core::crt::strnicmp(&path[slash + 1..], b"Skeleton", 8) != 0
    {
        return Err(GameError::unclassified(
            0x004E3539,
            "004E3539: the \"needs to have a 'skeleton*.nif'\" warning's editor name",
        ));
    }
    let anim = world
        .spawn(crate::tes::actor_animation::ActorAnimData::default())
        .id();
    set_actor_anim_data(world, reference, Some(anim))?;
    crate::tes::equipment::creature_body_equip(world, reference)?;
    let folder = match path.iter().rposition(|&byte| byte == b'\\') {
        Some(slash) => path[..=slash].to_vec(),
        None => Vec::new(),
    };
    let bodies: Vec<Vec<u8>> = world
        .get::<crate::tes::form_components::TesModelList>(base)
        .map(|list| {
            list.names_04
                .iter()
                .map(|name| {
                    let name = name.split(|&byte| byte == 0).next().unwrap_or_default();
                    [folder.as_slice(), name].concat()
                })
                .collect()
        })
        .unwrap_or_default();
    for body in bodies {
        crate::tes::biped_attach::attach_creature_body_model(
            world, reference, node, &body, services,
        )?;
    }
    let health = crate::tes::actor::actor_get_actor_value(world, reference, 8)?;
    if f64::from(health) > 0.0 {
        return Err(GameError::unclassified(
            0x004E3846,
            "004E3846: a living creature's KF files and special animations",
        ));
    }
    let text: String = path.iter().map(|&byte| char::from(byte)).collect();
    let name = format!("Data\\Meshes\\{text}");
    let cut = name.rfind('\\').expect("the prefix has one");
    let mut files: Vec<String> = Vec::new();
    for file in ["\\Idle.KF", "\\Death.KF"] {
        let pattern = format!("{}{file}", &name[..cut]);
        let mut found = Vec::new();
        if let Some(archives) = services.archives.as_deref_mut() {
            archives.with_archives(&mut |manager| {
                if let Some(manager) = manager {
                    found = manager.find_matching_files(&pattern, &text, 1);
                }
            });
        }
        found.extend(files);
        files = found;
    }
    let files = (!files.is_empty()).then_some(files);
    if !crate::tes::actor_animation::init_from_kf_list(
        world,
        anim,
        files,
        Some(node),
        reference,
        services,
    )? {
        return Err(GameError::unclassified(
            0x004E3A2E,
            "004E3A2E: \"Bad InitAnimation for Creature\" (its editor name)",
        ));
    }
    // 004E3A26 returns; a dead creature has no special animations. LABEL_93: 00476D10(0.0, -1.0), then LABEL_94.
    crate::tes::actor_animation::update_anim_data(world, anim, reference, 0.0, -1.0, services)?;
    actor_animation_tail(world, reference, node, Some(anim))
}

// IDA: 004E3490's LABEL_94 for an actor (vt+0x190): the process's weapon and arrow nodes (`process_animation_nodes`), then the
// node's NiMultiTargetTransformController (NiObjectNET_GetController) takes flag 0x40. Extra 0x34 with new animation data
// (NiAVObject_Update at its +0x10 and 0041F5A0's removal) is a stop. Completion: 100% of those arms.
fn actor_animation_tail(
    world: &mut World,
    reference: Entity,
    node: Entity,
    anim: Option<Entity>,
) -> Result<(), GameError> {
    crate::tes::equipment_3d::process_animation_nodes(world, reference, anim)?;
    if let Some(controller) = crate::gamebryo::object_net::get_controller(
        world,
        node,
        crate::gamebryo::stream_objects::NiClass::MultiTargetTransformController,
    ) && let Some(mut state) =
        world.get_mut::<crate::gamebryo::transform_controller::NodeController>(controller)
    {
        state.time.fields.flags |= 0x40;
    }
    if anim.is_some()
        && world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44
            .get(0x34)
            .is_some()
    {
        return Err(GameError::unclassified(
            0x004E3EEE,
            "004E3EEE: extra 0x34's update time",
        ));
    }
    Ok(())
}

// IDA: 004D9C30 TESObjectREFR_GetBaseFullNameOrEmpty's text for a base: its TESFullName, to the terminator, or "".
pub(crate) fn base_full_name(world: &World, base: Entity) -> Vec<u8> {
    world
        .get::<crate::tes::form_components::TesFullName>(base)
        .and_then(|name| name.name_04.data_00.clone())
        .map(|name| {
            let end = name
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(name.len());
            name[..end].to_vec()
        })
        .unwrap_or_default()
}

// IDA: 004E4206..004E425E: NiObjectNET_GetExtraData(node, "REF") cast to TESObjectExtraData takes +0x0C = the reference; none
// gets a new one through AddExtraDataWithGeneratedName, which inserts by name (006FF570).
fn attach_reference_extra(
    world: &mut World,
    node: Entity,
    reference: Entity,
) -> Result<(), GameError> {
    let extras = world
        .get::<NiObjectNet>(node)
        .ok_or(GameError::unclassified(
            0x004E4206,
            "004E4206: a node with no NiObjectNET",
        ))?
        .extra_data
        .clone();
    for &extra in &extras {
        let named = world
            .get::<NiExtraDataName>(extra)
            .is_some_and(|name| name.0.as_deref() == Some(b"REF".as_slice()));
        if named && let Some(mut data) = world.get_mut::<TesObjectExtraData>(extra) {
            data.reference_0c = reference;
            return Ok(());
        }
    }
    let extra = world
        .spawn((
            crate::gamebryo::stream_objects::NiClass::TesObjectExtraData,
            NiExtraDataName(Some(b"REF".to_vec())),
            TesObjectExtraData {
                reference_0c: reference,
            },
        ))
        .id();
    let position = extras
        .iter()
        .position(|&other| {
            world
                .get::<NiExtraDataName>(other)
                .and_then(|name| name.0.as_deref())
                .is_some_and(|name| name > b"REF".as_slice())
        })
        .unwrap_or(extras.len());
    world
        .get_mut::<NiObjectNet>(node)
        .expect("checked")
        .extra_data
        .insert(position, extra);
    Ok(())
}

/// IDA: 004E0CF0 SceneNode_ClassifyReferenceFormType: 004DC270 finds the TESObjectExtraData on the node or a parent; its
/// reference's base type maps ACTI, DOOR, STAT to 1, ARMO, CLOT, WEAP, AMMO to 4, CONT, FURN to 2, INGR, MISC, KEYM to 5, NPC_,
/// CREA to 3, anything else 6; no reference is 0. vt+0x170's base is read at +0x04 with no null test (004E0D14), so a
/// reference with no base faults: a host stop. Completion: 100% but the player's first-person node, which 004DC270 tries
/// first and which isn't built.
/// # Errors
/// A reference with no base form.
pub fn classify_reference(world: &World, node: Entity) -> Result<i32, GameError> {
    let mut cursor = Some(node);
    while let Some(object) = cursor {
        if let Some(net) = world.get::<NiObjectNet>(object) {
            for &extra in &net.extra_data {
                if let Some(data) = world.get::<TesObjectExtraData>(extra) {
                    let base = world
                        .get::<TesObjectRefr>(data.reference_0c)
                        .and_then(|refr| refr.base_form_1c)
                        .ok_or(GameError::unclassified(
                            0x004E_0D14,
                            "004E0D14: a null base form faults",
                        ))?;
                    return Ok(classify_reference_base_form_type(form_type(world, base)));
                }
            }
        }
        cursor = world.get::<NiAvParent>(object).and_then(|parent| parent.0);
    }
    Ok(0)
}

// IDA: 004E0D14..004E0D57, the base-type switch of 004E0CF0: `add eax, 0FFFFFFEEh`, `cmp eax, 15h`, `ja` the default, so
// every type outside 18..39 is 6; ACTI, DOOR, STAT 1; CONT, FURN 2; NPC_, CREA 3; ARMO, CLOT, WEAP, AMMO 4; INGR, MISC,
// KEYM 5; the rest 6.
#[must_use]
pub const fn classify_reference_base_form_type(form_type: u8) -> i32 {
    match form_type {
        0x12 | 0x18 | 0x1C => 1,
        0x14 | 0x16 | 0x21 | 0x22 => 4,
        0x17 | 0x20 => 2,
        0x19 | 0x1B | 0x27 => 5,
        0x23 | 0x24 => 3,
        _ => 6,
    }
}

/// IDA: the reference queues as the host runs them (D9: the clone thread and 00433590's timed run on the main thread, each
/// queue in arrival order, after the IO pass): every clone-queue task through `0043AF30` BackgroundCloneThread's task body
/// (the cell attached by 0043F8C0; 00B350D5 clear; not an actor (vt+0x190), `TES_ReferenceUsesCloneThread`, no +0x3C: vt+0x14C
/// Load3D into +0x2C; then TaskQueue_Push to the IOManager's queue, which is the host's finish queue), then every finish-queue
/// task through vt+0x14 → vt+0x38 `QueuedReference_FinishOnMainThread`.
/// Completion: 100% of both queues' bodies; their callees stop where they aren't ported.
pub fn run_reference_queues(
    world: &mut World,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    while let Some(id) = pop_queue(world, true) {
        let task = world
            .resource::<TaskGlobals>()
            .model_loader_b33a1c
            .as_ref()
            .and_then(|loader| loader.references.get(id))
            .cloned()
            .ok_or(GameError::unclassified(
                0x0043AF30,
                "0043AF30: a released reference task",
            ))?;
        let reference = task.reference_20;
        let attached = crate::tes::forms_references::parent_cell(world, reference)
            .is_some_and(|cell| crate::tes::forms_world::cell_attached(world, Some(cell), false));
        if !attached {
            continue;
        }
        if world.get::<crate::tes::actor::Actor>(reference).is_none()
            && crate::tes::tes::reference_takes_clone(world, reference)
            && world.get::<Reference3D>(reference).is_none()
        {
            let node = reference_load_3d(world, reference, services)?;
            if let Some(task) = world
                .resource_mut::<TaskGlobals>()
                .model_loader_b33a1c
                .as_mut()
                .and_then(|loader| loader.references.get_mut(id))
            {
                task.clone_2c = node;
            }
        }
        world
            .resource_mut::<TaskGlobals>()
            .model_loader_b33a1c
            .as_mut()
            .expect("checked")
            .references
            .finish_queue
            .push_back(id);
    }
    while let Some(id) = pop_queue(world, false) {
        finish_on_main_thread(world, id, services)?;
    }
    Ok(())
}

// IDA: N/A the front of the clone or the finish queue.
fn pop_queue(
    world: &mut World,
    clone: bool,
) -> Option<crate::bs::reference_tasks::ReferenceTaskId> {
    let mut tasks = world.resource_mut::<TaskGlobals>();
    let references = &mut tasks.model_loader_b33a1c.as_mut()?.references;
    if clone {
        references.clone_queue.pop_front()
    } else {
        references.finish_queue.pop_front()
    }
}

/// IDA: N/A host gate: the references whose clone reached a class with no rule yet (`gamebryo::clone`), which get no 3D here
/// where the original has it, with the stop's message. The original clones them; the list falls as the rules are ported.
#[derive(Resource, Clone, Debug, Default)]
pub struct Load3DHostGates {
    pub gated: Vec<(Entity, &'static str)>,
}

// IDA: N/A the clone's class stops, which `reference_load_3d` gates.
fn is_clone_gap(error: &GameError) -> bool {
    error.message.starts_with("00700610: a clone of")
        || error.message.starts_with("00700300: a clone of")
        || error.message.starts_with("004B38AE")
}

// IDA: 004CCA60 TESObjectCELL_ChangeProcessMiddleLowCount(cell, up): the cell's extra 0x0D (ExtraProcessMiddleLow, 0041D9C0, vtable 00A34DA4, +0x0C = 0). With none, `up` clear returns at once; `up` set makes one and adds it (0041E0A0), then walks the cell's +0x48 list under the region lock (a host no-op) and calls vt+0x1AC (005EBB30, `actor_process_moves::actor_move_to_middle_low_process`) at 004CCB42 on each MobileObject with +0x08 bit 0x800 clear, a +0x58 process and process level 3, each call recorded in `MiddleLowPromotions`. Then +0x0C counts up or down (a wrapping dword), and at 0 00422E20(entry, 1) removes and deletes the extra. 004E43BA (Load3D) counts up; 004E1557 (Set3D with a teleport door's 3D going) counts down. run-ah has the up calls on the clone thread. Completion: 100%.
fn cell_change_process_middle_low_count(
    world: &mut World,
    cell: Entity,
    up: bool,
) -> Result<(), GameError> {
    use crate::tes::base_extra_list::{EXTRA_TYPE_0D, ExtraData, new_extra_entry};
    let has = world
        .get::<crate::tes::forms_world::TesObjectCell>(cell)
        .expect("a cell")
        .extra_28
        .get(EXTRA_TYPE_0D)
        .is_some();
    if !has {
        // 004CCAA3..004CCAA7 `up` clear with no extra returns.
        if !up {
            return Ok(());
        }
        world
            .get_mut::<crate::tes::forms_world::TesObjectCell>(cell)
            .expect("a cell")
            .extra_28
            .add(new_extra_entry(EXTRA_TYPE_0D, ExtraData::Dword(0)));
        let objects: Vec<Entity> = world
            .get::<crate::tes::forms_world::TesObjectCell>(cell)
            .expect("a cell")
            .objects_48
            .items()
            .collect();
        for object in objects {
            if world
                .get::<crate::tes::actor::MobileObject>(object)
                .is_none()
            {
                continue;
            }
            let flags = world
                .get::<crate::tes::form::FormIdentity>(object)
                .expect("a form")
                .flags_08;
            if flags & 0x800 == 0 && crate::tes::actor::actor_process_level(world, object) == 3 {
                // 004CCB38..004CCB42 vt+0x1AC.
                world
                    .get_resource_or_insert_with(
                        crate::tes::actor_process_moves::MiddleLowPromotions::default,
                    )
                    .actors
                    .push(object);
                // run-ah: on the clone thread.
                crate::tes::actor_process_moves::actor_move_to_middle_low_process(
                    world, object, true,
                )?;
            }
        }
    }
    let mut data = world
        .get_mut::<crate::tes::forms_world::TesObjectCell>(cell)
        .expect("a cell");
    let entry = data
        .extra_28
        .get_mut(EXTRA_TYPE_0D)
        .expect("just found or added");
    let mut count = 1;
    if let ExtraData::Dword(value) = &mut entry.data {
        *value = if up {
            value.wrapping_add(1)
        } else {
            value.wrapping_sub(1)
        };
        count = *value;
    }
    // 004CCB5E..004CCB7A the count; at 0, 00422E20(entry, 1).
    if count == 0 {
        data.extra_28.remove_entry(EXTRA_TYPE_0D);
    }
    Ok(())
}

/// IDA: N/A the clone thread's CRT rand: `_rand` 009859E2 reads +0x14 of the per-thread data, which `_initptd` sets to 1, and
/// Random_Next's first-use flag is the main thread's to clear, so the thread's draws run its own sequence from `srand(1)`.
/// D9 runs the clone queue on the main thread, so this stands for that thread's state.
/// TODO: [ ] a capture of the clone thread's draws (none probed: run-af is main-thread only).
#[derive(Resource, Default, Debug)]
pub struct CloneThreadRand(pub oblivion_core::crt::MsvcRand);

/// IDA: 004E26F0(node), the FlameNodes' on walk: a name `FlameNode<c>` (`_strnicmp`, 9) looks up the default static `c - 0x12`
/// for a digit or `tolower(c) - 0x39` for a letter (0046B250); each child with vt+8 (a node) whose name starts `BASE`
/// (`strncmp`, 4) is detached in place (vt+0x8C, the slot goes null); a form whose +0x24 TESModel path is nonempty gives
/// vt+0x114 (004B2320 → 004B3750 with no reference), and a result takes `Random_Next(0) % 1000` (`idiv`), `fild`, `fdiv` 100.0
/// (A309F0), `fstp` float, 004DE3C0 over it and AttachChild(+0x84, it, 1), and answers true. A name `FlameCap` (`_stricmp`)
/// clears +0x18 bit 0. Then each child whose RTTI is NiNode itself (B3CA98's, not a derived class) recurses; any true sets
/// +0x18 to `& 0xFFEF | 2`. Completion: 100%; the clone thread's x87 precision for the quotient is not captured, so the
/// 53-bit form is used.
/// TODO: [ ] the clone thread's control word (the quotient's rounding).
fn flame_nodes_on(
    world: &mut World,
    node: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<bool, GameError> {
    use crate::gamebryo::scene_node::NiNodeChildren;
    let name = world
        .get::<NiObjectNet>(node)
        .and_then(|net| net.name.clone())
        .map(|name| {
            let end = name
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(name.len());
            name[..end].to_vec()
        });
    let mut attached = false;
    if let Some(name) = name {
        if name.len() >= 9 && name[..9].eq_ignore_ascii_case(b"FlameNode") {
            let next = name.get(9).copied().unwrap_or(0);
            let form = if next.is_ascii_digit() {
                world
                    .resource::<crate::tes::data_handler::FormsById>()
                    .lookup(u32::from(next).wrapping_sub(0x12))
            } else if next.is_ascii_alphabetic() {
                world
                    .resource::<crate::tes::data_handler::FormsById>()
                    .lookup(u32::from(next.to_ascii_lowercase()).wrapping_sub(0x39))
            } else {
                None
            };
            let children = world
                .get::<NiNodeChildren>(node)
                .map(|children| children.slots_ac.clone())
                .unwrap_or_default();
            for (index, child) in children.into_iter().enumerate() {
                let Some(child) = child else { continue };
                if world.get::<NiNodeChildren>(child).is_none() {
                    continue;
                }
                let base = world
                    .get::<NiObjectNet>(child)
                    .and_then(|net| net.name.as_deref())
                    .is_some_and(|name| name.starts_with(b"BASE"));
                if base {
                    // 0070B1D0 vt+0x8C DetachChildAt: the slot goes null, the child's +0x1C null.
                    crate::gamebryo::scene_node::set_child_at(world, node, index, None);
                }
            }
            if let Some(form) = form
                && model_path_len(world, form)? != 0
                && let Some(flame) = bound_object_create_3d(world, form, None, services)?
            {
                let draw = world.get_resource_or_init::<CloneThreadRand>().0.rand();
                let value = (f64::from(draw % 1000) / 100.0) as f32;
                set_controller_phases(world, Some(flame), value)?;
                crate::bs::scene_graphs::attach_scene_child(world, node, Some(flame), true);
                attached = true;
            }
        } else if name.eq_ignore_ascii_case(b"FlameCap") {
            world
                .get_mut::<SceneNodePayload>(node)
                .expect("a NiNode")
                .effective_av_flags &= !1;
        }
    }
    let children = world
        .get::<NiNodeChildren>(node)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default();
    for child in children.into_iter().flatten() {
        let exact_node = world.get::<crate::gamebryo::stream_objects::NiClass>(child)
            == Some(&crate::gamebryo::stream_objects::NiClass::Node)
            && world
                .get::<crate::bs::fade_node::BsFadeNode>(child)
                .is_none();
        if exact_node && flame_nodes_on(world, child, services)? {
            attached = true;
        }
    }
    if attached {
        let mut payload = world.get_mut::<SceneNodePayload>(node).expect("a NiNode");
        payload.effective_av_flags = payload.effective_av_flags & 0xFFEF | 2;
    }
    Ok(attached)
}

// IDA: the word at TESModel +0x08 (form +0x2C), or `strlen` of +0x04 (+0x28) for 0xFFFF.
fn model_path_len(world: &World, form: Entity) -> Result<usize, GameError> {
    let Some(model) = world.get::<crate::tes::form_components::TesModel>(form) else {
        return Err(GameError::unclassified(
            0x004E27F0,
            "004E27F0: a FlameNode form with no TESModel at +0x24",
        ));
    };
    let text = &model.path_04;
    if text.length_04 != 0xFFFF {
        return Ok(usize::from(text.length_04));
    }
    let data = text.data_00.as_deref().ok_or(GameError::unclassified(
        0x004E27FA,
        "004E27FA: strlen of a null +0x28",
    ))?;
    Ok(data
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(data.len()))
}

/// IDA: 0043B000 QueuedReference_FinishOnMainThread (vt+0x38): a task not at state 6 whose reference has neither +0x08 bit
/// 0x800 nor 0x20 and whose cell is attached (0043F8C0): with a Model (+0x28), `0043A8F0` on its +0x08 root, then
/// `TES_FinishReferenceLoad3D(g_tes, ref, cell, +0x2C, null)`; any task not at state 6 then leaves the +0x08 map (vt+0x10).
/// Completion: 100%.
fn finish_on_main_thread(
    world: &mut World,
    id: crate::bs::reference_tasks::ReferenceTaskId,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let task = world
        .resource::<TaskGlobals>()
        .model_loader_b33a1c
        .as_ref()
        .and_then(|loader| loader.references.get(id))
        .cloned()
        .ok_or(GameError::unclassified(
            0x0043B000,
            "0043B000: a released reference task",
        ))?;
    if task.state_0c == 6 {
        return Ok(());
    }
    let reference = task.reference_20;
    use crate::bs::reference_tasks::QueuedReferenceClass;
    if matches!(
        task.class,
        QueuedReferenceClass::Character
            | QueuedReferenceClass::Creature
            | QueuedReferenceClass::Player
    ) {
        // 0043B090 QueuedCharacter_FinishOnMainThread: this finish, then 004353D0 with the head (+0x38) and 00437B60 with the
        // helmet (+0x3C); a QueuedCreature's is this finish. Build 2: a stop inside is recorded (`CharacterTaskStops`) and the
        // task stays in the map.
        match finish_reference_task(world, &task, services) {
            Err(stop) => world
                .get_resource_or_init::<crate::tes::character_3d::CharacterTaskStops>()
                .stops
                .push((reference, stop.message)),
            Ok(()) => {
                world
                    .get_resource_or_init::<crate::tes::character_3d::CharacterTaskStops>()
                    .finished
                    .push(reference);
                // 0043B090's 004353D0 with +0x38's head, then 00437B60 with +0x3C's helmet.
                let head = world
                    .get_resource::<crate::tes::face_gen_tasks::FaceGenTasks>()
                    .and_then(|tasks| tasks.head_of_actor_38.get(&id).copied());
                if let Some(head) = head {
                    let defaults = services.prepare.property_defaults;
                    if let Err(stop) = crate::tes::face_gen_nodes::queued_head_finish_for_actor(
                        world, head, reference, &defaults,
                    ) {
                        world
                            .get_resource_or_init::<crate::tes::character_3d::CharacterTaskStops>()
                            .stops
                            .push((reference, stop.message));
                    }
                }
                let helmet = world
                    .get_resource::<crate::tes::helmet_tasks::HelmetTasks>()
                    .and_then(|tasks| tasks.helmet_of_actor_3c.get(&id).copied());
                if let Some(helmet) = helmet
                    && let Err(stop) =
                        crate::tes::helmet_tasks::queued_helmet_finish(world, helmet, services)
                {
                    world
                        .get_resource_or_init::<crate::tes::character_3d::CharacterTaskStops>()
                        .stops
                        .push((reference, stop.message));
                }
            }
        }
        return Ok(());
    }
    finish_reference_task(world, &task, services)
}

// IDA: 0043B000 QueuedReference_FinishOnMainThread's body past its state test.
fn finish_reference_task(
    world: &mut World,
    task: &crate::bs::reference_tasks::QueuedReference,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let reference = task.reference_20;
    let flags = world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08;
    let cell = crate::tes::forms_references::parent_cell(world, reference);
    if flags & 0x800 == 0
        && flags & 0x20 == 0
        && let Some(cell) =
            cell.filter(|&cell| crate::tes::forms_world::cell_attached(world, Some(cell), false))
    {
        if let Some(model) = task.model_28 {
            let root = world
                .resource::<TaskGlobals>()
                .model_loader_b33a1c
                .as_ref()
                .ok_or(GameError::unclassified(
                    0x0043B03A,
                    "0043B03A: a null g_modelLoader",
                ))?
                .models
                .store[model.0]
                .root_08;
            if let Some(root) = root
                && bsx_flag(world, root, 0x10)
                && flags & 0x80 == 0
            {
                preload_flame_models(world, root, services)?;
            }
        }
        finish_reference_load_3d(world, reference, cell, task.clone_2c, services)?;
    }
    world
        .resource_mut::<TaskGlobals>()
        .model_loader_b33a1c
        .as_mut()
        .ok_or(GameError::unclassified(
            0x0043B082,
            "0043B082: a null g_modelLoader",
        ))?
        .references
        .map_08
        .remove(&reference);
    Ok(())
}

/// IDA: 0043A100(loader, node): a `FlameNode<c>` name's default static (as `004E26F0` picks it; a null form faults, a
/// stop): its TESModel path (+0x24 vt+0x14) not in the model map is loaded at once (`00439EB0(path, 0, 0, 1)`) and the
/// reference that took is dropped again; then each child whose RTTI is NiNode itself recurses. Completion: 100%.
fn preload_flame_models(
    world: &mut World,
    node: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    use crate::gamebryo::scene_node::NiNodeChildren;
    let name = world
        .get::<NiObjectNet>(node)
        .and_then(|net| net.name.clone());
    if let Some(name) = name
        && name.len() >= 9
        && name[..9].eq_ignore_ascii_case(b"FlameNode")
    {
        let next = name.get(9).copied().unwrap_or(0);
        let id = if next.is_ascii_digit() {
            Some(u32::from(next).wrapping_sub(0x12))
        } else if next.is_ascii_alphabetic() {
            Some(u32::from(next.to_ascii_lowercase()).wrapping_sub(0x39))
        } else {
            None
        };
        let form = id
            .and_then(|id| {
                world
                    .resource::<crate::tes::data_handler::FormsById>()
                    .lookup(id)
            })
            .ok_or(GameError::unclassified(
                0x0043A15A,
                "0043A15A: a FlameNode with no default static faults",
            ))?;
        let path = crate::tes::reference_load::model_path_of(world, form).unwrap_or_default();
        let path = fixup_model_path(&path);
        let held = {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let loader = tasks
                .model_loader_b33a1c
                .as_mut()
                .ok_or(GameError::unclassified(
                    0x0043A16A,
                    "0043A16A: a null g_modelLoader",
                ))?;
            crate::bs::model_loader::model_map_lookup(loader, &path)?.is_some()
        };
        if !held
            && crate::bs::model_loader::load_model_now(world, &path, false, 0, true, services)?
                .is_some()
        {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let loader = tasks
                .model_loader_b33a1c
                .as_mut()
                .ok_or(GameError::unclassified(
                    0x0043A1B0,
                    "0043A1B0: a null g_modelLoader",
                ))?;
            if let Some(model) = crate::bs::model_loader::model_map_lookup(loader, &path)? {
                let count = &mut loader.models.store[model.0].references_04;
                *count = count.wrapping_sub(1);
            }
        }
    }
    let children = world
        .get::<NiNodeChildren>(node)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default();
    for child in children.into_iter().flatten() {
        if is_exact_node(world, child) {
            preload_flame_models(world, child, services)?;
        }
    }
    Ok(())
}

// IDA: N/A a vt+4 RTTI test against NiNode's own (B3CA98): the class tag `Node` without a fade node.
fn is_exact_node(world: &World, object: Entity) -> bool {
    world.get::<crate::gamebryo::stream_objects::NiClass>(object)
        == Some(&crate::gamebryo::stream_objects::NiClass::Node)
        && world
            .get::<crate::bs::fade_node::BsFadeNode>(object)
            .is_none()
}

// IDA: the sound form 004D9310 and 00441EF0 take for a reference: an activator's +0x54, a door's +0x60 loop and a light's
// +0x8C, the door and the light only without the reference's +0x08 bit 0x2000; a sound (0x0A) is its own.
fn reference_sound_form(
    world: &World,
    reference: Entity,
    base: Entity,
    flags: u32,
) -> Option<Entity> {
    let resolved = |form: crate::tes::form_components::FormRef| match form {
        crate::tes::form_components::FormRef::Resolved(form) => form,
        crate::tes::form_components::FormRef::Raw(_) => None,
    };
    match form_type(world, base) {
        0x12 => world
            .get::<crate::tes::forms_items::TesActivator>(base)
            .and_then(|a| resolved(a.sound_54)),
        0x18 if flags & 0x2000 == 0 => world
            .get::<crate::tes::forms_items::TesDoor>(base)
            .and_then(|d| d.loop_sound_60),
        0x1A if flags & 0x2000 == 0 => world
            .get::<crate::tes::forms_items::TesLight>(base)
            .and_then(|l| resolved(l.sound_8c)),
        0x0A => Some(base),
        _ => {
            let _ = reference;
            None
        }
    }
}

/// IDA: 00441EF0 TES_FinishReferenceLoad3D(ref, cell, node, a5 = null) for a reference that isn't an actor:
/// `0043FBA0` (+0x08 bits 0x8000 and 0x80000 with a node: a stop); the sound arms (a SOUN reference, an activator, a door
/// without bit 0x2000 and a light likewise, each with its sound: a sound to create, a stop; none in the prison, run-ae);
/// with `TES_ReferenceUsesCloneThread`: `0043F240` (an actor's, a stop), the node given (stored by 004D7D10 when the
/// reference has none) or vt+0x14C Load3D on this thread; with a node, a leveled creature's removal (a stop), otherwise
/// `004D58B0` the cell's node, `007C5D00` (the node against the shadow scene's lights: deferred), `004CEAD0`, `004DC100`,
/// `00667420` (`player_check_reference_overlap`), a light's base vt+0x3C (`0068F970`, nothing) or `007C7050` (the
/// geometry's shader passes and lights: deferred, `light_3d::ShadowSceneDeferrals`). Then
/// `TESObjectREFR_InitializeScriptAnd3DState`, without a save loading `TESObjectREFR_RunPendingScript` and
/// `004FBF90(0x1000)`, and a tree's `004CE610` arm (a stop). Completion: 100% of a non-actor's arms; the stops named.
fn finish_reference_load_3d(
    world: &mut World,
    reference: Entity,
    cell: Entity,
    node: Option<Entity>,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let flags = world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08;
    if flags & 0x8000 != 0 && flags & 0x80000 != 0 && world.get::<Reference3D>(reference).is_some()
    {
        return Err(GameError::unclassified(
            0x0043FBA0,
            "0043FBA0: a reference's node detached",
        ));
    }
    let Some(base) = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
    else {
        return Err(GameError::unclassified(
            0x00441F0B,
            "00441F0B: a reference with no base faults",
        ));
    };
    let base_type = form_type(world, base);
    if flags & 0x820 == 0 && reference_sound_form(world, reference, base, flags).is_some() {
        return Err(GameError::unclassified(
            0x00441F4C,
            "00441F4C: a finished reference's sound (006AE4B0)",
        ));
    }
    if crate::tes::tes::reference_takes_clone(world, reference) {
        // 0044218E 0043F240 TES_RemoveActorFromProcessQueue: an actor's (vt+0x190) vt+0x1C4 at 0043F25F (005F23E0,
        // `actor_process_moves::actor_sync_process_level`), then 00674E10's removal from the manager's queue.
        if world.get::<crate::tes::actor::Actor>(reference).is_some() {
            crate::tes::actor_process_moves::actor_sync_process_level(world, reference)?;
            crate::tes::actors::actor_process_frame::process_manager_drop_queued(world, reference);
        }
        let node = match node {
            Some(node) => {
                if world.get::<Reference3D>(reference).is_none() {
                    world.entity_mut(reference).insert(Reference3D(node));
                }
                Some(node)
            }
            None => {
                let gated = world
                    .get_resource::<Load3DHostGates>()
                    .is_some_and(|gates| gates.gated.iter().any(|(gated, _)| *gated == reference));
                if gated {
                    None
                } else {
                    reference_load_3d(world, reference, services)?
                }
            }
        };
        // Build 2: an actor whose Load3D is behind a host gate stops here, with the gate's reason, rather than finish with no 3D
        // past the actor arms below.
        if node.is_none()
            && world.get::<crate::tes::actor::Actor>(reference).is_some()
            && let Some(&(_, why)) = world
                .get_resource::<Load3DHostGates>()
                .and_then(|gates| gates.gated.iter().find(|(gated, _)| *gated == reference))
        {
            return Err(GameError::from_legacy_message(why));
        }
        if let Some(node) = node {
            if base_type == 0x25 {
                return Err(GameError::unclassified(
                    0x004421D0,
                    "004421D0: a leveled creature's node removal",
                ));
            }
            super::tes_grid::cell_node(world, cell)?;
            crate::bs::scene_lighting::update_object_lights(world, node);
            world
                .get_resource_or_init::<super::light_3d::ShadowSceneDeferrals>()
                .object_walks
                .push(node);
            attach_to_cell(
                world,
                cell,
                reference,
                node,
                &services.prepare.property_defaults,
            )?;
            finish_attached_reference(world, reference, node)?;
            player_check_reference_overlap(world, reference, node)?;
            if base_type != 0x1A {
                crate::bs::scene_lighting::update_geometry_lights(world, node)?;
                world
                    .get_resource_or_init::<super::light_3d::ShadowSceneDeferrals>()
                    .geometry_walks
                    .push(node);
            }
        }
        crate::tes::forms_init_world::init_reference_script(world, reference)?;
        if !crate::tes::forms_references::save_load_active(world)? {
            crate::tes::forms_references::run_pending_script(world, reference)?;
            crate::tes::ai_package::script_event_or(world, reference, 0x1000, None);
        }
        if base_type == 0x1E {
            return Err(GameError::unclassified(
                0x00442264,
                "00442264: a tree's 004CE610",
            ));
        }
    }
    Ok(())
}

/// IDA: 004CEAD0(cell, ref) for a reference with a node that isn't an actor: a marker base (vt+0xF4) goes under the cell
/// node's child 1 (004417D0); otherwise child `2 + q` (q 0 for an interior, else `004C9BE0`'s quadrant) and its child 3
/// for BSX bit 0 or a tree, else 2 (00441800); there AttachChild(+0x84, node, 1), then `NiAVObject_RefreshPropertyState`
/// and `NiAVObject_RefreshEffectState` on the node. Completion: 100% of an interior.
fn attach_to_cell(
    world: &mut World,
    cell: Entity,
    reference: Entity,
    node: Entity,
    defaults: &crate::gamebryo::scene_state::ScenePropertyDefaults,
) -> Result<(), GameError> {
    use crate::gamebryo::scene_node::NiNodeChildren;
    let child_at = |world: &World, parent: Option<Entity>, index: usize| {
        parent
            .and_then(|parent| world.get::<NiNodeChildren>(parent))
            .and_then(|children| children.slots_ac.get(index).copied().flatten())
    };
    let root = world
        .get::<crate::tes::forms_world::CellSceneNode>(cell)
        .map(|node| node.0);
    let base = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c;
    let parent = if base.is_some_and(|base| bound_object_is_marker(world, base)) {
        child_at(world, root, 1)
    } else {
        if !crate::tes::forms_world::cell_is_interior(world, cell) {
            return Err(GameError::unclassified(
                0x004CEB4A,
                "004CEB4A: an exterior's quadrant (004C9BE0)",
            ));
        }
        let slot = if bsx_flag(world, node, 1)
            || base.is_some_and(|base| form_type(world, base) == 0x1E)
        {
            3
        } else {
            2
        };
        child_at(world, child_at(world, root, 2), slot)
    };
    let Some(parent) = parent else {
        return Ok(());
    };
    crate::bs::scene_graphs::attach_scene_child(world, parent, Some(node), true);
    crate::gamebryo::scene_state::refresh_property_state(world, node, defaults)?;
    crate::gamebryo::scene_state::refresh_effect_state(world, node)?;
    Ok(())
}

/// IDA: 004DC100(ref) for a reference with a node: the requeue arm needs no node or bit 0x80000 (a stop when it would run:
/// the reference is still in the +0x08 map here, so 004354F0 answers true and it doesn't); `TESObjectREFR_StartOrStopAttachedSound(1)`
/// (no sound form: nothing; one held or to create: a stop); `NiAVObject_Update(node, 0, 0)`; vt+0x148 at `004DC20E` (the C++ port
/// read it as vt+0xD0, nothing): `004E1FD0` for a reference that isn't an actor (`cell_havok`), `0065B750` for an actor;
/// a cell at process level 5 or 6 gives `00424F90` (extra 0x4B, saved Havok data: a stop). Completion: 100% of a non-actor.
fn finish_attached_reference(
    world: &mut World,
    reference: Entity,
    node: Entity,
) -> Result<(), GameError> {
    let flags = world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08;
    let base = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
        .ok_or(GameError::unclassified(
            0x004DC10A,
            "004DC10A: a reference with no base",
        ))?;
    if flags & 0x80000 != 0 {
        return Err(GameError::unclassified(
            0x004DC151,
            "004DC151: 004354F0 with bit 0x80000",
        ));
    }
    if flags & 0x800 == 0 {
        let held = world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44
            .get(0x5B)
            .is_some();
        if held || reference_sound_form(world, reference, base, flags).is_some() {
            return Err(GameError::unclassified(
                0x004DC1D2,
                "004DC1D2: 004D9310 with a sound",
            ));
        }
    }
    crate::gamebryo::scene_update::update_scene_object(
        world,
        node,
        0.0,
        false,
        &mut crate::gamebryo::scene_update::NoSceneControllers,
    )?;
    // 004DC20E vt+0x148: TESObjectREFR's 004E1FD0 puts the reference's collision into its cell's Havok world; the Actor
    // family's is 0065B750 Actor_CreateCharacterController (a MiddleLow or Low process returns at its level test).
    if world.get::<crate::tes::actor::Actor>(reference).is_some() {
        crate::tes::character_controller::actor_create_character_controller(
            world,
            reference,
            crate::tes::character_controller::ControllerInputs::default(),
        )?;
    } else {
        reference_add_to_havok_world(world, reference)?;
    }
    if let Some(cell) = crate::tes::forms_references::parent_cell(world, reference) {
        let level = world
            .get::<crate::tes::forms_world::TesObjectCell>(cell)
            .expect("a cell")
            .process_level_26;
        if (level == 5 || level == 6)
            && world
                .get::<TesObjectRefr>(reference)
                .expect("a reference")
                .extra_data_44
                .get(0x4B)
                .is_some()
        {
            return Err(GameError::unclassified(
                0x00424F90,
                "00424F90: saved Havok data",
            ));
        }
    }
    Ok(())
}

/// IDA: 004E3490's arm for a reference that isn't an actor whose node's first child has a +0x0C controller: vt+0x48
/// (0046AC30 AddChangeFlags 0x2000000); the controller cast to NiControllerManager (a head that isn't one does nothing more);
/// its +0x6C cumulative only warns. A door: with extra 0x4A the thread's save-load flag is set around it (a stop), then
/// `004DE460(open, 1)` with open = `004D6E60(flags) != (flags & 0x80000)` over its change flags (0045BCC0 with the +0x04
/// map first). "Idle" and "SpecialIdle" from the sequence map (004715A0): an Idle that loops (+0x24 zero) is activated
/// (00470B20: priority 0, no start-over, weight 1, no ease, no sync) with +0x48 = -FLT_MAX, a warning otherwise; with neither,
/// a non-door sets the manager's +0x08 bit 3, deactivates every active sequence (004715C0), activates the first with +0x48 =
/// -FLT_MAX and updates the node at its +0x2C begin time with controllers (0070xxxx, 1); then, door or not, 004715C0 and
/// bit 3 cleared. A SpecialIdle is activated likewise. The actor and extra 0x34 tails don't apply. Completion: 100%.
fn run_manager_idle(
    world: &mut World,
    reference: Entity,
    base_type: u8,
    node: Entity,
    head: Entity,
) -> Result<(), GameError> {
    use crate::gamebryo::controller_sequence::{ControllerManager, ControllerSequence};
    crate::tes::forms_init::record_form_change(world, reference, 0x0200_0000)?;
    if world.get::<crate::gamebryo::stream_objects::NiClass>(head)
        != Some(&crate::gamebryo::stream_objects::NiClass::ControllerManager)
    {
        return Ok(());
    }
    let door = base_type == 0x18;
    if door {
        if world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44
            .get(0x4A)
            .is_some()
        {
            return Err(GameError::unclassified(
                0x004E35F9,
                "004E35F9: a door's extra 0x4A (the save-load thread flag)",
            ));
        }
        let flags = change_flags(world, reference, true);
        let open = door_open(world, reference, flags) != (flags & 0x80000 != 0);
        door_set_open(world, reference, node, Some(head), open, true)?;
    }
    let manager = world
        .get::<ControllerManager>(head)
        .expect("a NiControllerManager");
    let idle = manager.find_sequence(b"Idle");
    let special = manager.find_sequence(b"SpecialIdle");
    let first = manager.sequence_slots_40.first().copied().flatten();
    match idle {
        Some(idle) => {
            if world
                .get::<ControllerSequence>(idle)
                .expect("a sequence")
                .cycle_type_24
                == 0
            {
                activate_from_start(world, idle);
            }
        }
        None if special.is_none() => {
            if !door {
                set_manager_flag_8(world, head, true);
                deactivate_all(world, head);
                let first = first.ok_or(GameError::unclassified(
                    0x004E366A,
                    "004E366A: a manager with no first sequence faults",
                ))?;
                activate_from_start(world, first);
                let begin = world
                    .get::<ControllerSequence>(first)
                    .expect("a sequence")
                    .begin_key_time_2c;
                update_with_controllers(world, node, begin)?;
            }
            deactivate_all(world, head);
            set_manager_flag_8(world, head, false);
            return Ok(());
        }
        None => {}
    }
    if let Some(special) = special {
        activate_from_start(world, special);
    }
    Ok(())
}

// IDA: the manager's NiTimeController +0x08 bit 3 (`|= 8`, `&= ~8`).
fn set_manager_flag_8(world: &mut World, manager: Entity, set: bool) {
    let mut state = world
        .get_mut::<crate::gamebryo::transform_controller::NodeController>(manager)
        .expect("a NiControllerManager has time state");
    if set {
        state.time.fields.flags |= 8;
    } else {
        state.time.fields.flags &= !8;
    }
}

/// IDA: N/A the particle systems a reference load's controller update (004E3490's no-idle arm, 004DE460) reached: the
/// original runs each one's controllers there (0047C930's chain, 00753BE0 / 00750260, emission drawing on the clone
/// thread's rand), which the host's particle simulation doesn't do yet. A recorded deferral: the systems hold no particles
/// from it, the rest of the update runs.
/// TODO: [ ] The particle simulation (`ParticleControllerScheduleContext`'s services) for these updates.
#[derive(Resource, Default, Debug)]
pub struct DeferredParticleUpdates {
    pub systems: Vec<(Entity, f32)>,
}

// IDA: N/A the runtime of a reference load's controller update: `update_node_controller`'s node controllers, and each particle
// system's chain recorded in `DeferredParticleUpdates` with the pass going on.
pub(crate) struct LoadUpdateControllers;

impl crate::gamebryo::scene_update::SceneUpdateRuntime for LoadUpdateControllers {
    /// IDA: N/A `LoadUpdateControllers`'s side of `SceneUpdateRuntime`.
    fn update_controller(
        &mut self,
        world: &mut World,
        controller: Entity,
        time: f32,
    ) -> Result<crate::gamebryo::scene_update::SceneControllerStep, GameError> {
        crate::gamebryo::scene_update::update_node_controller(world, controller, time)
    }

    /// IDA: N/A `LoadUpdateControllers`'s side of `SceneUpdateRuntime`.
    fn run_particle_controllers(
        &mut self,
        world: &mut World,
        _head: Option<Entity>,
        system: Entity,
        time: f32,
    ) -> Result<
        crate::gamebryo::particle_controller_schedule::ParticleControllerScheduleResult,
        GameError,
    > {
        world
            .get_resource_or_init::<DeferredParticleUpdates>()
            .systems
            .push((system, time));
        Ok(crate::gamebryo::particle_controller_schedule::ParticleControllerScheduleResult {
            status: crate::gamebryo::particle_controller_schedule::ParticleControllerScheduleStatus::Completed,
            visited: 0,
            last_emitter_status: None,
            last_reset_status: None,
            last_modifier_value_status: None,
        })
    }
}

// IDA: NiAVObject_Update(node, time, 1) with the controllers.
pub(crate) fn update_with_controllers(
    world: &mut World,
    node: Entity,
    time: f32,
) -> Result<(), GameError> {
    let result = crate::gamebryo::scene_update::update_scene_object(
        world,
        node,
        time,
        true,
        &mut LoadUpdateControllers,
    )?;
    if result.status != crate::gamebryo::scene_update::SceneUpdateStatus::Updated {
        return Err(GameError::unclassified(
            0x004DE5A5,
            "004DE5A5: a load's controller update stopped",
        ));
    }
    Ok(())
}

// IDA: 004D6E60(flags): a zero `flags` reads 0045BCC0(form, 0); bit 0x40000 inverts 0041F830(8).
fn door_open(world: &World, reference: Entity, flags: u32) -> bool {
    let flags = if flags == 0 {
        change_flags(world, reference, false)
    } else {
        flags
    };
    // 0041F830(list, 8): the extra 0x13 byte's bit 3, or bit 3 of 1 (false) with none.
    let list = &world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44;
    crate::tes::base_extra_list::action_flag(list, 8) != (flags & 0x40000 != 0)
}

/// IDA: 004DE460(open, play) for a door: `004D8270(4)` to open or `004D82E0(4)` to close (each: 0045BCC0's +0x00 flags,
/// bit 0x40000 inverting 0041F830(8); the change flag 0x80000 cleared (vt+0x44) or set (vt+0x40) to match, then 00423DF0 or
/// 00423E50 with 4 on the extra 0x13 byte); the node's first child's +0x0C NiControllerManager: with both "Open" and "Close",
/// bit 3 on the manager, the other sequence deactivated, and with `play` activated (+0x48 = -FLT_MAX), the node updated
/// with controllers at the chosen sequence's +0x2C, the other deactivated again, `004D90D0` with the chosen name and
/// `NiAVObject_MarkCollisionForSync(node, 1)`; without `play` the chosen one activated and the node updated at its +0x2C.
/// With no manager, `004D90D0("Open")` or `("Close")`. Completion: 100%.
fn door_set_open(
    world: &mut World,
    reference: Entity,
    node: Entity,
    manager: Option<Entity>,
    open: bool,
    play: bool,
) -> Result<(), GameError> {
    use crate::gamebryo::controller_sequence::{
        ControllerManager, ControllerSequence, deactivate_sequence,
    };
    let flags = change_flags(world, reference, false);
    let list = &world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44;
    let state = crate::tes::base_extra_list::action_flag(list, 8) != (flags & 0x40000 != 0);
    let clear_80000 = state == open;
    if clear_80000 {
        crate::tes::forms_items::clear_change_flags(world, reference, 0x80000)?;
    } else {
        crate::tes::forms_items::set_change_flags(world, reference, 0x80000)?;
    }
    {
        let mut refr = world
            .get_mut::<TesObjectRefr>(reference)
            .expect("a reference");
        if open {
            refr.extra_data_44.or_action_flags(4);
        } else {
            refr.extra_data_44.clear_action_flags(4);
        }
    }
    let Some(manager) = manager.filter(|&manager| {
        world.get::<crate::gamebryo::stream_objects::NiClass>(manager)
            == Some(&crate::gamebryo::stream_objects::NiClass::ControllerManager)
    }) else {
        set_last_finished_sequence(world, reference, if open { b"Open" } else { b"Close" })?;
        return Ok(());
    };
    let state = world
        .get::<ControllerManager>(manager)
        .expect("a NiControllerManager");
    let (Some(open_sequence), Some(close_sequence)) =
        (state.find_sequence(b"Open"), state.find_sequence(b"Close"))
    else {
        return Ok(());
    };
    let (chosen, other) = if open {
        (open_sequence, close_sequence)
    } else {
        (close_sequence, open_sequence)
    };
    set_manager_flag_8(world, manager, true);
    deactivate_sequence(world, other, 0.0, false);
    if play {
        activate_from_start(world, other);
        let begin = world
            .get::<ControllerSequence>(chosen)
            .expect("a sequence")
            .begin_key_time_2c;
        update_with_controllers(world, node, begin)?;
        deactivate_sequence(world, other, 0.0, false);
        let name = world
            .get::<ControllerSequence>(chosen)
            .expect("a sequence")
            .name_08
            .clone()
            .unwrap_or_default();
        set_last_finished_sequence(world, reference, &name)?;
        mark_collision_for_sync(world, node);
    } else {
        activate_from_start(world, chosen);
        let begin = world
            .get::<ControllerSequence>(chosen)
            .expect("a sequence")
            .begin_key_time_2c;
        update_with_controllers(world, node, begin)?;
    }
    Ok(())
}

/// IDA: 004D90D0(name): with no save loading (or one with no extra 0x4A), 00424DE0: extra 0x4A removed, then a new
/// ExtraLastFinishedSequence (0042A720) with a copy of a nonnull name. Completion: 100%.
fn set_last_finished_sequence(
    world: &mut World,
    reference: Entity,
    name: &[u8],
) -> Result<(), GameError> {
    let loading = crate::tes::forms_references::save_load_active(world)?;
    let mut refr = world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference");
    if loading && refr.extra_data_44.get(0x4A).is_some() {
        return Ok(());
    }
    refr.extra_data_44.remove_entry(0x4A);
    let end = name
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(name.len());
    refr.extra_data_44
        .add(crate::tes::base_extra_list::new_extra_entry(
            0x4A,
            crate::tes::base_extra_list::ExtraData::LastFinishedSequence(name[..end].to_vec()),
        ));
    Ok(())
}

/// IDA: 004DE100 TESObjectREFR_ApplyRagdollPose(null) and 00497830 RagDollData_ApplyToReference.
/// Completion: Load3D's extra 19, ordered pose application and mismatched-part fallback.
/// TODO: [ ] The optional supplied pose with no scene node.
pub(crate) fn restore_reference_pose(
    world: &mut World,
    reference: Entity,
    root: Entity,
) -> Result<(), GameError> {
    use crate::tes::base_extra_list::{EXTRA_RAG_DOLL, ExtraData};
    let Some(ExtraData::RagDoll(data)) = world
        .get::<crate::tes::actor::TesObjectRefr>(reference)
        .and_then(|r| r.extra_data_44.get(EXTRA_RAG_DOLL))
        .map(|extra| &extra.data)
    else {
        return Ok(());
    };
    let count = usize::from(data.count_00);
    if count == 0 {
        return Ok(());
    }
    let poses = data
        .data_04
        .as_deref()
        .and_then(|v| v.get(..count * 28))
        .ok_or(GameError::unclassified(
            0x004976A8,
            "truncated saved ragdoll poses",
        ))?
        .to_vec();
    let mut index = 0;
    if !apply_node_poses(world, root, &poses, &mut index)? {
        crate::bs::havok_world::set_motion_type(world, Some(root), 1, true, false)?;
    }
    Ok(())
}

// IDA: N/A the skin info a PlayerCharacter_GetSkinInfo(view) answers: the third person's (Character +0x104) for 1, the first
// person's (+0x5C8) for 0.
fn player_skin_info(world: &World, player: Entity, third_person: bool) -> Option<Entity> {
    if third_person {
        world.get::<Character>(player).and_then(|c| c.skin_info_104)
    } else {
        world
            .get::<crate::tes::actors::player::PlayerCharacter>(player)
            .and_then(|p| p.first_person_skin_info_5c8)
    }
}

// IDA: N/A the player's +0x588 view byte (1: third person).
fn third_person(world: &World, player: Entity) -> bool {
    world
        .get::<crate::tes::actors::player::PlayerCharacter>(player)
        .is_some_and(|p| p.view_flags_588[0] != 0)
}

// IDA: N/A the teardowns' shared shape for the player (004DC5B0, 004DC8F0, 004DCAB0, 004DCCF0): the view's skin info
// (vt+0x168) takes `release`, then PlayerCharacter_GetSkinInfo(+0x588)'s; with a skin info the equipped-item and process-node
// arms are skipped.
fn release_player_slots(
    world: &mut World,
    player: Entity,
    release: &mut dyn FnMut(&mut World, Entity) -> Result<(), GameError>,
) -> Result<Entity, GameError> {
    if let Some(skin) = crate::tes::actors::equipment_3d::view_skin_info(world, player) {
        release(world, skin)?;
    }
    let skin = player_skin_info(world, player, third_person(world, player)).ok_or(
        GameError::unclassified(
            0x004DC5E8,
            "004DC5E8: the player's teardown with no skin info (the process and item arms)",
        ),
    )?;
    release(world, skin)?;
    Ok(skin)
}

/// IDA: 004E0F80 TESObjectREFR_Set3D(player, null) for the race menu's player with 3D: the node detached from its parent
/// (vt+0x88, the old transform kept for a new node, here none); `007C5E70` and `007C76D0` (counted); `004D7470` (a furniture
/// base's, not an NPC's); a living actor takes no save marks (`0045D220`); a light extra (0x30) is a stop; vt+0x44(0x2000000)
/// and, at process level 1 or below, MiddleHighProcess_SetAnimData(null) (`set_actor_anim_data`); `004DC5B0` (the weapon,
/// `00478CE0`), `004DC8F0`(0) (the shield, `00478DD0`; current action 6 a stop), `004DCAB0` (the torch, `00478E30` with
/// PlayerCharacter_SetViewLight(null)) and `004DCCF0` (the quiver, `00478DB0`), each then Actor_UpdateSkinProperty
/// (`005EA1A0`) where the exe calls it; vt+0x16C(0) (`00611B00` Character_SetSkinInfo: SkinInfo_ReleaseAllSlots `004796F0`,
/// the skin info freed); BSX bit 0x10 (`004DE1C0`) a stop; with a character controller `004D9A50` a stop; the process's
/// vt+0x470(0) (+0x188 = 0); a sound manager's `006AC420` (its +0x304 map, which the port never fills, so nothing); outside a cell at +0x26 1, ActorProcessManager +0x48's
/// release (empty, or a stop) and `00B3B7D0`; the node's "REF" extra removed; vt+0x144 (`0065AB80`: the controller and the node's
/// bodies out of the Havok world); +0x3C null; the base's
/// vt+0xF0 (`004B3210`: the model map's entry for its model, at scale 1.0); `004D9310`(0) for an NPC base (nothing);
/// `004D7D10`(null); a teleport extra outside a load is a stop. Answers the old node's parent. Completion: 100% of the race
/// menu's player.
/// # Errors
/// A stop above.
#[allow(clippy::too_many_lines)] // 004E0F80's own length.
pub fn player_set_3d_null(world: &mut World, player: Entity) -> Result<Option<Entity>, GameError> {
    let Some(node) = world.get::<Reference3D>(player).map(|loaded| loaded.0) else {
        return Err(GameError::unclassified(
            0x004E0FC3,
            "004E0FC3: Set3D(null) with no 3D (ModelLoader_CancelReference)",
        ));
    };
    let parent = world.get::<NiAvParent>(node).and_then(|parent| parent.0);
    if let Some(parent) = parent {
        crate::gamebryo::scene_node::detach_child(world, parent, node);
    }
    crate::tes::actors::equipment_3d::count(world, 0x007C_5E70);
    crate::tes::actors::equipment_3d::count(world, 0x007C_76D0);
    if crate::tes::actor::actor_is_dead(world, player, false) {
        return Err(GameError::unclassified(
            0x004E117B,
            "004E117B: a dead actor's save marks (0045D220)",
        ));
    }
    let extras = &world
        .get::<TesObjectRefr>(player)
        .expect("a reference")
        .extra_data_44;
    if extras
        .get(crate::tes::base_extra_list::EXTRA_LIGHT)
        .is_some()
    {
        return Err(GameError::unclassified(
            0x004E11AA,
            "004E11AA: the player's light extra (007C7DC0)",
        ));
    }
    set_actor_anim_data(world, player, None)?;
    // 004E125F 004DC5B0: the weapon.
    let skin = release_player_slots(world, player, &mut |world, skin| {
        release_weapon(world, skin)
    })?;
    let process = crate::tes::actor::process_of(world, player).ok_or(GameError::unclassified(
        0x004DC6A0,
        "004DC6A0: the player with no process faults",
    ))?;
    crate::tes::actors::actor_enable::find_skin_property(world, process, node)?;
    // 004E1268 004DC8F0(0): the shield.
    let action = world
        .get::<crate::tes::actors::actor_process::HighProcess>(process)
        .map_or(0xFFFF, |high| high.current_action_1f4);
    if action == 6 {
        return Err(GameError::unclassified(
            0x004DC94C,
            "004DC94C: the shield's teardown during action 6",
        ));
    }
    release_player_slots(world, player, &mut |world, skin| {
        release_skin_slot(world, skin, SkinSlotAt::Skin(13), true, None)
    })?;
    crate::tes::actors::actor_enable::find_skin_property(world, process, node)?;
    // 004E126F 004DCAB0: the torch (slot 14, a LIGH form only), then SetViewLight(null) for the player's skin info.
    release_player_slots(world, player, &mut |world, skin| {
        let form = world.get::<SkinInfo>(skin).expect("a skin info").slots_4c[14].form_00;
        if form.is_some_and(|form| crate::tes::forms_init::form_type(world, form) == 0x1A) {
            release_skin_slot(world, skin, SkinSlotAt::Skin(14), true, None)?;
        }
        let first = world
            .get::<crate::tes::actors::player::PlayerCharacter>(player)
            .is_some_and(|p| p.first_person_skin_info_5c8 == Some(skin));
        let mut data = world
            .get_mut::<crate::tes::actors::player::PlayerCharacter>(player)
            .expect("the player");
        if first {
            data.first_person_light_798 = 0;
        } else {
            data.third_person_light_79c = 0;
        }
        Ok(())
    })?;
    // 004E1276 004DCCF0: the quiver.
    release_player_slots(world, player, &mut |world, skin| {
        release_skin_slot(world, skin, SkinSlotAt::Skin(12), true, None)
    })?;
    crate::tes::actors::actor_enable::find_skin_property(world, process, node)?;
    let _ = skin;
    // 004E1288 vt+0x16C(0) 00611B00: every slot released (004796F0), then the skin info freed.
    if let Some(skin) = world.get::<Character>(player).and_then(|c| c.skin_info_104) {
        for slot in 0..16 {
            release_skin_slot(world, skin, SkinSlotAt::Skin(slot), true, None)?;
        }
        world.entity_mut(skin).despawn();
        world
            .get_mut::<Character>(player)
            .expect("a character")
            .skin_info_104 = None;
    }
    if bsx_flag(world, node, 0x10) {
        return Err(GameError::unclassified(
            0x004E12A6,
            "004E12A6: 004DE1C0, the FlameNodes' off walk",
        ));
    }
    // 004E12C1 004D9A50(null) on the character controller: 0089F650(+0x364, null, rtti_1), the phantom's 3D property removed.
    if let Some(controller) =
        crate::tes::actors::character_controller::actor_character_controller(world, player)
        && let Some(phantom) = world
            .get::<crate::bs::character_controller::BhkCharacterController>(controller)
            .and_then(|c| c.phantom_364)
    {
        crate::bs::character_controller::bind_phantom_3d(world, phantom, None);
    }
    // 004E12D7 the process's vt+0x470(0) (00629590: +0x188).
    if let Some(mut middle) =
        world.get_mut::<crate::tes::actors::actor_process::MiddleHighProcess>(process)
    {
        middle.bound_188 = None;
    }
    // 004E12ED 006AC420(player, 0.0) with a sound manager: the +0x304 map's sounds attached to the player's node stopped
    // (SoundInstance_StopAndRewind for a fade below 0.1) and removed. Nothing the port runs attaches a sound to a node, so the
    // map is empty and nothing is stopped.
    let cell = world
        .get::<TesObjectRefr>(player)
        .and_then(|refr| refr.parent_cell_40);
    let attached = cell
        .and_then(|cell| world.get::<crate::tes::forms_world::TesObjectCell>(cell))
        .is_some_and(|data| data.process_level_26 == 1);
    if !attached {
        let manager = &world
            .resource::<crate::tes::actors::actor_process::ActorProcessGlobals>()
            .manager;
        if manager.pointer_48 != [0, 0] {
            return Err(GameError::unclassified(
                0x004E1305,
                "004E1305: ActorProcessManager_ReleaseListForActor's +0x48 list",
            ));
        }
    }
    // 004E131E: the node's "REF" extra removed.
    let extras = world
        .get::<NiObjectNet>(node)
        .map(|net| net.extra_data.clone())
        .unwrap_or_default();
    let keep: Vec<Entity> = extras
        .into_iter()
        .filter(|&extra| {
            world
                .get::<crate::gamebryo::object_net::NiExtraDataName>(extra)
                .is_none_or(|name| name.0.as_deref() != Some(b"REF".as_slice()))
        })
        .collect();
    if let Some(mut net) = world.get_mut::<NiObjectNet>(node) {
        net.extra_data = keep;
    }
    // 004E1355 vt+0x144 (0065AB80) with the node still held.
    player_remove_from_world(world, player, Some(node))?;
    world.entity_mut(player).remove::<Reference3D>();
    // 004E13B7 the base's vt+0xF0 (004B3210): the model map's entry for its model path, at the player's scale 1.0.
    let base = world
        .get::<TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
        .ok_or(GameError::unclassified(
            0x004E13A6,
            "004E13A6: the player with no base",
        ))?;
    let path = crate::tes::reference_load::model_path_of(world, base).unwrap_or_default();
    let save_flags = world
        .get_resource::<crate::tes::tes::TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map(|game| game.fields_14[1]);
    {
        let mut tasks = world.resource_mut::<crate::bs::task_manager::TaskGlobals>();
        let loader = tasks
            .model_loader_b33a1c
            .as_mut()
            .ok_or(GameError::unclassified(
                0x004B3249,
                "004B3249: a null g_modelLoader",
            ))?;
        crate::bs::model_loader::lookup_and_release_model(loader, &path, true, save_flags)?;
    }
    // 004E13CF 004D9310(0): an NPC base has no attached sound. 004E13DB 004D7D10(null): vt+0x144 again (+0x3C now null),
    // then the null store.
    player_remove_from_world(world, player, None)?;
    world
        .get_mut::<TesObjectRefr>(player)
        .expect("a reference")
        .loaded_3d_3c = 0;
    if world
        .get::<TesObjectRefr>(player)
        .expect("a reference")
        .extra_data_44
        .get(crate::tes::base_extra_list::EXTRA_TELEPORT)
        .is_some()
        && !crate::tes::forms_references::save_load_active(world)?
    {
        return Err(GameError::unclassified(
            0x004E152A,
            "004E152A: the teleport's middle-low count (TESObjectCELL_ChangeProcessMiddleLowCount)",
        ));
    }
    Ok(parent)
}

/// IDA: 004B2A00 TESBoundObject_IsMarker, the bound objects' vt+0xF4 (is a marker): an activator (0x12) whose model is `EditorMarker.NIF` (0046D3F0, `_stricmp`); the prison marker door (0x18); a light (0x1A) with no model path (0046DA90); the door, travel, temple, divine, north, map, horse, X and XMarkerHeading statics (0x1C); a subspace (0x29); otherwise a form ID from 100 to 119. Completion: 100% of the TESModel arm; 004694A0's reference arm isn't reached from a base.
#[must_use]
pub fn bound_object_is_marker(world: &World, base: Entity) -> bool {
    let statics = world.resource::<DefaultFormStatics>();
    let is = |slot: Option<Entity>| slot == Some(base);
    let default_object = || form_id(world, base).wrapping_sub(100) <= 0x13;
    match form_type(world, base) {
        0x12 => {
            let path = model_path(world, base).unwrap_or_default();
            stricmp(&path, b"EditorMarker.NIF") == 0 || default_object()
        }
        0x18 => is(statics.prison_marker_b35ebc) || default_object(),
        0x1A => model_path(world, base).is_none_or(|path| path.is_empty()) || default_object(),
        0x1C => {
            [
                statics.door_marker_b35ea4,
                statics.travel_marker_b35eb4,
                statics.temple_marker_b35ec0,
                statics.divine_marker_b35ec4,
                statics.north_marker_b35eb8,
                statics.map_marker_b35ea8,
                statics.horse_marker_b35ed4,
                statics.x_marker_b35eac,
                statics.x_marker_heading_b35eb0,
            ]
            .into_iter()
            .any(is)
                || default_object()
        }
        0x29 => true,
        _ => default_object(),
    }
}

// IDA: 004DDF90 TESObjectREFR_GetFaceGenNiNodeBiped and 004DDFD0 TESObjectREFR_GetFaceGenNiNodeSkinned (the player's vt+0x130
// and vt+0x134): an actor whose vt+0x27C answers nonzero looks the node up under +0x3C (006F94A0).
pub(crate) fn player_face_node(cx: &InterfaceCx, player: Entity, name: &[u8]) -> Option<Entity> {
    cx.world.get::<Reference3D>(player).and_then(|loaded| {
        crate::gamebryo::object_net::object_by_name_with_palette(cx.world, loaded.0, name)
    })
}

/// IDA: `004E1FD0` TESObjectREFR_AddToHavokWorld, vt+0x148 for a reference that isn't an actor: a parent cell not at level
/// 5 or 6 returns. vt+0x154's node; none returns. An NiBSLODNode (none loads in the port) takes its child 0. Otherwise, not
/// an actor, a node with no collision object of its own whose child 0 is nonnull takes child 0, or child 1 when child 0 has
/// none either and child 1 is nonnull. With a root, the parent cell's world (none returns) gets vt+0x94 (`0088C740`) of
/// (root, node); then the +0x08 bit 0x100 runs `0088CF90`(node, 1, 1, 1) (a stop) and clears; then `004D8F20`. Without a
/// root, a BSTreeNode's arm (`004440C0`'s world vt+0x90) is a stop. Completion: 100% of the arms the prison reaches.
/// # Errors
/// A stop above, or under `0088C740`.
pub fn reference_add_to_havok_world(world: &mut World, reference: Entity) -> Result<(), GameError> {
    use crate::gamebryo::scene_node::NiNodeChildren;
    let cell = world
        .get::<crate::tes::actor::TesObjectRefr>(reference)
        .expect("a reference")
        .parent_cell_40;
    if let Some(cell) = cell {
        let level = world
            .get::<TesObjectCell>(cell)
            .expect("a cell")
            .process_level_26;
        if level != 6 && level != 5 {
            return Ok(());
        }
    }
    let Some(node) = world.get::<Reference3D>(reference).map(|loaded| loaded.0) else {
        return Ok(());
    };
    let child = |world: &World, index: usize| {
        world
            .get::<NiNodeChildren>(node)
            .and_then(|children| children.slots_ac.get(index).copied().flatten())
    };
    let count = world
        .get::<NiNodeChildren>(node)
        .map_or(0, |children| children.slots_ac.len());
    let mut root = Some(node);
    if world.get::<crate::tes::actor::Actor>(reference).is_none()
        && crate::bs::havok_world::collision_object(world, node).is_none()
        && count != 0
        && let Some(first) = child(world, 0)
    {
        root = Some(first);
        if crate::bs::havok_world::collision_object(world, first).is_none()
            && count > 1
            && let Some(second) = child(world, 1)
        {
            root = Some(second);
        }
    }
    let Some(cell) = cell else {
        return Ok(());
    };
    let bhk = if cell_is_interior(world, cell) {
        cell_bhk_world(world, cell)
    } else if world
        .get_resource::<HavokGlobals>()
        .is_some_and(|globals| globals.world_b35c24.is_some())
    {
        return Err(GameError::unclassified(
            0x004E20CB,
            "004E20CB: the exterior world's vt+0x94",
        ));
    } else {
        None
    };
    let Some(bhk) = bhk else {
        return Ok(());
    };
    crate::bs::havok_world::bhk_world_add_reference_3d(world, bhk, root, Some(node))?;
    let flags = world
        .get::<crate::tes::form::FormIdentity>(reference)
        .expect("a form")
        .flags_08;
    if flags & 0x100 != 0 {
        return Err(GameError::unclassified(
            0x004E20F6,
            "004E20F6: 0088CF90 on a reference with +0x08 bit 0x100",
        ));
    }
    reference_note_havok_3d(world, reference, node)?;
    Ok(())
}

/// IDA: `004D8F20`(node): no node, no base (vt+0x170), or a base of type 0x18 answers 0; a base of type 0x1C needs
/// `005368B0`, any other the node's BSX bit 8; then vt+0x48(8), the change flags. Completion: 100%.
/// # Errors
/// A stop in the change flags.
fn reference_note_havok_3d(
    world: &mut World,
    reference: Entity,
    node: Entity,
) -> Result<bool, GameError> {
    let Some(base) = world
        .get::<crate::tes::actor::TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
    else {
        return Ok(false);
    };
    let kind = crate::tes::forms_init::form_type(world, base);
    if kind == 0x18 {
        return Ok(false);
    }
    let wanted = if kind == 0x1C {
        tree_has_moving_body(world, Some(node))
    } else {
        bsx_flag(world, node, 8)
    };
    if !wanted {
        return Ok(false);
    }
    crate::tes::forms_items::set_change_flags(world, reference, 8)?;
    Ok(true)
}

// IDA: 004DC8F0's and 004E1B40's first test: no node (+0x3C) returns; a node is a stop here.
pub(crate) fn no_node_or_stop(
    world: &World,
    actor: Entity,
    stop: &'static str,
) -> Result<(), GameError> {
    if world.get::<Reference3D>(actor).is_some() {
        return Err(GameError::from_legacy_message(stop));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `verify_classifier`: 004E0CF0 categories.
    #[test]
    fn verify_classifier() {
        assert_eq!(classify_reference_base_form_type(18), 1);
        assert_eq!(classify_reference_base_form_type(32), 2);
        assert_eq!(classify_reference_base_form_type(35), 3);
        assert_eq!(classify_reference_base_form_type(33), 4);
        assert_eq!(classify_reference_base_form_type(39), 5);
        assert_eq!(classify_reference_base_form_type(19), 6);
    }
}
