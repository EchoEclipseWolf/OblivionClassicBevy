//! g_rendererUIState's three screen faders (`00B12DC0`, 0x18 bytes each): `005732D0` RendererUIState_CreateFader, `00572EC0`
//! RendererUIState_EndFader and `00572E30` RendererUIState_IsFaderActive. Fader 2 is the black one `0066EAF0`'s intro block
//! raises over the menus and ends before the movie; Build 4's idle frames read it through `00572E30`.
//!
//! Host substitution (D3, as the tiles' geometry): the fader's NiTriShape, its NiTexturingProperty with
//! `Data\Textures\Menus\Faders\<name>`, its NiMaterialProperty and NiAlphaProperty are a `FaderShape` record on the shape's
//! entity, with the IDs the constructors take (the geometry data's `00728670` word and the material's `007095A0` ID); the
//! NiNode, the names and the attach under the fader scene node are real.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::interface_manager::RendererUiFaderNodes;
use super::tile::InterfaceCx;
use crate::bs::scene_graphs::{AttachChildStatus, attach_scene_child};
use crate::gamebryo::mesh_geometry::allocate_geometry_data_id;
use crate::gamebryo::object_net::NiObjectNet;
use crate::gamebryo::object_net::set_object_name;
use crate::gamebryo::properties::new_material_property;
use crate::gamebryo::scene_node::{SceneNodePayload, spawn_scene_node};
use crate::tes::interface_manager::{
    attach_scene_light, attach_vertex_color, name_object, require_attached,
};

/// IDA: the static half of each record: +0x00 (`00B12DC0`) the scene byte (1: the fader node below the menus, 0: above) and
/// +0x04 (`00B12DC4`) the texture name, as the image holds them.
pub const FADER_STATICS: [(u8, &[u8]); 3] =
    [(1, b"LetterBox.dds"), (1, b"Black.dds"), (0, b"Black.dds")];

/// IDA: one 0x18-byte fader record at `00B12DC0 + 0x18 * i`: +0x08 the active byte, +0x0C the fade time, +0x10 the NiNode,
/// +0x14 the alpha. `00572DC0` zeroes the dynamic fields at startup, which `Default` is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FaderRecord {
    pub active_08: u8,
    pub fade_seconds_0c: f32,
    pub node_10: Option<Entity>,
    pub alpha_14: f32,
}

/// IDA: the three records at `00B12DC0`.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct RendererFaders {
    pub records: [FaderRecord; 3],
}

/// IDA: N/A the D3 record of `005732D0`'s NiTriShape (0xC0): four vertices from the renderer's half size, the UVs, the two
/// triangles, the base texture's path, the material's ID, emissive and alpha with its revision (+0x54), and the alpha
/// property's flags (0xEC | 1).
#[derive(Component, Clone, Debug, PartialEq)]
pub struct FaderShape {
    pub data_id_0a: u16,
    pub vertices: [[f32; 3]; 4],
    pub uvs: [[f32; 2]; 4],
    pub triangles: [u16; 6],
    pub texture: Vec<u8>,
    pub material_id_18: u32,
    pub material_revision_54: u32,
    pub material_alpha_50: f32,
    pub alpha_flags_18: u16,
}

// IDA: N/A a helper, with no address of its own.
fn require(status: AttachChildStatus, message: &'static str) -> Result<(), GameError> {
    if status == AttachChildStatus::Attached {
        Ok(())
    } else {
        Err(GameError::from_legacy_message(message))
    }
}

// IDA: N/A a helper: an object's name.
fn name(world: &mut World, entity: Entity, text: &[u8]) {
    set_object_name(
        &mut world
            .get_mut::<NiObjectNet>(entity)
            .expect("a NiObjectNET")
            .name,
        Some(text),
    );
}

