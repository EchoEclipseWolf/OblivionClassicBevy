//! A light reference's 3D: TESObjectLIGH vt+0x114 (004B20D0), its model root (004B0A30), the NiPointLight 004B1600 hangs
//! under the node and its attenuation (004B1210). The original's shadow-scene bookkeeping (007C6AE0 and the walks
//! 007C5D00 and 007C7050 run for each finished reference) is the renderer's light-geometry state, which Bevy replaces: the
//! port records what would have gone in (`ShadowSceneDeferrals`), a host substitution. Evidence: run-af's `after_move`
//! for the prison's lights (`1FBD2`, `1FC1D`, `1FC36`): an unnamed NiNode holding one NiPointLight `" PtLight"`, the
//! diffuse the color bytes / 255, +0xF8..+0x100 the radius, the attenuation `(0, 0, 16 / r^2)`, +0xDC the fade, and the
//! reference's extra 0x30 holding the light.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::bs::model_loader::ModelBuildServices;
use crate::gamebryo::av_object::{NiAvParent, NiAvProperties};
use crate::gamebryo::clone::{CloningProcess, clone_object};
use crate::gamebryo::object_net::NiObjectNet;
use crate::gamebryo::scene_derived::{PointLight, allocate_dynamic_effect_uid};
use crate::gamebryo::scene_node::{
    NiAvWorld, NiEffectAffectedNodes, NiNodeChildren, SceneNodePayload, spawn_cold_node,
};
use crate::gamebryo::stream_objects::NiClass;
use crate::gamebryo::transform::NiTransform;
use crate::tes::actor::TesObjectRefr;
use crate::tes::base_extra_list::{EXTRA_LIGHT, ExtraData, new_extra_entry};
use crate::tes::form::FormIdentity;
use crate::tes::forms_init::form_type;
use crate::tes::forms_items::TesLight;

/// IDA: 004129A0, the TESObjectLIGH vt+0xD4 (and TESObjectREFR's vt+0xD0): `firstButton` (A2F7EC), an empty string. The
/// light node takes it as its name and the point light `"%s PtLight"` of it.
const LIGHT_NAME_A2F7EC: &[u8] = b"";

/// IDA: N/A what the host leaves to Bevy where the original's renderer keeps light-geometry state: `lights` are the point
/// lights 007C6AE0 would add to the shadow scene's +0xE4 list (a ShadowSceneLight each, 007D6060 and
/// `ShadowSceneLight_SetLight`, then 007C5F60's walks); `object_walks` the nodes 007C5D00 would test against each light
/// (007D6900, 007D59E0); `geometry_walks` the nodes 007C7050 would walk, whose geometry's shader property 007C5750 updates
/// (+0x1C's scene index against the shadow scene's +0x11C, the pass list's 007D6940) before each light's 007D2EA0 and
/// 007D5790. A recorded substitution: the original's lists and those shader-property stores aren't kept.
#[derive(Resource, Default, Debug)]
pub struct ShadowSceneDeferrals {
    pub lights: Vec<Entity>,
    pub object_walks: Vec<Entity>,
    pub geometry_walks: Vec<Entity>,
}

/// IDA: the bLightAttenuation INI values 004B1210 reads live, and `dword_B35464[407]`'s low byte: 004B1160 sets it from
/// bUseConstant (1), bUseLinear (2) and bUseQuadratic (4), or 4 for none, as a CRT static initializer (009E26B0) before
/// the INI loads, so it holds the registered defaults: 4 (run-af reads 0x104).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct LightAttenuationSettings {
    pub bits_b35ac0: u8,
    pub linear_method_b08158: u32,
    pub quadratic_method_b08160: u32,
    pub linear_value_b08168: f32,
    pub quadratic_value_b08170: f32,
    pub constant_value_b08178: f32,
    pub out_quad_in_lin_b08180: u8,
    pub linear_radius_mult_b08188: f32,
    pub quadratic_radius_mult_b08190: f32,
}

