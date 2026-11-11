//! The shadow scene's geometry/light links used by actor detection and torch decisions.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::scene_graphs::{
    SceneGraphGlobals, ShadowSceneLight, ShadowSceneNode, construct_shadow_scene_light,
};
use super::sky_shader::NiGeometryShader;
use crate::gamebryo::av_object::find_property_of_type;
use crate::gamebryo::{
    mesh_geometry::NiGeometry,
    properties::LightingProperty,
    scene_derived::PointLight,
    scene_node::{NiAvWorld, NiNodeChildren},
    scene_state::ScenePropertyState,
    scene_update::child_flags,
};

// IDA: 007D2EA0 ShadowSceneLight_AffectsObject, point-light sphere arm, also used
// by 007ECC60 when ordering the property list. Completion: the two stored distances.
fn surface_distance(world: &World, source: Entity, bound: [f32; 4]) -> f32 {
    let position = world
        .get::<NiAvWorld>(source)
        .expect("a light world transform")
        .transform
        .translation;
    let [x, y, z] = std::array::from_fn(|i| position[i] - bound[i]);
    ((x * x + y * y) + z * z).sqrt() - bound[3]
}

// IDA: 007D5790 ShadowSceneLight_AddGeometry, 007EE330 BSShaderLightingProperty_AddLight.
// Completion: the ordinary point-light arm of a geometry with a bound lighting shader.
fn add_geometry(world: &mut World, light: Entity, geometry: Entity) {
    if world.get::<NiGeometryShader>(geometry).is_none() {
        return;
    }
    let Some(property) = world
        .get::<NiGeometry>(geometry)
        .and_then(|g| g.property_state_ac)
        .and_then(|state| world.get::<ScenePropertyState>(state))
        .and_then(|state| state.slots[4])
        .filter(|&p| world.get::<LightingProperty>(p).is_some())
    else {
        return;
    };
    {
        let mut entry = world.get_mut::<ShadowSceneLight>(light).unwrap();
        if let Some(fence) = entry.geometry_refresh_fence_144
            && let Some(offset) = entry.geometry_list_e4[fence + 1..]
                .iter()
                .position(|&g| g == geometry)
        {
            entry.geometry_list_e4.remove(fence + 1 + offset);
            entry.geometry_list_e4.insert(fence, geometry);
            entry.geometry_refresh_fence_144 = Some(fence + 1);
            return;
        }
    }
    let bound = world
        .get::<NiAvWorld>(geometry)
        .expect("a triangle geometry")
        .bound;
    let score = |light| {
        let source = world
            .get::<ShadowSceneLight>(light)
            .expect("a registered light")
            .light_100
            .unwrap();
        surface_distance(world, source, bound)
            / world.get::<PointLight>(source).unwrap().specular_f8[0]
    };
    let new_score = score(light);
    let lights = &world.get::<LightingProperty>(property).unwrap().lights_6c;
    // 007ECC60 stops at the first equal pointer or the first strictly larger score.
    let mut before = Some(lights.len());
    for (i, &old) in lights.iter().enumerate() {
        if old == light {
            before = None;
            break;
        }
        if score(old) > new_score {
            before = Some(i);
            break;
        }
    }
    let mut property = world.get_mut::<LightingProperty>(property).unwrap();
    if let Some(i) = before {
        property.lights_6c.insert(i, light);
    }
    property.last_pass_state_24 = 0;
    let mut entry = world.get_mut::<ShadowSceneLight>(light).unwrap();
    entry.geometry_list_e4.insert(0, geometry);
    if let Some(fence) = &mut entry.geometry_refresh_fence_144 {
        *fence += 1;
    }
}

// IDA: 007D6940 ShadowSceneLight_RemoveGeometry, 007EE720 BSShaderLightingProperty_RemoveLight.
// Completion: unlink the local property and the shadow light's geometry entry.
fn remove_geometry(world: &mut World, light: Entity, geometry: Entity) -> Result<(), GameError> {
    if let Some(property) = find_property_of_type(world, geometry, 4)?
        && let Some(mut p) = world.get_mut::<LightingProperty>(property)
    {
        if let Some(i) = p.lights_6c.iter().position(|&l| l == light) {
            p.lights_6c.remove(i);
        }
        p.last_pass_state_24 = 0;
    }
    let mut light = world
        .get_mut::<ShadowSceneLight>(light)
        .expect("a registered light");
    if let Some(i) = light.geometry_list_e4.iter().position(|&g| g == geometry) {
        light.geometry_list_e4.remove(i);
    }
    Ok(())
}