/// IDA: 005732D0 RendererUIState_CreateFader(index, seconds) on g_rendererUIState. A record with a node keeps it when active,
/// else 00572EC0(index, 1) ends it first. The quad from the renderer's half width and height (x from -w to w, z from h to -h,
/// y 0), UVs (0,0) (0,1) (1,0) (1,1), triangles 0 1 2 2 1 3; a NiNode named by the record's texture at +0x10; a NiAlphaProperty
/// (flags 0xEC | 1); the NiTriShape (00728670's data ID) attached (vt+0x84) and named `Data\Textures\Menus\Faders\<name>`; its
/// NiSourceTexture (00B42D00's prefs) under a NiTexturingProperty; a NiMaterialProperty (+0x54 += 1 for emissive black and
/// again for ambient white); the bound; 004A2A90(node, 0.0) sets the material's alpha 0 (+0x54 += 1) and the alpha property's
/// bit 0; the scene byte picks the fader node below (+0) or above (+4) the menus, AttachChild, the effect and property
/// refreshes and Update(0.0, 0); +0x0C = seconds, +0x08 = 1. Completion: 100% of the state, with the shape as D3's record.
/// # Errors
/// An attach that fails.
pub fn create_fader(cx: &mut InterfaceCx, index: usize, seconds: f32) -> Result<(), GameError> {
    let record = cx.world.get_resource_or_init::<RendererFaders>().records[index];
    if record.node_10.is_some() {
        if record.active_08 != 0 {
            return Ok(());
        }
        end_fader(cx, index, true)?;
    }
    let half_width = cx.settings.renderer_width as f32 * 0.5;
    let half_height = 0.5 * cx.settings.renderer_height as f32;
    let vertices = [
        [-half_width, 0.0, half_height],
        [-half_width, 0.0, -half_height],
        [half_width, 0.0, half_height],
        [half_width, 0.0, -half_height],
    ];
    let (scene_byte, texture) = FADER_STATICS[index];
    let node = spawn_scene_node(cx.world, SceneNodePayload::default());
    cx.world.get_resource_or_init::<RendererFaders>().records[index].node_10 = Some(node);
    name(cx.world, node, texture);
    let data_id_0a = allocate_geometry_data_id(cx.world);
    let shape = spawn_scene_node(cx.world, SceneNodePayload::default());
    require(
        attach_scene_child(cx.world, node, Some(shape), true),
        "005733B5: the fader's shape was not attached",
    )?;
    let mut path = b"Data\\Textures\\Menus\\Faders\\".to_vec();
    path.extend_from_slice(texture);
    name(cx.world, shape, &path);
    let material = new_material_property(cx.world);
    // 0057361A..00573650: emissive (00B3FA90..) and ambient (g_niColorWhite), each `++` on +0x54 after the material's 1.
    let mut revision = material.raw_state_54 + 2;
    // 004A2A90(node, 0.0): the default alpha 1.0 differs, so +0x54 steps again.
    revision += 1;
    cx.world.entity_mut(shape).insert(FaderShape {
        data_id_0a,
        vertices,
        uvs: [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]],
        triangles: [0, 1, 2, 2, 1, 3],
        texture: path,
        material_id_18: material.material_id_18,
        material_revision_54: revision,
        material_alpha_50: 0.0,
        alpha_flags_18: 0xEC | 1,
    });
    let nodes = cx
        .world
        .get_resource::<RendererUiFaderNodes>()
        .copied()
        .unwrap_or_default();
    let parent = if scene_byte != 0 {
        nodes.below_00
    } else {
        nodes.above_04
    }
    .ok_or(GameError::unclassified(
        0x005736A6,
        "005736A6: a null fader scene node faults",
    ))?;
    require(
        attach_scene_child(cx.world, parent, Some(node), true),
        "005736B3: the fader was not attached",
    )?;
    crate::gamebryo::scene_update::update_scene_object(
        cx.world,
        parent,
        0.0,
        false,
        &mut crate::gamebryo::scene_update::NoSceneControllers,
    )?;
    let mut faders = cx.world.resource_mut::<RendererFaders>();
    faders.records[index].fade_seconds_0c = seconds;
    faders.records[index].active_08 = 1;
    Ok(())
}

