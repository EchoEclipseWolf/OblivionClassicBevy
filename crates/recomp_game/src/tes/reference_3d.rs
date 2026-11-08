//! A reference's 3D: the base's vt+0x114 (`004B2320` → `004B3750`), which takes the Model's root or clones it.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::actor::TesObjectRefr;
use super::default_forms::DefaultFormStatics;
use super::form::FormIdentity;
use super::forms_init::form_type;
use crate::bs::model_loader::ModelBuildServices;
use crate::bs::model_loader::fixup_model_path;
use crate::bs::task_manager::TaskGlobals;
use crate::gamebryo::av_object::NiAvParent;
use crate::gamebryo::clone::{CloningProcess, clone_object};
use crate::gamebryo::object_net::{NiExtraDataName, NiObjectNet};
use crate::gamebryo::scene_node::SceneNodePayload;
use crate::gamebryo::transform::NiTransform;

/// IDA: TESObjectREFR +0x3C, the reference's NiNode (`TesObjectRefr::loaded_3d_3c` holds 1 while it is set).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference3D(pub Entity);

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
) -> Result<Vec<u8>, StreamError> {
    let mut key =
        super::reference_load::model_for_reference(world, base, reference)?.unwrap_or_default();
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
) -> Result<Option<Entity>, StreamError> {
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
        Some(reference) => super::reference_load::model_for_reference(world, base, reference)?,
        None => super::reference_load::model_path_of(world, base),
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
        .is_some_and(|base| super::reference_load::bound_object_is_marker(world, base));
    let stage = super::reference_load::model_stage(form_type(world, base));
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
) -> Result<Option<Entity>, StreamError> {
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
        .ok_or(StreamError("004B37BB: a model root that isn't a NiNode"))?
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
                    return Err(StreamError("004B39E0: the a3 clone arm (004430C0)"));
                }
                true
            }
            // 004B38E2 00468FF0(slot 13, 0): +0x04's bit 13 on the armor's TESBipedModelForm (+0x64).
            0x14 if world
                .get::<super::form_components::TesBipedModelForm>(base)
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
            let reference =
                reference.ok_or(StreamError("004B3ABF: a scaled clone with no reference"))?;
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
fn set_geometry_classifier(
    world: &mut World,
    object: Entity,
    value: i32,
) -> Result<(), StreamError> {
    if matches!(
        world.get::<crate::gamebryo::stream_objects::NiClass>(object),
        Some(
            crate::gamebryo::stream_objects::NiClass::TriShape
                | crate::gamebryo::stream_objects::NiClass::TriStrips
        )
    ) {
        let property = crate::bs::sky_shader::find_property_of_type(world, object, 4)?;
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
) -> Result<Option<Entity>, StreamError> {
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
    if Some(reference) == super::ai_conditions::the_player(world) {
        return super::player_3d::player_load_3d(world, reference, services);
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
) -> Result<Option<Entity>, StreamError> {
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
    let actor = world.get::<super::actor::Actor>(reference).is_some();
    let base_type = form_type(world, base);
    if actor && !matches!(base_type, 0x23 | 0x24) {
        return Err(StreamError(
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
            return Err(StreamError(
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
                .push((reference, error.0));
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
            .get::<super::forms_items::TesDoor>(base)
            .is_some_and(|door| door.flags_64 & 2 != 0);
        if automatic && flags & 0x2000 == 0 {
            return Err(StreamError(
                "004B7FB5: an automatic door's controller (0060E0A0)",
            ));
        }
        if world
            .get::<NiObjectNet>(node)
            .is_some_and(|net| net.controller_0c.is_some())
        {
            return Err(StreamError(
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
            .ok_or(StreamError("004E412E: an actor node with no NiObjectNET"))?
            .name = Some(name);
    }
    // 004E41C0..004E41F8: BSX bit 0x10 walks the FlameNodes, off (004DE1C0) for +0x08 bit 0x80, else on (004E26F0).
    if bsx_flag(world, node, 0x10) {
        if flags & 0x80 != 0 {
            return Err(StreamError("004E41F0: 004DE1C0, the FlameNodes' off walk"));
        }
        match flame_nodes_on(world, node, services) {
            Ok(_) => {}
            Err(error) if is_clone_gap(&error) => {
                world
                    .get_resource_or_init::<Load3DHostGates>()
                    .gated
                    .push((reference, error.0));
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
    }
    if base_type == 0x21 {
        detach_scabbard(world, node);
    }
    let (position, rotation, scale) = {
        let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
        (refr.position_2c, refr.rotation_20, refr.scale_38)
    };
    {
        let mut payload = world.get_mut::<SceneNodePayload>(node).expect("a NiNode");
        payload.local_transform.translation = position;
        payload.local_transform.rotation = super::forms_world::subspace_rotation_matrix(rotation);
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
        return Err(StreamError("004E41F7: 004D6F20's Havok arm (0088CEB0)"));
    }
    mark_collision_for_sync(world, node);
    attach_reference_extra(world, node, reference)?;
    let classifier = classify_reference(world, node);
    set_geometry_classifier(world, node, classifier)?;
    let statics = world.resource::<DefaultFormStatics>();
    if Some(base) == statics.door_marker_b35ea4 || Some(base) == statics.travel_marker_b35eb4 {
        world
            .get_mut::<SceneNodePayload>(node)
            .expect("a NiNode")
            .effective_av_flags |= 1;
    }
    // 004E438D: a teleport whose linked door (0042B460) has a parent cell counts that cell up (004CCA60 with 1).
    if let Some(cell) = super::ai_conditions::teleport_door(world, reference)
        .and_then(|door| super::ai_package::parent_cell(world, door))
    {
        cell_count_process_middle_low(world, cell)?;
    }
    let extras = &world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44;
    if extras.get(0x4B).is_some() || extras.get(0x4A).is_some() {
        return Err(StreamError(
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
        return Err(StreamError("004D83F6: a reference's extra 0x15 removed"));
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
            return Err(StreamError("004E3A4C: the node's vt+0x60 update stopped"));
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
    Ok(Some(node))
}

// IDA: 00521A40 TESNPC vt+0x114(reference): the reference's skin info (vt+0x168), 004B3750 TESBoundObject_TakeOrCloneModel3D
// (an NPC always clones), the clone's "Bip01" (006F94A0); with a skin info and a Bip01, 00478070 SkinInfo_FindBones; with none and
// a Bip01, a new 0x154 skin info stored by vt+0x16C, a stop here (005268D0 made one for every NPC that gets here); with a Bip01,
// NiAVObject_RefreshPropertyState, RefreshEffectState and NiAVObject_Update(0, 0). Completion: 100% with a skin info.
fn npc_create_3d(
    world: &mut World,
    base: Entity,
    reference: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, StreamError> {
    let skin = super::character_3d::skin_info(world, reference);
    let Some(root) = bound_object_create_3d(world, base, Some(reference), services)? else {
        return Ok(None);
    };
    if crate::bs::model_loader::object_by_name(world, root, b"Bip01").is_none() {
        return Ok(Some(root));
    }
    let Some(skin) = skin else {
        return Err(StreamError("00521A8F: an NPC's 3D with no skin info"));
    };
    super::character_3d::skin_info_find_bones(world, skin, root)?;
    crate::gamebryo::scene_state::refresh_property_state(
        world,
        root,
        &services.prepare.property_defaults,
    )?;
    crate::gamebryo::scene_state::refresh_effect_state(world, root)?;
    let result = crate::gamebryo::scene_update::update_scene_object(
        world,
        root,
        0.0,
        false,
        &mut crate::gamebryo::scene_update::NoSceneControllers,
    )?;
    if result.status != crate::gamebryo::scene_update::SceneUpdateStatus::Updated {
        return Err(StreamError("00521AD9: the NPC's NiAVObject_Update stopped"));
    }
    Ok(Some(root))
}

/// IDA: 004D83B0 TESObjectREFR_SetAnimData(reference, anim) for an actor (vt+0x188): vt+0x48 (AddChangeFlags) or, for null, vt+0x44 (ClearChangeFlags) with 0x2000000; a process of level 0 or 1 takes it (0064FFF0: an older one, unless the same, is destroyed by 00475B60 and freed; then +0x17C). A higher level's extra 0x15 (0041E630, 0041E9C0) is a stop. Answers `anim`. Completion: 100% of an actor with a high process.
/// # Errors
/// As above.
pub fn set_actor_anim_data(
    world: &mut World,
    reference: Entity,
    anim: Option<Entity>,
) -> Result<Option<Entity>, StreamError> {
    if anim.is_some() {
        super::forms_init::record_form_change(world, reference, 0x0200_0000)?;
    } else {
        super::forms_items::clear_change_flags(world, reference, 0x0200_0000)?;
    }
    let process = super::ai_conditions::process_of(world, reference);
    if let Some(process) = process
        && super::actor_process::process_level(world, process) <= 1
    {
        let mut state = world
            .get_mut::<super::actor_process::MiddleHighProcess>(process)
            .ok_or(StreamError(
                "0064FFF0: a level 0 or 1 process with no MiddleHigh part",
            ))?;
        if let Some(old) = state.anim_data_17c {
            if Some(old) == anim {
                return Ok(anim);
            }
            return Err(StreamError(
                "0065000D: 00475B60, an older ActorAnimData destroyed",
            ));
        }
        state.anim_data_17c = anim;
        return Ok(anim);
    }
    Err(StreamError(
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
) -> Result<(), StreamError> {
    set_actor_anim_data(world, reference, None)?;
    let path = super::reference_load::model_path_of(world, base).unwrap_or_default();
    if let Some(slash) = path.iter().rposition(|&byte| byte == b'\\')
        && recomp_core::crt::strnicmp(&path[slash + 1..], b"Skeleton", 8) != 0
    {
        return Err(StreamError(
            "004E3539: the \"needs to have a 'skeleton*.nif'\" warning's editor name",
        ));
    }
    let Some(skin) = super::character_3d::skin_info(world, reference) else {
        return Err(StreamError(
            "004E3A38: an NPC with no skin info (LABEL_94's tail)",
        ));
    };
    super::character_3d::release_all_slots(world, skin)?;
    let anim = world
        .spawn(super::actor_animation::ActorAnimData::default())
        .id();
    set_actor_anim_data(world, reference, Some(anim))?;
    super::character_3d::npc_update_biped(world, base, reference, services)?;
    let health = super::ai_process::actor_get_actor_value(world, reference, 8)?;
    if f64::from(health) <= 0.0 {
        return Err(StreamError("004E3AC2: a dead NPC's Idle.KF and Death.KF"));
    }
    let files = super::forms_ai::root_kf_files(world, &path)?;
    if !super::actor_animation::init_from_kf_list(
        world,
        anim,
        files,
        Some(node),
        reference,
        services,
    )? {
        return Err(StreamError(
            "004E3B8A: \"Bad InitAnimation for NPC\" (its editor name)",
        ));
    }
    // 004E3BB2: a living NPC whose base's TESAnimation lists special animations (TESAnimation_HasNames) takes
    // 00476080, a stop; none of the nine has one.
    if world
        .get::<super::form_components::TesAnimation>(base)
        .is_some_and(|animation| !animation.names_04.is_empty())
    {
        return Err(StreamError(
            "004E3C1C: 00476080, the NPC's special animations",
        ));
    }
    // 004E3D9E: with a process, vt+0x2FC (0064AD10, +0x114) takes vt+0x308(1) (0064AD40, +0x115); the player's arm is a stop;
    // then vt+0x150 (00656270) with vt+0x304's +0x115, the skin info, the anim data (004D8370) and the actor.
    if let Some(process) = world
        .get::<super::actor::MobileObject>(reference)
        .and_then(|mobile| mobile.process_58)
    {
        match world.get_mut::<super::actor_process::MiddleHighProcess>(process) {
            Some(mut middle) => {
                if middle.byte_114 != 0 {
                    middle.byte_115 = 1;
                }
            }
            None => super::actor_enable::low_process_only(
                world,
                process,
                "004E3DAC: a process's vt+0x2FC",
            )?,
        }
        // 004E3DD7: the player whose view skin (vt+0x168: +0x104 while +0x588 is set) is its first-person one (006600D0(1),
        // +0x5C8) passes 0065D750(1)'s first-person anim data, a stop here; otherwise the anim data, as an NPC's.
        if Some(reference) == super::ai_conditions::the_player(world) {
            let data = world
                .get::<super::player::PlayerCharacter>(reference)
                .expect("the player");
            let view = if data.view_flags_588[0] != 0 {
                Some(skin)
            } else {
                data.first_person_skin_info_5c8.or(Some(skin))
            };
            if view.is_some() && view == data.first_person_skin_info_5c8 {
                return Err(StreamError(
                    "004E3DE2: the player's vt+0x150 with its first-person skin",
                ));
            }
        }
        super::biped_attach::place_weapon(world, process, skin, reference)?;
    }
    // LABEL_92 / LABEL_93: 00476D10(0.0, -1.0), then LABEL_94.
    super::actor_animation::update_anim_data(world, anim, reference, 0.0, -1.0)?;
    actor_animation_tail(world, reference, node, Some(anim))
}

/// IDA: N/A host deferral D15: 004E1580's creature body models: each TESModelList name (+0xEC) under the model's folder through
/// ModelLoader_LoadModelNow(path, 0, 3, 1), cloned at the reference's scale (004430C0 with a NiGeomMorpherController), and with
/// 00471B80 and a "Bip01" bone attached through 00478EC0 (under "SkinAttachment" or the bone's +0x1C), else 00479140. And
/// 004E1900's arm for an actor with a node and no skin info: 00479450(model, slot 9 or 14 for a bow, ref, null), named
/// "Weapon (%08X)" (a bow's by 00A3CF00), then outside a load 005E0DA0 and the process's vt+0x124 / vt+0x308 / vt+0x150, a bow's
/// 005E13D0 and vt+0x114, 005EA1A0 and 005EE1B0. The creatures' 3D has no body or weapon until it's ported; nothing it does
/// draws (run-af's creatures make no draws).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct CreatureBodyDeferrals {
    /// The reference and its body model paths, per 004E1580 loop not run.
    pub bodies: Vec<(Entity, Vec<Vec<u8>>)>,
    /// The actor and the weapon, per 004E1900 arm not run.
    pub weapons: Vec<(Entity, Entity)>,
}

// IDA: 004E3490 TESObjectREFR_InitAnimation's creature arm (004E3836): 004D83B0(ref, 0) and the "Skeleton*" warning (as the NPC
// arm's); a new ActorAnimData through 004D83B0, the node again (gone: "SetAnimation cleared 3D because %s (%08X) was moved"),
// 004E1580 (004728C0 on the new data, a no-op with no accum root; `creature_body_equip`, then the body models, D15), the health
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
) -> Result<(), StreamError> {
    set_actor_anim_data(world, reference, None)?;
    let path = super::reference_load::model_path_of(world, base).unwrap_or_default();
    if let Some(slash) = path.iter().rposition(|&byte| byte == b'\\')
        && recomp_core::crt::strnicmp(&path[slash + 1..], b"Skeleton", 8) != 0
    {
        return Err(StreamError(
            "004E3539: the \"needs to have a 'skeleton*.nif'\" warning's editor name",
        ));
    }
    let anim = world
        .spawn(super::actor_animation::ActorAnimData::default())
        .id();
    set_actor_anim_data(world, reference, Some(anim))?;
    super::actor_enable::creature_body_equip(world, reference)?;
    let folder = match path.iter().rposition(|&byte| byte == b'\\') {
        Some(slash) => path[..=slash].to_vec(),
        None => Vec::new(),
    };
    let bodies: Vec<Vec<u8>> = world
        .get::<super::form_components::TesModelList>(base)
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
    world
        .get_resource_or_init::<CreatureBodyDeferrals>()
        .bodies
        .push((reference, bodies));
    let health = super::ai_process::actor_get_actor_value(world, reference, 8)?;
    if f64::from(health) > 0.0 {
        return Err(StreamError(
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
    if !super::actor_animation::init_from_kf_list(
        world,
        anim,
        files,
        Some(node),
        reference,
        services,
    )? {
        return Err(StreamError(
            "004E3A2E: \"Bad InitAnimation for Creature\" (its editor name)",
        ));
    }
    // 004E3A26 returns; a dead creature has no special animations. LABEL_93: 00476D10(0.0, -1.0), then LABEL_94.
    super::actor_animation::update_anim_data(world, anim, reference, 0.0, -1.0)?;
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
) -> Result<(), StreamError> {
    super::actor_enable::process_animation_nodes(world, reference, anim)?;
    let mut cursor = world
        .get::<NiObjectNet>(node)
        .and_then(|net| net.controller_0c);
    while let Some(controller) = cursor {
        if world.get::<crate::gamebryo::stream_objects::NiClass>(controller)
            == Some(&crate::gamebryo::stream_objects::NiClass::MultiTargetTransformController)
        {
            if let Some(mut state) =
                world.get_mut::<crate::gamebryo::transform_controller::NodeController>(controller)
            {
                state.time.fields.flags |= 0x40;
            }
            break;
        }
        cursor = world
            .get::<crate::gamebryo::transform_controller::NodeController>(controller)
            .and_then(|state| state.time.next_34)
            .or_else(|| {
                crate::gamebryo::particle_controller::controller_time(world, controller)
                    .and_then(|time| time.next_34)
            });
    }
    if anim.is_some()
        && world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44
            .get(0x34)
            .is_some()
    {
        return Err(StreamError("004E3EEE: extra 0x34's update time"));
    }
    Ok(())
}

// IDA: 004D9C30 TESObjectREFR_GetBaseFullNameOrEmpty's text for a base: its TESFullName, to the terminator, or "".
pub(crate) fn base_full_name(world: &World, base: Entity) -> Vec<u8> {
    world
        .get::<super::form_components::TesFullName>(base)
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
) -> Result<(), StreamError> {
    let extras = world
        .get::<NiObjectNet>(node)
        .ok_or(StreamError("004E4206: a node with no NiObjectNET"))?
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
/// CREA to 3, anything else 6; no reference is 0. Completion: 100% but the player's first-person node, which 004DC270 tries
/// first and which isn't built.
#[must_use]
pub fn classify_reference(world: &World, node: Entity) -> i32 {
    let mut cursor = Some(node);
    while let Some(object) = cursor {
        if let Some(net) = world.get::<NiObjectNet>(object) {
            for &extra in &net.extra_data {
                if let Some(data) = world.get::<TesObjectExtraData>(extra) {
                    let Some(base) = world
                        .get::<TesObjectRefr>(data.reference_0c)
                        .and_then(|refr| refr.base_form_1c)
                    else {
                        return 0;
                    };
                    return match form_type(world, base) {
                        0x12 | 0x18 | 0x1C => 1,
                        0x14 | 0x16 | 0x21 | 0x22 => 4,
                        0x17 | 0x20 => 2,
                        0x19 | 0x1B | 0x27 => 5,
                        0x23 | 0x24 => 3,
                        _ => 6,
                    };
                }
            }
        }
        cursor = world.get::<NiAvParent>(object).and_then(|parent| parent.0);
    }
    0
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
) -> Result<(), StreamError> {
    while let Some(id) = pop_queue(world, true) {
        let task = world
            .resource::<TaskGlobals>()
            .model_loader_b33a1c
            .as_ref()
            .and_then(|loader| loader.references.get(id))
            .cloned()
            .ok_or(StreamError("0043AF30: a released reference task"))?;
        let reference = task.reference_20;
        let attached = super::ai_package::parent_cell(world, reference)
            .is_some_and(|cell| super::ai_cell_moves::cell_attached(world, cell, false));
        if !attached {
            continue;
        }
        if world.get::<super::actor::Actor>(reference).is_none()
            && super::reference_load::reference_takes_clone(world, reference)
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
fn is_clone_gap(error: &StreamError) -> bool {
    error.0.starts_with("00700610: a clone of")
        || error.0.starts_with("00700300: a clone of")
        || error.0.starts_with("004B38AE")
}

// IDA: 00480820(object): a NiGeomMorpherController in the object's chain (NiObjectNET_GetController, 00B3CA90's RTTI), else a
// node's children in slot order, the first true answering. Completion: 100%.
pub(crate) fn has_geom_morpher(world: &World, object: Entity) -> bool {
    let mut cursor = world
        .get::<NiObjectNet>(object)
        .and_then(|net| net.controller_0c);
    while let Some(controller) = cursor {
        if world.get::<crate::gamebryo::stream_objects::NiClass>(controller)
            == Some(&crate::gamebryo::stream_objects::NiClass::GeomMorpherController)
        {
            return true;
        }
        cursor = crate::gamebryo::particle_controller::controller_time(world, controller)
            .and_then(|time| time.next_34);
    }
    world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(object)
        .is_some_and(|children| {
            children
                .slots_ac
                .iter()
                .flatten()
                .any(|&child| has_geom_morpher(world, child))
        })
}

// IDA: 004CCA60 TESObjectCELL_ChangeProcessMiddleLowCount(cell, 1): the cell's extra 0x0D (ExtraProcessMiddleLow, 0041D9C0, vtable 00A34DA4, +0x0C = 0) found or made and added (0041E0A0); a new one first walks the cell's +0x48 list under the region lock (a host no-op) and calls vt+0x1AC on each MobileObject with +0x08 bit 0x800 clear, a +0x58 process and process level 3; then +0x0C counts up. The count never reaches 0 on the up arm, so 00422E20 does not run. Completion: 100% of the up arm; the vt+0x1AC call is deferred (`DeferredMiddleLowPromotions`).
// TODO: [ ] 004CCB1D vt+0x1AC 005EBB30 on each level-3 actor (Build 2); the down arm (a2 = 0) with its 00422E20 removal.
fn cell_count_process_middle_low(world: &mut World, cell: Entity) -> Result<(), StreamError> {
    use super::base_extra_list::{EXTRA_TYPE_0D, ExtraData, new_extra_entry};
    let has = world
        .get::<super::forms_world::TesObjectCell>(cell)
        .expect("a cell")
        .extra_28
        .get(EXTRA_TYPE_0D)
        .is_some();
    if !has {
        world
            .get_mut::<super::forms_world::TesObjectCell>(cell)
            .expect("a cell")
            .extra_28
            .add(new_extra_entry(EXTRA_TYPE_0D, ExtraData::Dword(0)));
        let objects: Vec<Entity> = world
            .get::<super::forms_world::TesObjectCell>(cell)
            .expect("a cell")
            .objects_48
            .items()
            .collect();
        for object in objects {
            if world.get::<super::actor::MobileObject>(object).is_none() {
                continue;
            }
            let flags = world
                .get::<super::form::FormIdentity>(object)
                .expect("a form")
                .flags_08;
            if flags & 0x800 == 0 && super::actor::actor_process_level(world, object) == 3 {
                world
                    .get_resource_or_insert_with(DeferredMiddleLowPromotions::default)
                    .actors
                    .push(object);
            }
        }
    }
    let mut data = world
        .get_mut::<super::forms_world::TesObjectCell>(cell)
        .expect("a cell");
    let entry = data
        .extra_28
        .get_mut(EXTRA_TYPE_0D)
        .expect("just found or added");
    if let ExtraData::Dword(count) = &mut entry.data {
        *count = count.wrapping_add(1);
    }
    Ok(())
}

/// IDA: N/A the actors 004CCA60 would hand to vt+0x1AC (005EBB30, the move to a MiddleLowProcess at level 2), in its walk order. A host deferral, a recorded state divergence: run-af's `after_move` has ImperialDungeon02's six (`176FF`, `17708`, `1772E`, `1770C`, `17752`, `17754`) on MiddleLow (vtable A72D64) with +0xCA = 1; here they stay Low at level 3, and 005E4B00's dispel and container changes have not run.
#[derive(Resource, Default, Debug)]
pub struct DeferredMiddleLowPromotions {
    pub actors: Vec<Entity>,
}

/// IDA: N/A the clone thread's CRT rand: `_rand` 009859E2 reads +0x14 of the per-thread data, which `_initptd` sets to 1, and
/// Random_Next's first-use flag is the main thread's to clear, so the thread's draws run its own sequence from `srand(1)`.
/// D9 runs the clone queue on the main thread, so this stands for that thread's state.
/// TODO: [ ] a capture of the clone thread's draws (none probed: run-af is main-thread only).
#[derive(Resource, Default, Debug)]
pub struct CloneThreadRand(pub recomp_core::crt::MsvcRand);

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
) -> Result<bool, StreamError> {
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
                    .resource::<super::data_handler::FormsById>()
                    .lookup(u32::from(next).wrapping_sub(0x12))
            } else if next.is_ascii_alphabetic() {
                world
                    .resource::<super::data_handler::FormsById>()
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
fn model_path_len(world: &World, form: Entity) -> Result<usize, StreamError> {
    let Some(model) = world.get::<super::form_components::TesModel>(form) else {
        return Err(StreamError(
            "004E27F0: a FlameNode form with no TESModel at +0x24",
        ));
    };
    let text = &model.path_04;
    if text.length_04 != 0xFFFF {
        return Ok(usize::from(text.length_04));
    }
    let data = text
        .data_00
        .as_deref()
        .ok_or(StreamError("004E27FA: strlen of a null +0x28"))?;
    Ok(data
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(data.len()))
}

/// IDA: 004DE3C0(object, value): with an object, each controller from +0x0C by +0x34 takes +0x10 (the phase) = value, then
/// the same for the chain of its texturing property (00707530 type 6), then with vt+8 (a node) each child slot recurses
/// (a null slot returns at once). Completion: 100%; a chain entry with no host time state is a stop.
fn set_controller_phases(
    world: &mut World,
    object: Option<Entity>,
    value: f32,
) -> Result<(), StreamError> {
    use crate::gamebryo::controller_sequence::{next_of, time_mut};
    let Some(object) = object else {
        return Ok(());
    };
    let mut owners = vec![object];
    if let Some(property) = crate::bs::sky_shader::find_property_of_type(world, object, 6)? {
        owners.push(property);
    }
    for owner in owners {
        let mut cursor = world
            .get::<NiObjectNet>(owner)
            .and_then(|net| net.controller_0c);
        while let Some(controller) = cursor {
            time_mut(world, controller)
                .ok_or(StreamError(
                    "004DE3D8: a controller with no host time state",
                ))?
                .fields
                .phase = value;
            cursor = next_of(world, controller);
        }
    }
    let children = world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(object)
        .map(|children| children.slots_ac.clone());
    for child in children.unwrap_or_default() {
        set_controller_phases(world, child, value)?;
    }
    Ok(())
}

/// IDA: 0043B000 QueuedReference_FinishOnMainThread (vt+0x38): a task not at state 6 whose reference has neither +0x08 bit
/// 0x800 nor 0x20 and whose cell is attached (0043F8C0): with a Model (+0x28), `0043A8F0` on its +0x08 root, then
/// `TES_FinishReferenceLoad3D(g_tes, ref, cell, +0x2C, null)`; any task not at state 6 then leaves the +0x08 map (vt+0x10).
/// Completion: 100%.
fn finish_on_main_thread(
    world: &mut World,
    id: crate::bs::reference_tasks::ReferenceTaskId,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), StreamError> {
    let task = world
        .resource::<TaskGlobals>()
        .model_loader_b33a1c
        .as_ref()
        .and_then(|loader| loader.references.get(id))
        .cloned()
        .ok_or(StreamError("0043B000: a released reference task"))?;
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
                .get_resource_or_init::<super::character_3d::CharacterTaskStops>()
                .stops
                .push((reference, stop.0)),
            Ok(()) => {
                world
                    .get_resource_or_init::<super::character_3d::CharacterTaskStops>()
                    .finished
                    .push(reference);
                // 0043B090's 004353D0 (+0x38's head) and 00437B60 (+0x3C's helmet): host deferral D12.
                if matches!(
                    task.class,
                    QueuedReferenceClass::Character | QueuedReferenceClass::Player
                ) {
                    world
                        .get_resource_or_init::<super::character_3d::FaceGenDeferrals>()
                        .head_attaches
                        .push(reference);
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
) -> Result<(), StreamError> {
    let reference = task.reference_20;
    let flags = world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08;
    let cell = super::ai_package::parent_cell(world, reference);
    if flags & 0x800 == 0
        && flags & 0x20 == 0
        && let Some(cell) =
            cell.filter(|&cell| super::ai_cell_moves::cell_attached(world, cell, false))
    {
        if let Some(model) = task.model_28 {
            let root = world
                .resource::<TaskGlobals>()
                .model_loader_b33a1c
                .as_ref()
                .ok_or(StreamError("0043B03A: a null g_modelLoader"))?
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
        .ok_or(StreamError("0043B082: a null g_modelLoader"))?
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
) -> Result<(), StreamError> {
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
                    .resource::<super::data_handler::FormsById>()
                    .lookup(id)
            })
            .ok_or(StreamError(
                "0043A15A: a FlameNode with no default static faults",
            ))?;
        let path = super::reference_load::model_path_of(world, form).unwrap_or_default();
        let path = fixup_model_path(&path);
        let held = {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let loader = tasks
                .model_loader_b33a1c
                .as_mut()
                .ok_or(StreamError("0043A16A: a null g_modelLoader"))?;
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
                .ok_or(StreamError("0043A1B0: a null g_modelLoader"))?;
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
    let resolved = |form: super::form_components::FormRef| match form {
        super::form_components::FormRef::Resolved(form) => form,
        super::form_components::FormRef::Raw(_) => None,
    };
    match form_type(world, base) {
        0x12 => world
            .get::<super::forms_items::TesActivator>(base)
            .and_then(|a| resolved(a.sound_54)),
        0x18 if flags & 0x2000 == 0 => world
            .get::<super::forms_items::TesDoor>(base)
            .and_then(|d| d.loop_sound_60),
        0x1A if flags & 0x2000 == 0 => world
            .get::<super::forms_items::TesLight>(base)
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
/// `00667420` (the player's node: none during the load), a light's base vt+0x3C (`0068F970`, nothing) or `007C7050` (the
/// geometry's shader passes and lights: deferred, `light_3d::ShadowSceneDeferrals`). Then
/// `TESObjectREFR_InitializeScriptAnd3DState`, without a save loading `TESObjectREFR_RunPendingScript` and
/// `004FBF90(0x1000)`, and a tree's `004CE610` arm (a stop). Completion: 100% of a non-actor's arms; the stops named.
fn finish_reference_load_3d(
    world: &mut World,
    reference: Entity,
    cell: Entity,
    node: Option<Entity>,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), StreamError> {
    let flags = world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08;
    if flags & 0x8000 != 0 && flags & 0x80000 != 0 && world.get::<Reference3D>(reference).is_some()
    {
        return Err(StreamError("0043FBA0: a reference's node detached"));
    }
    let Some(base) = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
    else {
        return Err(StreamError("00441F0B: a reference with no base faults"));
    };
    let base_type = form_type(world, base);
    if flags & 0x820 == 0 && reference_sound_form(world, reference, base, flags).is_some() {
        return Err(StreamError(
            "00441F4C: a finished reference's sound (006AE4B0)",
        ));
    }
    if super::reference_load::reference_takes_clone(world, reference) {
        // 0044218E 0043F240 TES_RemoveActorFromProcessQueue: an actor's vt+0x1C4 (005F23E0: a process level equal to its desired
        // one answers 1; a change is a stop), then 00674E10's removal from the manager's queue.
        if world.get::<super::actor::Actor>(reference).is_some() {
            if world
                .get::<super::actor::MobileObject>(reference)
                .expect("an actor")
                .process_58
                .is_some()
                && super::actor::actor_process_level(world, reference)
                    != super::actor_process::actor_desired_level(world, reference)?
            {
                return Err(StreamError(
                    "005F2408: a process level change at the finish",
                ));
            }
            super::cell_reference_loop::process_manager_drop_queued(world, reference);
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
            && world.get::<super::actor::Actor>(reference).is_some()
            && let Some(&(_, why)) = world
                .get_resource::<Load3DHostGates>()
                .and_then(|gates| gates.gated.iter().find(|(gated, _)| *gated == reference))
        {
            return Err(StreamError(why));
        }
        if let Some(node) = node {
            if base_type == 0x25 {
                return Err(StreamError("004421D0: a leveled creature's node removal"));
            }
            super::tes_grid::cell_node(world, cell)?;
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
            if super::ai_conditions::the_player(world)
                .is_some_and(|player| world.get::<Reference3D>(player).is_some())
            {
                return Err(StreamError(
                    "00667420: the player's node near a finished reference",
                ));
            }
            if base_type != 0x1A {
                world
                    .get_resource_or_init::<super::light_3d::ShadowSceneDeferrals>()
                    .geometry_walks
                    .push(node);
            }
        }
        super::forms_init_world::init_reference_script(world, reference)?;
        if !super::forms_references::save_load_active(world)? {
            super::ai_process::run_pending_script(world, reference)?;
            super::ai_package::script_event_or(world, reference, 0x1000, None);
        }
        if base_type == 0x1E {
            return Err(StreamError("00442264: a tree's 004CE610"));
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
) -> Result<(), StreamError> {
    use crate::gamebryo::scene_node::NiNodeChildren;
    let child_at = |world: &World, parent: Option<Entity>, index: usize| {
        parent
            .and_then(|parent| world.get::<NiNodeChildren>(parent))
            .and_then(|children| children.slots_ac.get(index).copied().flatten())
    };
    let root = world
        .get::<super::forms_world::CellSceneNode>(cell)
        .map(|node| node.0);
    let base = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c;
    let parent =
        if base.is_some_and(|base| super::reference_load::bound_object_is_marker(world, base)) {
            child_at(world, root, 1)
        } else {
            if !super::forms_world::cell_is_interior(world, cell) {
                return Err(StreamError("004CEB4A: an exterior's quadrant (004C9BE0)"));
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
/// (no sound form: nothing; one held or to create: a stop); `NiAVObject_Update(node, 0, 0)`; vt+0xD0 (`004129A0`, nothing);
/// a cell at process level 5 or 6 gives `00424F90` (extra 0x4B, saved Havok data: a stop). Completion: 100% of a non-actor.
fn finish_attached_reference(
    world: &mut World,
    reference: Entity,
    node: Entity,
) -> Result<(), StreamError> {
    let flags = world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08;
    let base = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
        .ok_or(StreamError("004DC10A: a reference with no base"))?;
    if flags & 0x80000 != 0 {
        return Err(StreamError("004DC151: 004354F0 with bit 0x80000"));
    }
    if flags & 0x800 == 0 {
        let held = world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44
            .get(0x5B)
            .is_some();
        if held || reference_sound_form(world, reference, base, flags).is_some() {
            return Err(StreamError("004DC1D2: 004D9310 with a sound"));
        }
    }
    crate::gamebryo::scene_update::update_scene_object(
        world,
        node,
        0.0,
        false,
        &mut crate::gamebryo::scene_update::NoSceneControllers,
    )?;
    if let Some(cell) = super::ai_package::parent_cell(world, reference) {
        let level = world
            .get::<super::forms_world::TesObjectCell>(cell)
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
            return Err(StreamError("00424F90: saved Havok data"));
        }
    }
    Ok(())
}

/// IDA: 00480770(node), a weapon's in Load3D: the children in slot order; the first named `Scb` (A3CE0C, `_stricmp`) is
/// detached in place (vt+0x8C) and released, answering true; one named `FadeNode ` (`_strnicmp`, 9) becomes the node
/// searched from its slot 0 when it is a node (vt+8), else false. Completion: 100%.
fn detach_scabbard(world: &mut World, node: Entity) -> bool {
    use crate::gamebryo::scene_node::NiNodeChildren;
    let mut current = node;
    'walk: loop {
        let children = world
            .get::<NiNodeChildren>(current)
            .map(|children| children.slots_ac.clone())
            .unwrap_or_default();
        for (index, child) in children.into_iter().enumerate() {
            let Some(child) = child else { continue };
            let Some(name) = world
                .get::<NiObjectNet>(child)
                .and_then(|net| net.name.clone())
            else {
                continue;
            };
            if name.eq_ignore_ascii_case(b"Scb") {
                crate::gamebryo::scene_node::set_child_at(world, current, index, None);
                return true;
            }
            if name.len() >= 9 && name[..9].eq_ignore_ascii_case(b"FadeNode ") {
                if world.get::<NiNodeChildren>(child).is_none() {
                    return false;
                }
                current = child;
                continue 'walk;
            }
        }
        return false;
    }
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
) -> Result<(), StreamError> {
    use crate::gamebryo::controller_sequence::{ControllerManager, ControllerSequence};
    super::forms_init::record_form_change(world, reference, 0x0200_0000)?;
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
            return Err(StreamError(
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
                let first = first.ok_or(StreamError(
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

// IDA: 00470B20(sequence, 0, 0, 1.0, 0.0, 0), then +0x48 = -FLT_MAX.
fn activate_from_start(world: &mut World, sequence: Entity) {
    crate::gamebryo::controller_sequence::activate_sequence(
        world, sequence, 0, false, 1.0, 0.0, None, false,
    );
    world
        .get_mut::<crate::gamebryo::controller_sequence::ControllerSequence>(sequence)
        .expect("a NiControllerSequence")
        .offset_48 = f32::MIN;
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

// IDA: 004715C0(0.0): NiControllerSequence_Deactivate(0.0, 0) on each of the manager's +0x4C active sequences, in order.
fn deactivate_all(world: &mut World, manager: Entity) {
    let active = world
        .get::<crate::gamebryo::controller_sequence::ControllerManager>(manager)
        .expect("a NiControllerManager")
        .active_sequences_4c
        .clone();
    for sequence in active {
        crate::gamebryo::controller_sequence::deactivate_sequence(world, sequence, 0.0, false);
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
    ) -> Result<crate::gamebryo::scene_update::SceneControllerStep, StreamError> {
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
        StreamError,
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
) -> Result<(), StreamError> {
    let result = crate::gamebryo::scene_update::update_scene_object(
        world,
        node,
        time,
        true,
        &mut LoadUpdateControllers,
    )?;
    if result.status != crate::gamebryo::scene_update::SceneUpdateStatus::Updated {
        return Err(StreamError("004DE5A5: a load's controller update stopped"));
    }
    Ok(())
}

// IDA: 0045BCC0 TESSaveLoadGame_GetChangeFlags(form, use_04): with `use_04` and a +0x04 map, its record's flags when it has one;
// otherwise the +0x00 map's, 0 for none.
pub(crate) fn change_flags(world: &World, reference: Entity, use_04: bool) -> u32 {
    let id = world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .form_id_0c;
    let Some(save) = world
        .get_resource::<super::tes::TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
    else {
        return 0;
    };
    if use_04
        && save.field_04 != 0
        && let Some(record) = save.changes_04.get(id)
    {
        return record.flags_00;
    }
    save.changes_00.get(id).map_or(0, |record| record.flags_00)
}

// IDA: 0041F830(list, 8): the extra 0x13 byte's bit 3, or bit 3 of 1 (false) with none.
fn action_flag_8(world: &World, reference: Entity) -> bool {
    match world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44
        .get(super::base_extra_list::EXTRA_ACTION)
        .map(|entry| &entry.data)
    {
        Some(super::base_extra_list::ExtraData::Action { flags_0c, .. }) => flags_0c & 8 != 0,
        _ => false,
    }
}

// IDA: 004D6E60(flags): a zero `flags` reads 0045BCC0(form, 0); bit 0x40000 inverts 0041F830(8).
fn door_open(world: &World, reference: Entity, flags: u32) -> bool {
    let flags = if flags == 0 {
        change_flags(world, reference, false)
    } else {
        flags
    };
    action_flag_8(world, reference) != (flags & 0x40000 != 0)
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
) -> Result<(), StreamError> {
    use crate::gamebryo::controller_sequence::{
        ControllerManager, ControllerSequence, deactivate_sequence,
    };
    let flags = change_flags(world, reference, false);
    let state = action_flag_8(world, reference) != (flags & 0x40000 != 0);
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
) -> Result<(), StreamError> {
    let loading = super::forms_references::save_load_active(world)?;
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
        .add(super::base_extra_list::new_extra_entry(
            0x4A,
            super::base_extra_list::ExtraData::LastFinishedSequence(name[..end].to_vec()),
        ));
    Ok(())
}