// IDA: 007D59E0 ShadowSceneLight_UpdateObjectTree. The ordinary point-light arm tests
// each bound before descending. Zero-radius and hidden children are skipped.
// Completion: NiNode, NiTriShape and NiTriStrips on the prison path.
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "the x87 compare: a NaN takes the negated arm"
)]
fn update_object_tree(world: &mut World, light: Entity, source: Entity, root: Entity) {
    let mut objects = vec![root];
    while let Some(object) = objects.pop() {
        let Some(bound) = world.get::<NiAvWorld>(object).map(|n| n.bound) else {
            continue;
        };
        if bound[3] == 0.0 || child_flags(world, object).is_some_and(|f| f & 1 != 0) {
            continue;
        }
        let radius = world.get::<PointLight>(source).unwrap().specular_f8[0];
        if !(radius > surface_distance(world, source, bound)) {
            continue;
        }
        if let Some(children) = world.get::<NiNodeChildren>(object) {
            objects.extend(children.slots_ac.iter().rev().flatten().copied());
        } else if world.get::<NiGeometry>(object).is_some() {
            add_geometry(world, light, object);
        }
    }
}

// IDA: 007C6AE0 ShadowSceneNode_AddLight(source, 0), 007D3400 ShadowSceneLight_SetLight,
// then 007C5F60 ShadowSceneNode_UpdateLightObjects. Completion: slot zero point lights.
pub(crate) fn add_point_light(world: &mut World, source: Entity) -> Result<(), GameError> {
    let Some(scene) = world
        .get_resource::<SceneGraphGlobals>()
        .and_then(|g| g.shadow_slots[0])
    else {
        return Err(GameError::unclassified(
            0x007C6AE0,
            "point light has no world shadow scene",
        ));
    };
    let entries = &world.get::<ShadowSceneNode>(scene).unwrap().light_list_e4;
    let existing = entries
        .iter()
        .copied()
        .find(|&e| world.get::<ShadowSceneLight>(e).unwrap().light_100 == Some(source));
    let light = if let Some(light) = existing {
        light
    } else {
        let mut entry = construct_shadow_scene_light();
        entry.light_100 = Some(source);
        entry.is_point_light_fc = 1;
        let light = world.spawn(entry).id();
        world
            .get_mut::<ShadowSceneNode>(scene)
            .unwrap()
            .light_list_e4
            .push(light);
        light
    };
    // 007D5ED0 puts the same marker at the head on each pass.
    let marker = match world
        .get::<ShadowSceneLight>(light)
        .unwrap()
        .geometry_fence_marker_148
    {
        Some(marker) => marker,
        None => {
            let marker = crate::gamebryo::mesh_geometry::construct_tri_shape(world, None);
            let mut object = world
                .get_mut::<crate::gamebryo::object_net::NiObjectNet>(marker)
                .unwrap();
            crate::gamebryo::object_net::set_object_name(&mut object.name, Some(b"fence"));
            world
                .get_mut::<ShadowSceneLight>(light)
                .unwrap()
                .geometry_fence_marker_148 = Some(marker);
            marker
        }
    };
    let mut entry = world.get_mut::<ShadowSceneLight>(light).unwrap();
    entry.geometry_list_e4.insert(0, marker);
    entry.geometry_refresh_fence_144 = Some(0);
    if child_flags(world, source).is_none_or(|f| f & 1 == 0) {
        let children = world.get::<NiNodeChildren>(scene).unwrap().slots_ac.clone();
        for child in children.into_iter().flatten() {
            if world.get::<NiNodeChildren>(child).is_some() {
                update_object_tree(world, light, source, child);
            }
        }
    }
    // 007D6A40 starts at the fence. Rediscovered nodes have moved ahead of it
    // without reinserting their property lights, including equal-score lights.
    let entry = world.get::<ShadowSceneLight>(light).unwrap();
    let stale = entry.geometry_list_e4[entry.geometry_refresh_fence_144.unwrap()..].to_vec();
    for geometry in stale {
        remove_geometry(world, light, geometry)?;
    }
    world
        .get_mut::<ShadowSceneLight>(light)
        .unwrap()
        .geometry_refresh_fence_144 = None;
    Ok(())
}