/// IDA: 00572EC0 RendererUIState_EndFader(index, now) (stdcall). With a node: `now` or 00B3A6D4 (set only inside 0057CCC0
/// and InterfaceManager's destructor, so clear here) detaches it from its parent (vt+0x88), +0x14 = 0.0, +0x10 = null, and
/// 00440AF0(0, 0, 0); otherwise only +0x08 = 0, and 00572F60 fades it out over the frames. Completion: 100% of the clear arm;
/// the detach arm is a stop.
/// # Errors
/// The detach arm.
pub fn end_fader(cx: &mut InterfaceCx, index: usize, now: bool) -> Result<(), GameError> {
    let record = cx.world.get_resource_or_init::<RendererFaders>().records[index];
    if record.node_10.is_none() {
        return Ok(());
    }
    if now {
        return Err(GameError::unclassified(
            0x00572EE9,
            "00572EE9: 00572EC0's detach and 00440AF0 close",
        ));
    }
    cx.world.resource_mut::<RendererFaders>().records[index].active_08 = 0;
    Ok(())
}

/// IDA: 00572E30 RendererUIState_IsFaderActive(index): a node, the active byte, and an alpha of exactly 1.0. Completion: 100%.
#[must_use]
pub fn fader_active(world: &World, index: usize) -> bool {
    world
        .get_resource::<RendererFaders>()
        .is_some_and(|faders| {
            let record = faders.records[index];
            record.node_10.is_some() && record.active_08 != 0 && record.alpha_14 == 1.0
        })
}

/// IDA: 004A2A90 NiNode_SetChildrenMaterialAlpha. Walk child slots in order, applying
/// the material alpha, shader alpha and blending flag before descending each child.
/// Completion: bound geometry, shader properties and the host fader shape.
/// # Errors
/// An unbound property class.
pub fn node_set_children_material_alpha(
    world: &mut World,
    node: Entity,
    alpha: f32,
) -> Result<(), GameError> {
    let children: Vec<Entity> = world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(node)
        .map(|children| children.slots_ac.iter().flatten().copied().collect())
        .unwrap_or_default();
    for child in children {
        if let Some(mut shape) = world.get_mut::<FaderShape>(child) {
            if alpha != shape.material_alpha_50 {
                shape.material_revision_54 += 1;
                shape.material_alpha_50 = alpha;
            }
            if alpha < 1.0 {
                shape.alpha_flags_18 |= 1;
            }
            continue;
        }
        if let Some(material) = crate::gamebryo::av_object::find_property_of_type(world, child, 2)?
            && let Some(mut values) =
                world.get_mut::<crate::gamebryo::properties::MaterialProperty>(material)
            && alpha != values.values.alpha
        {
            values.raw_state_54 = values.raw_state_54.wrapping_add(1);
            values.values.alpha = alpha;
        }
        let mut clear_blend = false;
        if let Some(property) = crate::gamebryo::av_object::find_property_of_type(world, child, 4)?
            && crate::bs::sky_shader::shader_property_id(world, property)? != -1
        {
            clear_blend = !crate::bs::sky_shader::set_shader_alpha(world, property, alpha)?;
        }
        if let Some(property) = crate::gamebryo::av_object::find_property_of_type(world, child, 0)?
            && let Some(mut flags) =
                world.get_mut::<crate::gamebryo::object_net::AlphaProperty>(property)
        {
            if alpha < 1.0 {
                flags.flags_18 |= 1;
            } else if clear_blend {
                flags.flags_18 &= !1;
            }
        }
        if world
            .get::<crate::gamebryo::scene_node::NiNodeChildren>(child)
            .is_some()
        {
            node_set_children_material_alpha(world, child, alpha)?;
        }
    }
    Ok(())
}