/// IDA: 004B1160 over the registered defaults (the CRT runs it before any INI read). Completion: 100% of the bits; its
/// clamps of 00B08158 and 00B08160 and its zero-multiplier fixes apply to the defaults, which already pass them.
#[must_use]
pub fn crt_attenuation_bits() -> u8 {
    let default = |address: u32| {
        crate::tes::settings_table::STATIC_SETTINGS
            .iter()
            .find(|setting| setting.address == address)
            .map_or(0, |setting| setting.raw as u8)
    };
    let mut bits = 0;
    if default(0x00B0_8138) == 1 {
        bits |= 1;
    }
    if default(0x00B0_8140) == 1 {
        bits |= 2;
    }
    if default(0x00B0_8148) == 1 {
        bits |= 4;
    }
    if bits == 0 { 4 } else { bits }
}

/// IDA: 004B20D0 TESObjectLIGH vt+0x114(ref) for a reference: the root by vt+0x110 (`004B0A30`); a null one answers
/// null. With an empty model path the root itself, named by vt+0xD4; the base's +0x08 bit 0x10 sets the reference's
/// (0046A9C0); a cloning process at the reference's scale (vt+0xEC), and with a model path `NiObject_Clone`; a result
/// takes the reference's position (vt+0x174) and `TESObjectREFR_BuildInitialRotationMatrix`, then `NiAVObject_Update(0, 0)`;
/// `004B1600`; `NiAVObject_RefreshPropertyState`. Completion: 100% for a reference that isn't an actor.
/// # Errors
/// A stop in a callee.
pub fn light_create_3d(
    world: &mut World,
    base: Entity,
    reference: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, GameError> {
    let has_model =
        crate::tes::reference_load::model_path_of(world, base).is_some_and(|path| !path.is_empty());
    let Some(root) = light_model_root(world, base, reference, has_model, services)? else {
        return Ok(None);
    };
    let mut node = None;
    if !has_model {
        world.get_mut::<NiObjectNet>(root).expect("a NiNode").name =
            Some(LIGHT_NAME_A2F7EC.to_vec());
        node = Some(root);
    }
    if world.get::<FormIdentity>(base).expect("a form").flags_08 & 0x10 != 0 {
        world
            .get_mut::<FormIdentity>(reference)
            .expect("a form")
            .flags_08 |= 0x10;
    }
    let (position, scale) = {
        let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
        (refr.position_2c, refr.scale_38)
    };
    // 004B2251 004D7AF0 (a light's base is never null here: it's the caller's `base`).
    let rotation = crate::tes::forms_references::reference_initial_rotation(world, reference)?;
    if has_model {
        let mut process = CloningProcess {
            scale_10: [scale; 3],
            ..CloningProcess::default()
        };
        node = Some(clone_object(
            world,
            &mut process,
            root,
            &mut services.prepare,
        )?);
    }
    if let Some(node) = node {
        {
            let mut payload = world.get_mut::<SceneNodePayload>(node).expect("a NiNode");
            payload.local_transform.translation = position;
            payload.local_transform.rotation = rotation;
        }
        update(world, node)?;
    }
    attach_point_light(world, base, reference, node)?;
    if let Some(node) = node {
        crate::gamebryo::scene_state::refresh_property_state(
            world,
            node,
            &services.prepare.property_defaults,
        )?;
    }
    Ok(node)
}

/// IDA: 004B0A30 TESObjectLIGH vt+0x110(ref): with a model path, `TESBoundObject_GetModelRoot`, otherwise a new NiNode
/// (FormHeap 0xDC, 0070xxxx with no children); a root that isn't a node (vt+8) would go under a new NiNode (a stop).
/// Completion: 100%.
fn light_model_root(
    world: &mut World,
    base: Entity,
    reference: Entity,
    has_model: bool,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, GameError> {
    let root = if has_model {
        super::reference_3d::model_root_for(world, base, Some(reference), services)?
    } else {
        Some(spawn_cold_node(world))
    };
    if root.is_some_and(|root| world.get::<NiNodeChildren>(root).is_none()) {
        return Err(GameError::unclassified(
            0x004B0A8B,
            "004B0A8B: a light model root that isn't a node",
        ));
    }
    Ok(root)
}

// IDA: NiAVObject_Update(object, 0.0, 0) on the main thread.
fn update(world: &mut World, object: Entity) -> Result<(), GameError> {
    crate::gamebryo::scene_update::update_scene_object(
        world,
        object,
        0.0,
        false,
        &mut crate::gamebryo::scene_update::NoSceneControllers,
    )?;
    Ok(())
}

/// IDA: 004B1600(ref, node): a null either returns. The light's +0x7C bit 0x20, or the reference below its cell's water
/// (004CACE0), takes the under-water arm: a light base with a lit FlameNode (004DE320) sets the reference's bit 0x80
/// (0046AB60) and blacks the materials' emissive (004B1580), a stop; then a reference that isn't an actor with bit 0x20
/// returns. The colors are +0x78..+0x7A / 255 (negated for +0x7C bit 4). `AttachLight` (vt+0x58) if it is a node, else the
/// node itself, is searched for a NiPointLight child; with none, a new NiPointLight `"%s PtLight"` is attached (vt+0x84
/// with 1) and the parent updated. `TESObjectREFR_SetExtraLight`; +0xE0 ambient and +0xF8 specular zero and +0xB8 up by four
/// through the stores; +0xF8..+0x100 the radius (+0x74), +0xEC diffuse; 007C6AE0 into the shadow scene (deferred);
/// `004B1210(radius)`; +0xDC the fade (+0x88), +0xB8 up. Completion: 100%; the under-water arm's FlameNode case stops.
fn attach_point_light(
    world: &mut World,
    base: Entity,
    reference: Entity,
    node: Option<Entity>,
) -> Result<(), GameError> {
    let Some(node) = node else {
        return Ok(());
    };
    let light = world
        .get::<TesLight>(base)
        .cloned()
        .ok_or(GameError::unclassified(
            0x004B1600,
            "004B1600: a light base with no TESObjectLIGH data",
        ))?;
    let below_water =
        crate::tes::forms_references::parent_cell(world, reference).is_some_and(|cell| {
            let z = world
                .get::<TesObjectRefr>(reference)
                .expect("a reference")
                .position_2c[2];
            super::tes_grid::cell_water_height(world, cell) > z
        });
    if light.flags_7c & 0x20 != 0 || below_water {
        if form_type(world, base) == 0x1A && lit_flame_node(world, node) {
            return Err(GameError::unclassified(
                0x004B163E,
                "004B163E: an under-water light's flames (004B1580)",
            ));
        }
        if world.get::<crate::tes::actor::Actor>(reference).is_none() && light.flags_7c & 0x20 != 0
        {
            return Ok(());
        }
    }
    let bytes = light.color_78.to_le_bytes();
    let mut diffuse = [
        f32::from(bytes[0]) / 255.0,
        f32::from(bytes[1]) / 255.0,
        f32::from(bytes[2]) / 255.0,
    ];
    if light.flags_7c & 4 != 0 {
        diffuse = diffuse.map(|value| -value);
    }
    let attach = crate::gamebryo::scene_node::object_by_name(world, node, b"AttachLight")
        .filter(|&found| world.get::<NiNodeChildren>(found).is_some())
        .unwrap_or(node);
    let existing = world
        .get::<NiNodeChildren>(attach)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .find(|&child| world.get::<NiClass>(child) == Some(&NiClass::PointLight));
    let point = match existing {
        Some(point) => point,
        None => {
            let point = spawn_point_light(world);
            let mut name = LIGHT_NAME_A2F7EC.to_vec();
            name.extend_from_slice(b" PtLight");
            world.get_mut::<NiObjectNet>(point).expect("spawned").name = Some(name);
            crate::bs::scene_graphs::attach_scene_child(world, attach, Some(point), true);
            update(world, attach)?;
            point
        }
    };
    set_extra_light(world, reference, point);
    let radius = light.radius_74 as f32;
    {
        let mut state = world.get_mut::<PointLight>(point).expect("a NiPointLight");
        state.ambient_e0 = [0.0; 3];
        state.specular_f8 = [0.0; 3];
        state.revision_b8 = state.revision_b8.wrapping_add(4);
        state.specular_f8 = [radius; 3];
        state.diffuse_ec = diffuse;
    }
    crate::bs::scene_lighting::add_point_light(world, point)?;
    world
        .get_resource_or_init::<ShadowSceneDeferrals>()
        .lights
        .push(point);
    let settings =
        *world
            .get_resource::<LightAttenuationSettings>()
            .ok_or(GameError::unclassified(
                0x004B1210,
                "004B1210: the bLightAttenuation settings aren't published",
            ))?;
    let interior = world
        .get_resource::<super::tes::TesGlobals>()
        .and_then(|globals| globals.tes.as_ref())
        .is_some_and(|tes| tes.current_interior_cell_34.is_some());
    let mut state = world.get_mut::<PointLight>(point).expect("a NiPointLight");
    set_attenuation(&mut state, light.radius_74, interior, &settings);
    state.dimmer_dc = light.fade_88;
    state.revision_b8 = state.revision_b8.wrapping_add(1);
    Ok(())
}

/// IDA: 004B1210(radius, light): +0xB8 up by three; +0x108 = fConstantValue with bit 1, else 0; +0x10C and +0x110 = 0.
/// With bit 2, or an interior current cell and bOutQuadInLin 1: v = radius * fLinearRadiusMult; uLinearMethod 0 gives
/// fLinearValue, 2 gives fLinearValue / (v * v), 1 gives 1 / radius * fLinearValue, each with a zero (or NaN) v giving
/// 0.01; +0xB8 up. With bit 4, or no interior and bOutQuadInLin 1, likewise for +0x110 with uQuadraticMethod,
/// fQuadraticRadiusMult and fQuadraticValue. On the main thread at 24-bit precision, so each step rounds to float.
/// Completion: 100%.
pub fn set_attenuation(
    light: &mut PointLight,
    radius: u32,
    interior: bool,
    settings: &LightAttenuationSettings,
) {
    let bits = settings.bits_b35ac0;
    light.revision_b8 = light.revision_b8.wrapping_add(3);
    light.attenuation_108 = [
        if bits & 1 != 0 {
            settings.constant_value_b08178
        } else {
            0.0
        },
        0.0,
        0.0,
    ];
    let radius = radius as f32;
    let term = |method: u32, mult: f32, value: f32| {
        let scaled = radius * mult;
        match method {
            0 => value,
            _ if scaled == 0.0 || scaled.is_nan() => 0.01,
            2 => value / (scaled * scaled),
            _ => 1.0 / radius * value,
        }
    };
    if bits & 2 != 0 || (interior && settings.out_quad_in_lin_b08180 == 1) {
        light.attenuation_108[1] = term(
            settings.linear_method_b08158,
            settings.linear_radius_mult_b08188,
            settings.linear_value_b08168,
        );
        light.revision_b8 = light.revision_b8.wrapping_add(1);
    }
    if bits & 4 != 0 || (!interior && settings.out_quad_in_lin_b08180 == 1) {
        light.attenuation_108[2] = term(
            settings.quadratic_method_b08160,
            settings.quadratic_radius_mult_b08190,
            settings.quadratic_value_b08170,
        );
        light.revision_b8 = light.revision_b8.wrapping_add(1);
    }
}

/// IDA: `TESObjectREFR_SetExtraLight` (004DD130): a FormHeap(8) ExtraLightData {the light with a reference, 1.0}, then
/// `BaseExtraList_SetLightData` (00428CC0): an entry 0x30 takes it in place of its old data, otherwise a new ExtraLight
/// (00429970) is added (0041E0A0). Completion: 100%.
fn set_extra_light(world: &mut World, reference: Entity, light: Entity) {
    let data = ExtraData::LightData {
        light_00: light,
        fade_04: 1.0,
    };
    let mut refr = world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference");
    let extras = &mut refr.extra_data_44;
    match extras.get_mut(EXTRA_LIGHT) {
        Some(entry) => entry.data = data,
        None => extras.add(new_extra_entry(EXTRA_LIGHT, data)),
    }
}

/// IDA: 004DE320(node): a `FlameNode` name (`_strnicmp`, 9) with a nonzero +0xB8 child count, or any child whose RTTI is
/// NiNode itself that answers true. Completion: 100%.
fn lit_flame_node(world: &World, node: Entity) -> bool {
    let named = world
        .get::<NiObjectNet>(node)
        .and_then(|net| net.name.as_deref())
        .is_some_and(|name| name.len() >= 9 && name[..9].eq_ignore_ascii_case(b"FlameNode"));
    let children = world
        .get::<NiNodeChildren>(node)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default();
    let mut lit = named && children.iter().flatten().count() != 0;
    for child in children.into_iter().flatten() {
        let exact = world.get::<NiClass>(child) == Some(&NiClass::Node)
            && world
                .get::<crate::bs::fade_node::BsFadeNode>(child)
                .is_none();
        if exact && !lit {
            lit = lit_flame_node(world, child);
        }
    }
    lit
}

// IDA: 004B1600's FormHeap(0x114) and the constructors above; the +0xB0 UID is drawn in 007090C0's order. Completion: 100%.
pub fn spawn_point_light(world: &mut World) -> Entity {
    let uid_b0 = allocate_dynamic_effect_uid(world);
    world
        .spawn((
            NiClass::PointLight,
            PointLight {
                av_flags_18: 0x000E,
                local_transform: NiTransform::default(),
                enabled_ac: true,
                uid_b0,
                unknown_b4: 0,
                revision_b8: 1,
                dimmer_dc: 1.0,
                ambient_e0: [1.0; 3],
                diffuse_ec: [1.0; 3],
                specular_f8: [1.0; 3],
                unknown_104: 0,
                attenuation_108: [0.0, 1.0, 0.0],
            },
            NiObjectNet::default(),
            NiAvProperties::default(),
            NiAvParent::default(),
            NiAvWorld::default(),
            NiEffectAffectedNodes::default(),
        ))
        .id()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> LightAttenuationSettings {
        LightAttenuationSettings {
            bits_b35ac0: crt_attenuation_bits(),
            linear_method_b08158: 1,
            quadratic_method_b08160: 2,
            linear_value_b08168: 3.0,
            quadratic_value_b08170: 16.0,
            constant_value_b08178: 0.0,
            out_quad_in_lin_b08180: 0,
            linear_radius_mult_b08188: 1.0,
            quadratic_radius_mult_b08190: 1.0,
        }
    }

    // run-af's after_move: the prison's lights of radius 400, 700 and 250.
    #[test]
    fn the_prison_lights_take_the_quadratic_arm() {
        assert_eq!(crt_attenuation_bits(), 4);
        let mut world = World::new();
        let entity = spawn_point_light(&mut world);
        let mut light = world.get::<PointLight>(entity).unwrap().clone();
        for (radius, quadratic) in [
            (400, 0x38D1_B717_u32),
            (700, 0x3808_F4F5),
            (250, 0x3986_37BD),
        ] {
            set_attenuation(&mut light, radius, false, &defaults());
            assert_eq!(light.attenuation_108[0], 0.0);
            assert_eq!(light.attenuation_108[1], 0.0);
            assert_eq!(
                light.attenuation_108[2].to_bits(),
                quadratic,
                "radius {radius}"
            );
        }
    }
}