// IDA: 007C5D00 ShadowSceneNode_UpdateObjectLights. Completion: ordinary slot-zero lights
// with the reference loaded and its root visible.
pub(crate) fn update_object_lights(world: &mut World, root: Entity) {
    let Some(scene) = world
        .get_resource::<SceneGraphGlobals>()
        .and_then(|g| g.shadow_slots[0])
    else {
        return;
    };
    let lights = world
        .get::<ShadowSceneNode>(scene)
        .unwrap()
        .light_list_e4
        .clone();
    for light in lights {
        let source = world
            .get::<ShadowSceneLight>(light)
            .unwrap()
            .light_100
            .unwrap();
        update_object_tree(world, light, source, root);
    }
}

// IDA: 007C7050 ShadowSceneNode_UpdateGeometryLights, 007C6100 UpdateGeometryLightsLeaf.
// Completion: visible triangle geometry of ordinary prison scene nodes.
pub(crate) fn update_geometry_lights(world: &mut World, root: Entity) -> Result<(), GameError> {
    let mut objects = vec![root];
    while let Some(object) = objects.pop() {
        if let Some(children) = world.get::<NiNodeChildren>(object) {
            objects.extend(
                children
                    .slots_ac
                    .iter()
                    .rev()
                    .flatten()
                    .copied()
                    .filter(|&c| child_flags(world, c).is_none_or(|f| f & 1 == 0)),
            );
            continue;
        }
        let Some(property) = find_property_of_type(world, object, 4)? else {
            continue;
        };
        let Some(p) = world.get::<LightingProperty>(property) else {
            continue;
        };
        let Some(scene) = world
            .resource::<SceneGraphGlobals>()
            .shadow_slots
            .get(usize::from(p.scene_index_1c))
            .copied()
            .flatten()
        else {
            continue;
        };
        for light in p.lights_6c.clone() {
            remove_geometry(world, light, object)?;
        }
        let bound = world
            .get::<NiAvWorld>(object)
            .expect("a lit geometry")
            .bound;
        let lights = world
            .get::<ShadowSceneNode>(scene)
            .unwrap()
            .light_list_e4
            .clone();
        for light in lights {
            let source = world
                .get::<ShadowSceneLight>(light)
                .unwrap()
                .light_100
                .unwrap();
            if world.get::<PointLight>(source).unwrap().specular_f8[0]
                > surface_distance(world, source, bound)
            {
                add_geometry(world, light, object);
            }
        }
    }
    Ok(())
}

// IDA: 007D3400 ShadowSceneLight_SetLight(light): +0x100 NiPointer assign (equal skips), +0xFC = IsKindOf(NiPointLight 00B3FD80); a point light with +0x104 set
// copies the light's world translate into +0x108..+0x110; otherwise +0xD4 = 1.0. Completion: 100% for the bound classes: only NiDirectionalLight is bound, so the
// IsKindOf is false for every host light.
// TODO: [ ] The point light arm (007D347C), with NiPointLight.
pub fn set_shadow_scene_light_light(light: &mut ShadowSceneLight, effect: Entity) {
    // 007D3404..007D3430.
    if light.light_100 != Some(effect) {
        light.light_100 = Some(effect);
    }
    // 007D3440..007D3470.
    light.is_point_light_fc = 0;
    // 007D34A8 +0xD4 = 1.0.
    light.unknown_d4[0] = 1.0;
}

// IDA: 007C5850 ShadowSceneNode_SetSunLight(node, light): +0x118 is deleted (vtable +0, 1) and cleared, a new 0x220 ShadowSceneLight (007D6060) is stored raw at
// +0x118, then 007D3400(light) on it. Completion: 100% of the order.
pub fn set_shadow_scene_sun_light(node: &mut ShadowSceneNode, light: Entity) {
    // 007C5856..007C586E.
    node.light_118 = None;
    // 007C5870..007C5887 FormHeap(0x220) 007D6060 -> +0x118 (raw).
    let mut new_light = Box::new(construct_shadow_scene_light());
    // 007C588D 007D3400(light) on the new ShadowSceneLight.
    set_shadow_scene_light_light(&mut new_light, light);
    node.light_118 = Some(new_light);
}