/// IDA: 00572F60(textureManager, completeLoading) (stdcall, `retn 4`), each frame from 0040DB49 with 1 and from 005ADF1E: for
/// each of the three records with a node: active, with `completeLoading` and record 2, LoadingMenu_CompleteProgress (a
/// stop: no record 2 is active in the frames this reaches); an alpha below 1.0 steps up by frame time / +0x0C, clamped at
/// 1.0, then `004A2A90`. Inactive with an alpha above 0.0 steps down the same way, clamped at 0.0, then `004A2A90`.
/// Inactive at 0.0: the node's parent's vt+0x88 DetachChild (`detach_child`, the NiPointer it hands back released), +0x10
/// null and +0x14 = 0.0. Precision: 24-bit, each step stored as a float. Completion: 100% but the loading arm.
/// # Errors
/// The loading arm, or a stop in `004A2A90`.
pub fn fader_frame(
    world: &mut World,
    complete_loading: bool,
    frame_time: f32,
) -> Result<(), GameError> {
    for index in 0..3 {
        let record = world.get_resource_or_init::<RendererFaders>().records[index];
        let Some(node) = record.node_10 else {
            continue;
        };
        if record.active_08 != 0 {
            if complete_loading && index == 2 {
                return Err(GameError::unclassified(
                    0x00572FA0,
                    "00572FA0: LoadingMenu_CompleteProgress under fader 2",
                ));
            }
            if record.alpha_14 < 1.0 {
                // 24-bit: each x87 step rounds as an f32 operation does.
                let step = 1.0 / record.fade_seconds_0c * frame_time + record.alpha_14;
                let alpha = if step > 1.0 { 1.0 } else { step };
                world.resource_mut::<RendererFaders>().records[index].alpha_14 = alpha;
                node_set_children_material_alpha(world, node, alpha)?;
            }
        } else if record.alpha_14 > 0.0 {
            let step = record.alpha_14 - 1.0 / record.fade_seconds_0c * frame_time;
            let alpha = if step < 0.0 { 0.0 } else { step };
            world.resource_mut::<RendererFaders>().records[index].alpha_14 = alpha;
            node_set_children_material_alpha(world, node, alpha)?;
        } else {
            if let Some(parent) = world
                .get::<crate::gamebryo::av_object::NiAvParent>(node)
                .and_then(|parent| parent.0)
            {
                crate::gamebryo::scene_node::detach_child(world, parent, node);
            }
            let mut faders = world.resource_mut::<RendererFaders>();
            faders.records[index].node_10 = None;
            faders.records[index].alpha_14 = 0.0;
        }
    }
    Ok(())
}

/// IDA: 005730B0 (graph, distance, below): NiNode (0xDC) stored in faders +0 (below, "FaderNode Below Menus") or +4
/// (above, "FaderNode Above Menus", distance - 20.0); "FaderNodeLight"; vertex color property; scale = |s| with s =
/// 1.0, or 1.3333334 (A688AC)
/// when H/W != 0.75, times 1280/W when W != 1280; translate (0, distance, 0); graph AttachChild(node, 1). Completion:
/// 100%.
#[allow(clippy::assign_op_pattern)] // 005730B0's operand order: 1280/W first.
pub fn interface_create_fader_node(
    cx: &mut InterfaceCx,
    graph: Entity,
    mut distance: f32,
    below: bool,
) -> Result<(), GameError> {
    let node = spawn_scene_node(cx.world, SceneNodePayload::default());
    cx.world.init_resource::<RendererUiFaderNodes>();
    let mut faders = cx.world.resource_mut::<RendererUiFaderNodes>();
    if below {
        faders.below_00 = Some(node);
        name_object(cx.world, node, b"FaderNode Below Menus");
    } else {
        faders.above_04 = Some(node);
        name_object(cx.world, node, b"FaderNode Above Menus");
        distance -= 20.0;
    }
    attach_scene_light(cx.world, node, b"FaderNodeLight")?;
    attach_vertex_color(cx.world, node)?;
    let width = cx.settings.renderer_width as f32;
    let mut scale = 1.0f32;
    let ratio = cx.settings.renderer_height as f32 / width;
    if ratio != 0.75 {
        scale = 1.333_333_4;
    }
    if width != 1280.0 {
        scale = 1280.0 / width * scale;
    }
    let mut payload = cx
        .world
        .get_mut::<SceneNodePayload>(node)
        .expect("just spawned");
    payload.local_transform.scale = scale.abs();
    payload.local_transform.translation = [0.0, distance, 0.0];
    require_attached(
        attach_scene_child(cx.world, graph, Some(node), true),
        "005732B0: the child was not attached",
    )
}
