//! Phase 05 row 05-C: 00406950 Startup_InitializeSceneGraphs, reached from WinMain 0040EAAC: the World, Menu and 3DMenu SceneGraphs
//! with their NiCamera and NiCullingProcess, the ShadowSceneNode with its two ShadowSceneLights and four properties, the Sky and
//! Weather nodes, the LOD roots (BSClearZNode), the shader-flag globals and the final world update. 007BA0F0 (05-D), 007C2450 and
//! 004814C0 (05-E) are injected steps, which `winmain_scene` binds. Plus WinMain 0040EAEF..0040EB06 (Phase 06), which publishes the
//! World camera aliases. The classes and globals are in `recomp_game::bs::scene_graphs`.
//!
//! Every x87 sequence runs after CreateDevice (0040E865) under the 24-bit precision control word, so each arithmetic result is a
//! float and the arithmetic here is `f32` in the original operand grouping.
//! Evidence: evidence/phase05-window-2026-10-06/disasm-05C*.txt, P05-RUN-001.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87::float_copy;
use recomp_game::bs::scene_graphs::{
    AttachChildStatus, CameraStaticAliases, REFERENCE_FORM_TYPE_CLASSIFIER_ADDRESS,
    SceneGlobalStore, SceneGraph, SceneGraphGlobals, ShadowSceneNode, attach_scene_child,
    attach_scene_property, construct_scene_graph, construct_shadow_scene_node,
    scene_graph_settings_from, set_object_name, spawn_clear_z_node,
};
use recomp_game::gamebryo::object_net::NiObjectNet;
use recomp_game::gamebryo::properties::{
    FogProperty, VertexColorProperty, WireframeProperty, ZBufferProperty,
};
use recomp_game::gamebryo::scene_node::{NiNodeChildren, SceneNodePayload, spawn_scene_node};
use recomp_game::gamebryo::scene_state::{
    ScenePropertyDefaults, refresh_effect_state, refresh_property_state,
};
use recomp_game::gamebryo::scene_update::{
    SceneUpdateResult, SceneUpdateRuntime, SceneUpdateStatus, update_scene_object,
};
use recomp_game::gamebryo::screen_elements::build_image_space_fallback_quad;
use recomp_game::gamebryo::stream_objects::NiClass;
use recomp_game::tes::settings::SettingCollection;

/// IDA: N/A one injected step of 00406950. It sees the globals as the original's callee would, through the global names.
pub type SceneStep<'a> =
    Box<dyn FnMut(&mut World, &SceneGraphGlobals) -> Result<(), StreamError> + 'a>;

/// IDA: N/A inputs and injected steps of 00406950. An empty step is recorded as not yet ported, as the C++ port's empty function is.
pub struct SceneGraphInputs<'a> {
    pub ini: &'a SettingCollection,
    /// 00B06C4C.
    pub renderer_width: u32,
    /// 00B06C50.
    pub renderer_height: u32,
    /// 00B42F48.
    pub render_path: i32,
    /// 00B42F40: `RendererGlobals::shader_flags`, whose owner the injected steps also borrow.
    pub store_shader_flags: Box<dyn FnMut(u32) + 'a>,
    /// g_dx9Renderer is non-null.
    pub renderer_present: bool,
    /// 00B42F3E (05-B).
    pub image_space_effects: u8,
    /// 00B3608F.
    pub lod_far_mode: bool,
    /// 00B333A0.
    pub tes_present: bool,
    /// 007BA0F0.
    pub initialize_shader_manager: Option<SceneStep<'a>>,
    /// 007C2450.
    pub create_render_targets: Option<SceneStep<'a>>,
    /// 004814C0.
    pub load_test_sphere: Option<SceneStep<'a>>,
    /// The process defaults 00707610 merges over. Null leaves the fallback quad pending, as in the C++ port.
    pub property_defaults: Option<ScenePropertyDefaults>,
    /// 00405A30 of 00B333E8 g_imageSpaceRenderedTexture (null at startup).
    pub fallback_base_texture: Option<Entity>,
}

/// IDA: N/A which injected steps had a port to run (order record).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneGraphResult {
    pub pending_steps: Vec<u32>,
    pub fallback_branch: bool,
}

// IDA: N/A host view of one SettingInfo value dword (03-E collection).
fn setting_raw(ini: &SettingCollection, address: u32) -> u32 {
    ini.find_address(address).map_or(0, |setting| setting.raw)
}

// IDA: movzx/mov of the low byte of a bool SettingInfo. Completion: 100%.
fn setting_byte(ini: &SettingCollection, address: u32) -> u8 {
    setting_raw(ini, address) as u8
}

// IDA: N/A a helper, with no address of its own.
fn trace(globals: &mut SceneGraphGlobals, address: u32, width: u8) {
    globals
        .store_trace
        .push(SceneGlobalStore { address, width });
}

// IDA: N/A a host update that reports an unsupported state stops the port.
fn require_updated(result: SceneUpdateResult) -> Result<(), StreamError> {
    if result.status != SceneUpdateStatus::Updated {
        return Err(StreamError(
            "00406950: scene update reported an unsupported state",
        ));
    }
    Ok(())
}

// IDA: N/A no address of its own; its comments cite 00406950.
fn require_attached(status: AttachChildStatus) -> Result<(), StreamError> {
    if status != AttachChildStatus::Attached {
        return Err(StreamError("00406950: AttachChild did not attach"));
    }
    Ok(())
}

// IDA: N/A a NiNode with a name, the shape 00406950 builds nine times (0070B800, 006FF420).
fn spawn_named_node(world: &mut World, name: &[u8]) -> Entity {
    let node = spawn_scene_node(world, SceneNodePayload::default());
    name_object(world, node, name);
    node
}

// IDA: N/A a helper, with no address of its own.
fn name_object(world: &mut World, object: Entity, name: &[u8]) {
    let mut net = world.get_mut::<NiObjectNet>(object).expect("a NiObjectNET");
    set_object_name(&mut net.name, Some(name));
}

// IDA: 00406950 Startup_InitializeSceneGraphs. See each address below; the injected steps run in their original positions.
// Completion: 100% of the scene graph core in original order. An unsupported host update state is an error.
pub fn initialize_scene_graphs(
    world: &mut World,
    globals: &mut SceneGraphGlobals,
    inputs: &mut SceneGraphInputs<'_>,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneGraphResult, StreamError> {
    let ini = inputs.ini;
    let settings = scene_graph_settings_from(
        ini,
        inputs.renderer_width,
        inputs.renderer_height,
        inputs.lod_far_mode,
        inputs.tes_present,
    );
    let mut result = SceneGraphResult::default();

    // 00406977..004069ED World SceneGraph("World", 0, null) -> 00B333CC.
    let world_graph = construct_scene_graph(world, b"World", false, None, &settings, runtime)?;
    globals.world = Some(world_graph);
    trace(globals, 0x00B3_33CC, 4);
    // 004069F3..00406A1B ShadowSceneNode (local ebp).
    let shadow = construct_shadow_scene_node(world);
    globals.shadow_scene = Some(shadow);
    // 00406A1D..00406A62 00B43104 = g_dx9Renderer (NiPointer assign).
    globals.renderer_reference = inputs.renderer_present;
    trace(globals, 0x00B4_3104, 4);
    // 00406A68..00406A81 byte copy of A30048 into 00B42F68.
    globals.shader_directory = "data\\shaders\\".to_owned();
    trace(
        globals,
        0x00B4_2F68,
        (globals.shader_directory.len() + 1) as u8,
    );
    // 00406A83 007BA0F0 (05-D).
    if let Some(step) = inputs.initialize_shader_manager.as_mut() {
        step(world, globals)?;
    } else {
        result.pending_steps.push(0x007B_A0F0);
    }
    // 00406A8B shadow +0x11C = 0; 00406A92 / 00406A9A 007B4270(0, shadow) twice.
    world
        .get_mut::<ShadowSceneNode>(shadow)
        .expect("just built")
        .byte_11c = 0;
    for _ in 0..2 {
        globals.shadow_slots[0] = Some(shadow);
        trace(globals, 0x00B4_2F54, 4);
    }
    // 00406ABD / 00406AC4.
    globals.byte_b42ea7 = 0;
    trace(globals, 0x00B4_2EA7, 1);
    globals.byte_b42e85 = 0;
    trace(globals, 0x00B4_2E85, 1);
    // 00406A9F..00406B28 g_rendererShaderFlags: specular 8, texture 4, diffuse 2, ambient 1 (each `== 1`), 0x10, then canopy 0x20
    // when set and g_renderPath >= 2.
    let bit = |address: u32, value: u32| {
        if setting_byte(ini, address) == 1 {
            value
        } else {
            0
        }
    };
    let mut flags = bit(0x00B0_6CDC, 8) | bit(0x00B0_6CE4, 4);
    flags |= bit(0x00B0_6CD4, 2);
    flags |= bit(0x00B0_6CCC, 1);
    flags |= 0x10;
    let canopy = if setting_byte(ini, 0x00B0_6CBC) != 0 && inputs.render_path >= 2 {
        0x20
    } else {
        0
    };
    (inputs.store_shader_flags)(canopy | flags);
    trace(globals, 0x00B4_2F40, 4);
    // 00406B1C..00406B42 00B2C678: actor shadows 8, static and arch shadows 2.
    let static_shadows = if setting_byte(ini, 0x00B0_6CF4) != 0 {
        2
    } else {
        0
    };
    let actor_shadows = if setting_byte(ini, 0x00B0_6CEC) != 0 {
        8
    } else {
        0
    };
    globals.shadow_flags = static_shadows | actor_shadows;
    trace(globals, 0x00B2_C678, 4);
    // 00406B34 push 00B43104 (007C2450 never reads it), 00406B48 ecx 00B42F50, 00406B4E 007C2450 (05-E-1), then 00406B53 004814C0
    // (05-E-2).
    if let Some(step) = inputs.create_render_targets.as_mut() {
        step(world, globals)?;
    } else {
        result.pending_steps.push(0x007C_2450);
    }
    if let Some(step) = inputs.load_test_sphere.as_mut() {
        step(world, globals)?;
    } else {
        result.pending_steps.push(0x0048_14C0);
    }
    // 00406B58..00406B66.
    globals.float_b44ee4 = 0.0;
    trace(globals, 0x00B4_4EE4, 4);
    globals.reference_classifier = REFERENCE_FORM_TYPE_CLASSIFIER_ADDRESS;
    trace(globals, 0x00B4_2EB4, 4);
    // 00406B70..00406B7B World vtable +0x84 AttachChild(shadow, 0).
    require_attached(attach_scene_child(world, world_graph, Some(shadow), false))?;
    // 00406B7D..00406BCB NiVertexColorProperty: flags 8, `and 0FFEFh; or 28h`.
    let flags = (VertexColorProperty::default().flags_18 & 0xFFEF) | 0x0028;
    let vertex_color = world
        .spawn((
            NiClass::VertexColorProperty,
            NiObjectNet::default(),
            VertexColorProperty { flags_18: flags },
        ))
        .id();
    attach_scene_property(world, shadow, Some(vertex_color))?;
    // 00406BD0..00406C14 NiZBufferProperty: flags 0x0F, `or 3`.
    let flags = ZBufferProperty::default().flags_18 | 0x0003;
    let z_buffer = world
        .spawn((
            NiClass::ZBufferProperty,
            NiObjectNet::default(),
            ZBufferProperty { flags_18: flags },
        ))
        .id();
    attach_scene_property(world, shadow, Some(z_buffer))?;
    // 00406C19..00406CA4 NiWireframeProperty -> 00B333E0, `and 0FFFEh`.
    let mut wireframe_property = WireframeProperty::default();
    let wireframe = world
        .spawn((NiObjectNet::default(), wireframe_property))
        .id();
    globals.wireframe = Some(wireframe);
    trace(globals, 0x00B3_33E0, 4);
    wireframe_property.flags_18 &= 0xFFFE;
    world.entity_mut(wireframe).insert(wireframe_property);
    attach_scene_property(world, shadow, Some(wireframe))?;
    // 00406CA9..00406D3C BSFogProperty -> 00B333E4, `or 1`, 007C8270(1.0).
    let mut fog_property = FogProperty::default();
    let fog = world.spawn((NiObjectNet::default(), fog_property)).id();
    globals.fog = Some(fog);
    trace(globals, 0x00B3_33E4, 4);
    fog_property.flags_18 |= 0x0001;
    fog_property.set_start_fraction(1.0);
    world.entity_mut(fog).insert(fog_property);
    attach_scene_property(world, shadow, Some(fog))?;
    // 00406D41..00406DDA "Sky" NiNode -> 00B333D8, attached to the shadow node.
    let sky = spawn_scene_node(world, SceneNodePayload::default());
    globals.sky = Some(sky);
    trace(globals, 0x00B3_33D8, 4);
    name_object(world, sky, b"Sky");
    require_attached(attach_scene_child(world, shadow, Some(sky), false))?;
    // 00406DDC..00406EA2 "Weather" NiNode -> 00B333DC: z-buffer, wireframe and vertex-color properties (the same objects), then
    // attached.
    let weather = spawn_scene_node(world, SceneNodePayload::default());
    globals.weather = Some(weather);
    trace(globals, 0x00B3_33DC, 4);
    name_object(world, weather, b"Weather");
    attach_scene_property(world, weather, Some(z_buffer))?;
    attach_scene_property(world, weather, Some(wireframe))?;
    attach_scene_property(world, weather, Some(vertex_color))?;
    require_attached(attach_scene_child(world, shadow, Some(weather), false))?;
    // 00406EA4 00405B80 "Menu" (menu mode) -> 00B333D0.
    globals.menu = Some(construct_scene_graph(
        world, b"Menu", true, None, &settings, runtime,
    )?);
    trace(globals, 0x00B3_33D0, 4);
    // 00406EA9 00405C30 "3DMenu" -> 00B333D4.
    globals.menu_3d = Some(construct_scene_graph(
        world, b"3DMenu", false, None, &settings, runtime,
    )?);
    trace(globals, 0x00B3_33D4, 4);
    // 00406EAE..00406F02 BSClearZNode -> 00B333A8 (raw), "LODRoot".
    let lod_root = spawn_clear_z_node(world);
    globals.lod_root = Some(lod_root);
    trace(globals, 0x00B3_33A8, 4);
    name_object(world, lod_root, b"LODRoot");
    require_attached(attach_scene_child(world, shadow, Some(lod_root), false))?;
    // 00406F04..00406F7E "LandLOD": attached, then local translate (0, 0, fLODLandVerticalBias) stored x, y, z.
    let land_lod = spawn_named_node(world, b"LandLOD");
    require_attached(attach_scene_child(world, lod_root, Some(land_lod), false))?;
    world
        .get_mut::<SceneNodePayload>(land_lod)
        .expect("just built")
        .local_transform
        .translation = [0.0, 0.0, float_copy(setting_raw(ini, 0x00B0_2D68))];
    // 00406F81..00406FC9 "DistantRefLOD".
    let distant_lod = spawn_named_node(world, b"DistantRefLOD");
    require_attached(attach_scene_child(
        world,
        lod_root,
        Some(distant_lod),
        false,
    ))?;
    // 00406FCB..0040701E "LOD Trees" under DistantRefLOD -> 00B42D64 (raw).
    let lod_trees = spawn_named_node(world, b"LOD Trees");
    require_attached(attach_scene_child(
        world,
        distant_lod,
        Some(lod_trees),
        false,
    ))?;
    globals.lod_trees = Some(lod_trees);
    trace(globals, 0x00B4_2D64, 4);
    // 00407019..00407072 "ObjectLODRoot" -> 00B333AC (raw).
    let object_lod_root = spawn_scene_node(world, SceneNodePayload::default());
    globals.object_lod_root = Some(object_lod_root);
    trace(globals, 0x00B3_33AC, 4);
    name_object(world, object_lod_root, b"ObjectLODRoot");
    require_attached(attach_scene_child(
        world,
        shadow,
        Some(object_lod_root),
        false,
    ))?;
    // 00407074..004070C7 "MagicProjectileRoot" -> 00B333A4 (raw).
    let projectile_root = spawn_scene_node(world, SceneNodePayload::default());
    globals.magic_projectile_root = Some(projectile_root);
    trace(globals, 0x00B3_33A4, 4);
    name_object(world, projectile_root, b"MagicProjectileRoot");
    require_attached(attach_scene_child(
        world,
        shadow,
        Some(projectile_root),
        false,
    ))?;
    // 004070C9..0040710A World child 0 (the camera node) local translate (0, 0, 128.0) stored x, y, z.
    let camera_node = world
        .get::<NiNodeChildren>(world_graph)
        .expect("a NiNode has child slots")
        .slots_ac
        .first()
        .copied()
        .flatten()
        .ok_or(StreamError("004070E2: the World graph has no child 0"))?;
    world
        .get_mut::<SceneNodePayload>(camera_node)
        .ok_or(StreamError("004070E2: World child 0 is not a NiNode"))?
        .local_transform
        .translation = [0.0, 0.0, 128.0];
    // 00407113 00707610 / 0040711E 00707980: the World graph holds no NiGeometry or NiDynamicEffect, NiNode +0x6C / +0x70 (0070A840 /
    // 0070A900) only recurse and the NiCamera slots are nullsubs, so the passes leave no persistent state. The C++ port omits them;
    // without process defaults this does too.
    if let Some(defaults) = inputs.property_defaults.as_ref() {
        refresh_property_state(world, world_graph, defaults)?;
        refresh_effect_state(world, world_graph)?;
    }
    // 00407131 NiAVObject_Update(World, 0.0, 1).
    require_updated(update_scene_object(world, world_graph, 0.0, true, runtime)?)?;
    // 0040713D..00407143 00B33396 = bStaticMenuBackground.
    globals.static_menu_background = setting_byte(ini, 0x00B0_6DC4);
    trace(globals, 0x00B3_3396, 1);
    // 00407148..00407156: the fallback quad when 00B333EC is null and image-space effects are off (05-E-3; 004071D9 the NiPointer
    // store).
    if globals.fallback_quad.is_none() && inputs.image_space_effects == 0 {
        result.fallback_branch = true;
        if let Some(defaults) = inputs.property_defaults.as_ref() {
            globals.fallback_quad = Some(build_image_space_fallback_quad(
                world,
                defaults,
                inputs.fallback_base_texture,
            )?);
            trace(globals, 0x00B3_33EC, 4);
        } else {
            result.pending_steps.push(0x0040_715C);
        }
    }
    Ok(result)
}

// IDA: WinMain 0040EAEF..0040EB06 (Phase 06 row 06-D-1, D6): the World SceneGraph's camera (+0xDC) copied raw (no refcount) into 00B39B84 and
// 00B39E00. The original reads through the pointer without a null check. Completion: 100%.
pub fn publish_camera_aliases(
    world: &World,
    globals: &SceneGraphGlobals,
    aliases: &mut CameraStaticAliases,
) {
    let camera = globals
        .world
        .and_then(|graph| world.get::<SceneGraph>(graph))
        .and_then(|graph| graph.camera_dc);
    // 0040EAEF..0040EAFA: eax = g_worldSceneGraph; [00B39B84] = [eax+0DCh].
    aliases.face_gen_node_camera = camera;
    // 0040EB00..0040EB06: [00B39E00] = [eax+0DCh] (read again).
    aliases.tree_node_camera = camera;
}

#[cfg(test)]
mod tests {
    use super::*;
    use recomp_core::fixtures;
    use recomp_game::bs::effect_textures::{
        EffectTextureGlobals, EffectTextureInputs, create_effect_textures,
    };
    use recomp_game::bs::render_targets::{
        DepthStencilBuffer, RenderMemoryStatistics, RenderTargetGlobals, RenderTargetServices,
        TextureManager, TextureManagerServices, create_render_targets,
    };
    use recomp_game::bs::scene_graphs::{ClearZNode, SceneCamera};
    use recomp_game::bs::shader_manager::{
        ShaderManagerGlobals, ShaderManagerInputs, initialize_shader_manager,
        texture_manager_without_device,
    };
    use recomp_game::gamebryo::av_object::{NiAvParent, NiAvProperties};
    use recomp_game::gamebryo::dx9_system::{D3DFMT_R5G6B5, D3DFMT_X8R8G8B8};
    use recomp_game::gamebryo::mesh_geometry::NiGeometry;
    use recomp_game::gamebryo::pixel_data::{PixelData, pixel_format_from_d3d};
    use recomp_game::gamebryo::properties::MaterialProperty;
    use recomp_game::gamebryo::registration::{
        NiProcessDefaults, StartupCallbackScheduler, register_known_crt_callbacks,
    };
    use recomp_game::gamebryo::renderer::{
        RenderPathInputs, RendererGlobals, RendererState, select_render_path,
    };
    use recomp_game::gamebryo::scene_node::NiAvWorld;
    use recomp_game::gamebryo::scene_state::ScenePropertyState;
    use recomp_game::gamebryo::scene_update::NoSceneControllers;
    use recomp_game::gamebryo::screen_elements::ScreenElementsQuad;
    use recomp_game::gamebryo::texture::SourceTexture;
    use recomp_game::gamebryo::texture::TexturingProperty;
    use recomp_game::gamebryo::texture_loading::TextureLoadServices;
    use recomp_game::tes::settings::{SettingCollections, register_static_settings};
    use recomp_game::tes::shader_system::ShaderSystemGlobals;
    use std::cell::Cell;
    use std::path::PathBuf;

    use crate::winmain_shader::{ShaderWindowInputs, run_shader_window};

    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // IDA: N/A fixture: the 03-E INI collection read from a fixture INI.
    fn load_collection(temp: &TempRoot, name: &str, text: &str) -> SettingCollections {
        let ini = temp.0.join(name);
        std::fs::write(&ini, text).unwrap();
        let mut collections = SettingCollections::default();
        register_static_settings(&mut collections);
        collections.ini.set_path(&ini);
        assert!(collections.ini.open_ini());
        let _ = collections.ini.read_all();
        collections.ini.close();
        collections
    }

    /// One 00406950 run with no injected steps, at 1920x1080 and render path 5.
    struct SceneRun {
        world: World,
        globals: SceneGraphGlobals,
        result: SceneGraphResult,
        shader_flags: u32,
    }

    fn run_scene(
        ini: &SettingCollection,
        image_space_effects: u8,
        property_defaults: Option<ScenePropertyDefaults>,
        world: Option<World>,
    ) -> SceneRun {
        let mut world = world.unwrap_or_default();
        let mut globals = SceneGraphGlobals::default();
        let shader_flags = Cell::new(0x2F);
        let mut inputs = SceneGraphInputs {
            ini,
            renderer_width: 1920,
            renderer_height: 1080,
            render_path: 5,
            store_shader_flags: Box::new(|flags| shader_flags.set(flags)),
            renderer_present: true,
            image_space_effects,
            lod_far_mode: false,
            tes_present: false,
            initialize_shader_manager: None,
            create_render_targets: None,
            load_test_sphere: None,
            property_defaults,
            fallback_base_texture: None,
        };
        let result = initialize_scene_graphs(
            &mut world,
            &mut globals,
            &mut inputs,
            &mut NoSceneControllers,
        )
        .unwrap();
        drop(inputs);
        SceneRun {
            world,
            globals,
            result,
            shader_flags: shader_flags.get(),
        }
    }

    fn name_of(world: &World, object: Entity) -> &[u8] {
        world
            .get::<NiObjectNet>(object)
            .unwrap()
            .name
            .as_deref()
            .unwrap_or_default()
    }

    fn children(world: &World, node: Entity) -> Vec<Option<Entity>> {
        world.get::<NiNodeChildren>(node).unwrap().slots_ac.clone()
    }

    /// IDA: 00406950 global store order (the injected steps add none here).
    fn scene_trace() -> Vec<SceneGlobalStore> {
        [
            (0x00B3_33CC, 4),
            (0x00B4_3104, 4),
            (0x00B4_2F68, 14),
            (0x00B4_2F54, 4),
            (0x00B4_2F54, 4),
            (0x00B4_2EA7, 1),
            (0x00B4_2E85, 1),
            (0x00B4_2F40, 4),
            (0x00B2_C678, 4),
            (0x00B4_4EE4, 4),
            (0x00B4_2EB4, 4),
            (0x00B3_33E0, 4),
            (0x00B3_33E4, 4),
            (0x00B3_33D8, 4),
            (0x00B3_33DC, 4),
            (0x00B3_33D0, 4),
            (0x00B3_33D4, 4),
            (0x00B3_33A8, 4),
            (0x00B4_2D64, 4),
            (0x00B3_33AC, 4),
            (0x00B3_33A4, 4),
            (0x00B3_3396, 1),
        ]
        .map(|(address, width)| SceneGlobalStore { address, width })
        .to_vec()
    }

    /// `verify_scene_graph_initialization`: 00406950 with the default INI (P05-RUN-001 SCENE_RET values).
    #[test]
    fn verify_scene_graph_initialization() {
        let temp = TempRoot::new("oblivion-recomp-phase05-scenes");
        let defaults = load_collection(&temp, "Empty.ini", "[General]\n");
        let run = run_scene(&defaults.ini, 1, None, None);
        assert_eq!(
            run.result.pending_steps,
            [0x007B_A0F0, 0x007C_2450, 0x0048_14C0],
            "00406950 injected steps differ"
        );
        assert!(!run.result.fallback_branch && run.globals.fallback_quad.is_none());
        assert_eq!(
            run.globals.store_trace,
            scene_trace(),
            "00406950 global store order differs"
        );
        let world = &run.world;
        let globals = &run.globals;
        // P05-RUN-001 SCENE_RET: shaderFlags 3F, B2C678 08, B42EA7 / B42E85 0, B44EE4 0, B33396 1, B42EB4 004E0CF0, "data\shaders\".
        assert_eq!(run.shader_flags, 0x3F);
        assert_eq!(globals.shadow_flags, 8);
        assert_eq!(globals.byte_b42ea7, 0);
        assert_eq!(globals.byte_b42e85, 0);
        assert_eq!(globals.float_b44ee4, 0.0);
        assert_eq!(globals.static_menu_background, 1);
        assert_eq!(globals.reference_classifier, 0x004E_0CF0);
        assert_eq!(globals.shader_directory, "data\\shaders\\");
        assert!(globals.renderer_reference);

        let graph = globals.world.unwrap();
        let slots = children(world, graph);
        assert_eq!(
            slots.len(),
            2,
            "World must hold the camera node and shadow node"
        );
        let camera_node = slots[0].unwrap();
        let shadow = globals.shadow_scene.unwrap();
        assert_eq!(slots[1], Some(shadow));
        let state = world.get::<ShadowSceneNode>(shadow).unwrap();
        assert_eq!(state.original_vtable, 0x00A9_04B4);
        assert_eq!(name_of(world, shadow), b"shadow scene node");
        assert_eq!(state.byte_11c, 0);
        assert!(state.light_110.is_some() && state.light_114.is_some());
        assert_eq!(globals.shadow_slots[0], Some(shadow));
        assert_eq!(
            world
                .get::<SceneNodePayload>(camera_node)
                .unwrap()
                .local_transform
                .translation,
            [0.0, 0.0, 128.0]
        );
        let camera_entity = world.get::<SceneGraph>(graph).unwrap().camera_dc.unwrap();
        assert_eq!(
            world
                .get::<NiAvWorld>(camera_entity)
                .unwrap()
                .transform
                .translation,
            [0.0, 0.0, 128.0],
            "004070FC camera node translate / world update differ"
        );
        // Direction (column 0) is (0, 1, 0), so the translation leaves row 3's w at -0 and row 2's w at -near * depth.
        let camera = world.get::<SceneCamera>(camera_entity).unwrap();
        let depth = camera.frustum_ec.far_plane
            * (1.0 / (camera.frustum_ec.far_plane - camera.frustum_ec.near_plane));
        assert_eq!(camera.world_to_clip_ac[15], 0.0);
        assert_eq!(
            camera.world_to_clip_ac[11],
            -camera.frustum_ec.near_plane * depth
        );

        let names: [&[u8]; 5] = [
            b"Sky",
            b"Weather",
            b"LODRoot",
            b"ObjectLODRoot",
            b"MagicProjectileRoot",
        ];
        let shadow_children = children(world, shadow);
        assert_eq!(shadow_children.len(), names.len());
        for (child, name) in shadow_children.iter().zip(names) {
            let child = child.unwrap();
            assert_eq!(name_of(world, child), name, "shadow node child order");
            assert_eq!(world.get::<NiAvParent>(child).unwrap().0, Some(shadow));
        }
        // AddHead order: the last attached property is the head.
        let properties = &world.get::<NiAvProperties>(shadow).unwrap().0;
        assert_eq!(properties.len(), 4);
        assert_eq!(properties[0], globals.fog);
        assert_eq!(properties[1], globals.wireframe);
        let z_buffer = properties[2].unwrap();
        let vertex = properties[3].unwrap();
        assert_eq!(
            world.get::<ZBufferProperty>(z_buffer).unwrap().flags_18,
            0x000F
        );
        assert_eq!(
            world.get::<VertexColorProperty>(vertex).unwrap().flags_18,
            0x0028
        );
        assert_eq!(
            world
                .get::<WireframeProperty>(globals.wireframe.unwrap())
                .unwrap()
                .flags_18,
            0
        );
        let fog = world.get::<FogProperty>(globals.fog.unwrap()).unwrap();
        assert_eq!(fog.flags_18, 1);
        assert_eq!(
            fog.start_distance_2c.to_bits(),
            0x46D9_999B,
            "P05-RUN-001 fog"
        );
        assert_eq!(fog.end_distance_30, 163_840.0);
        assert_eq!(fog.depth_1c, 1.0);
        let weather = &world
            .get::<NiAvProperties>(globals.weather.unwrap())
            .unwrap()
            .0;
        assert_eq!(*weather, [Some(vertex), globals.wireframe, Some(z_buffer)]);

        let lod_root = globals.lod_root.unwrap();
        let clear = world.get::<ClearZNode>(lod_root).unwrap();
        assert_eq!(clear.original_vtable, 0x00A3_F894);
        assert_eq!((clear.byte_dc, clear.byte_dd), (0, 0));
        assert_eq!(clear.float_e0, 283_840.0);
        assert_eq!(clear.float_e4, 100.0);
        let lod_children = children(world, lod_root);
        assert_eq!(lod_children.len(), 2);
        let land = lod_children[0].unwrap();
        let distant = lod_children[1].unwrap();
        assert_eq!(name_of(world, land), b"LandLOD");
        assert_eq!(
            world
                .get::<SceneNodePayload>(land)
                .unwrap()
                .local_transform
                .translation,
            [0.0; 3]
        );
        assert_eq!(name_of(world, distant), b"DistantRefLOD");
        assert_eq!(children(world, distant), [globals.lod_trees]);
        assert_eq!(name_of(world, globals.lod_trees.unwrap()), b"LOD Trees");
        assert_eq!(globals.object_lod_root, shadow_children[3]);
        assert_eq!(globals.magic_projectile_root, shadow_children[4]);
        let menu = globals.menu.unwrap();
        assert!(world.get::<SceneGraph>(menu).unwrap().menu_mode_e8);
        assert_eq!(name_of(world, menu), b"MenuRoot Node");
        let menu_3d = globals.menu_3d.unwrap();
        assert!(!world.get::<SceneGraph>(menu_3d).unwrap().menu_mode_e8);
        assert_eq!(name_of(world, menu_3d), b"3DMenuRoot Node");

        // 00407136..00407156: the fallback branch with image-space effects off.
        let branch = run_scene(&defaults.ini, 0, None, None);
        assert!(branch.result.fallback_branch);
        assert_eq!(branch.result.pending_steps.last(), Some(&0x0040_715C));
    }

    // IDA: N/A a world whose process defaults exist, as the CRT callbacks leave them.
    fn world_with_defaults() -> (World, ScenePropertyDefaults) {
        let mut world = World::new();
        let mut known = StartupCallbackScheduler::default();
        register_known_crt_callbacks(&mut known).unwrap();
        known.run(&mut world).unwrap();
        let defaults = world
            .resource::<NiProcessDefaults>()
            .scene_property_defaults();
        (world, defaults)
    }

    /// `verify_fallback_quad`: 0040715C..004073B3 the image-space fallback quad (05-E-3) with bDoImageSpaceEffects = 0.
    #[test]
    fn verify_fallback_quad() {
        let temp = TempRoot::new("oblivion-recomp-phase05-fallback");
        let settings = load_collection(&temp, "Empty.ini", "[General]\n");
        // Without the property defaults the step stays pending.
        let skipped = run_scene(&settings.ini, 0, None, None);
        assert!(skipped.result.fallback_branch && skipped.globals.fallback_quad.is_none());
        assert!(skipped.result.pending_steps.contains(&0x0040_715C));

        let (world, defaults) = world_with_defaults();
        let run = run_scene(&settings.ini, 0, Some(defaults), Some(world));
        assert!(run.result.fallback_branch && run.globals.fallback_quad.is_some());
        assert_eq!(
            run.globals.store_trace.last(),
            Some(&SceneGlobalStore {
                address: 0x00B3_33EC,
                width: 4
            }),
            "00406950 must build 00B333EC without image-space effects"
        );
        let world = &run.world;
        let entity = run.globals.fallback_quad.unwrap();
        let quad = world.get::<ScreenElementsQuad>(entity).unwrap();
        // 00702EC0(0, 0, 0, 1, 1), 0072A0F0, 00702FC0(0, 0, 0, 0, 1, 1).
        assert_eq!(quad.polygon_count, 1);
        assert_eq!(quad.vertex_count, 4);
        assert_eq!(quad.index_count, 6);
        assert_eq!(quad.vertices[1], [0.0, 1.0, 0.0]);
        assert_eq!(quad.vertices[3], [1.0, 0.0, 0.0]);
        assert_eq!(quad.texture_coordinates[2], [1.0, 1.0]);
        assert_eq!(quad.bound_center, [0.5, 0.5, 0.0]);
        assert_eq!(quad.bound_radius, 0.5f32.sqrt());
        assert_eq!(
            world.get::<NiAvWorld>(entity).unwrap().bound[3],
            quad.bound_radius
        );
        assert_eq!(quad.dirty_flags, 1);
        // AddHead order: texturing, z-buffer, material, vertex color.
        let properties: Vec<Entity> = world
            .get::<NiAvProperties>(entity)
            .unwrap()
            .0
            .iter()
            .flatten()
            .copied()
            .collect();
        assert_eq!(properties.len(), 4);
        let texturing = world.get::<TexturingProperty>(properties[0]).unwrap();
        let material = world.get::<MaterialProperty>(properties[2]).unwrap();
        assert_eq!(
            world
                .get::<VertexColorProperty>(properties[3])
                .unwrap()
                .flags_18,
            0
        );
        assert_eq!(
            world
                .get::<ZBufferProperty>(properties[1])
                .unwrap()
                .flags_18,
            12
        );
        assert_eq!(material.values.emissive, [1.0; 3]);
        assert_eq!(material.values.ambient, [0.5; 3]);
        assert_eq!(material.raw_state_54, 2);
        assert!(material.material_id_18 > 1);
        assert_eq!(texturing.flags_18, 4);
        let base = texturing.maps_1c[0].unwrap();
        assert_eq!(base.flags_04, 0);
        assert!(base.texture_08.is_none());
        assert!(texturing.maps_1c[1].is_none());
        let state = world.get::<NiGeometry>(entity).unwrap().property_state_ac;
        let state = world.get::<ScenePropertyState>(state.unwrap()).unwrap();
        assert_eq!(
            state.slots[2],
            Some(properties[2]),
            "00707610 merges the material"
        );
        assert_eq!(state.slots[9], Some(properties[1]), "and the z-buffer");
        assert_eq!(state.slots[0], defaults.alpha, "over the default alpha");
    }

    /// `verify_window_predicate_and_readers`, 0040E938 (05-B-4, D1): the branch reads g_rendererHDR 00B43070 as Phase 04 left it (00498340 HDR
    /// predicate, 007B45F0 clears it on render paths 0/1); and the in-window readers of the window's stores: 007B8990 reads 00B42D40,
    /// 00406950 / 007BA0F0 / 007C2450 read 00B42F3E.
    #[test]
    fn verify_window_predicate_and_readers() {
        let temp = TempRoot::new("oblivion-recomp-phase05-readers");
        let defaults = load_collection(&temp, "Empty.ini", "[General]\n");
        let off = load_collection(
            &temp,
            "Oblivion.ini",
            "[Display]\nbDoImageSpaceEffects=0\nbLocalMapShader=0\n",
        );
        // The predicate input: RendererGlobals::hdr after 007B45F0.
        let window_for = |shader_version: u32, ini: &SettingCollection| {
            let mut renderer = RendererGlobals {
                hdr: true, // 004987AE..004987ED: HDR, image-space effects, FP16 blending
                ..RendererGlobals::default()
            };
            select_render_path(
                &mut renderer,
                &RenderPathInputs {
                    pixel_shader_version: shader_version,
                    vertex_shader_version: shader_version,
                    force_1x: false,
                    have_adapter: true,
                    driver: Vec::new(),
                    ps20_instructions: 0,
                },
            );
            let mut window = ShaderSystemGlobals::default();
            run_shader_window(
                &mut window,
                &ShaderWindowInputs {
                    ini,
                    texture_palette: false,
                    renderer_hdr: renderer.hdr,
                },
            );
            (renderer, window)
        };
        let (sm3, hdr_window) = window_for(0x0300, &defaults.ini);
        let (sm1, blur_window) = window_for(0x0101, &defaults.ini);
        assert!(sm3.hdr && sm3.render_path >= 2);
        assert_eq!(hdr_window.bright_clamp, 0.35);
        assert_eq!(hdr_window.blur_passes, 0);
        assert!(!sm1.hdr);
        assert_eq!(sm1.render_path, 1);
        assert_eq!(blur_window.bright_clamp, 0.0);
        assert_eq!(blur_window.blur_texture_size, 256);
        let (_, off_window) = window_for(0x0300, &off.ini);
        assert_eq!(hdr_window.image_space_effects, 1);
        assert_eq!(hdr_window.local_map_shader, 1);
        assert_eq!(off_window.image_space_effects, 0);
        assert_eq!(off_window.local_map_shader, 0);

        // 007B8990 reads 00B42D40: the local-map tint E6 D4 B8 40 or 0 0 0 40.
        let tint_for = |window: &ShaderSystemGlobals| {
            let mut world = World::new();
            let mut textures = EffectTextureGlobals::default();
            let mut services = TextureLoadServices {
                create_renderer_data_on_create: false,
                file_exists: Some(Box::new(|_| false)),
                ..TextureLoadServices::default()
            };
            create_effect_textures(
                &mut world,
                &mut textures,
                &EffectTextureInputs {
                    local_map_shader: window.local_map_shader != 0,
                    image_space_effects: window.image_space_effects != 0,
                    render_path: 2,
                    property_defaults: Some(ScenePropertyDefaults::default()),
                },
                &mut services,
            );
            let tint = world
                .get::<SourceTexture>(textures.local_map_tint.unwrap())
                .unwrap();
            world
                .get::<PixelData>(tint.app_pixel_data_3c.unwrap())
                .unwrap()
                .level(0, 0)[0]
        };
        assert_eq!(tint_for(&hdr_window), 0xE6, "007B8990 must read 00B42D40");
        assert_eq!(tint_for(&off_window), 0);

        // 007BA0F0 reads 00B42F3E: the 00803A30 step only with image-space effects.
        let image_space_step = |window: &ShaderSystemGlobals| {
            let state = RendererState::default();
            let renderer = RendererGlobals {
                render_path: 5,
                ..RendererGlobals::default()
            };
            let built = Cell::new(false);
            let mut inputs = ShaderManagerInputs {
                construct_texture_manager: texture_manager_without_device(&state),
                renderer_globals: &renderer,
                max_texture_blend_stages: 8,
                renderer_reference: true,
                image_space_effects: window.image_space_effects,
                open_file: Box::new(|_| None),
                initialize_screen_element_palette: None,
                create_effect_textures: None,
                create_shader: Some(Box::new(|_, _, _| Ok(()))),
                create_image_space_shaders: Some(Box::new(|_, _| {
                    built.set(true);
                    Ok(())
                })),
            };
            let mut manager = ShaderManagerGlobals::default();
            initialize_shader_manager(&mut World::new(), &mut manager, &mut inputs).unwrap();
            drop(inputs);
            built.get()
        };
        assert!(image_space_step(&hdr_window), "007BA0F0 must read 00B42F3E");
        assert!(!image_space_step(&off_window));

        // 007C2450 reads 00B42F3E: target 10 and the screen flags 0x26 / 0x22.
        let targets_for = |window: &ShaderSystemGlobals| {
            let mut state = RendererState::default();
            state.back_buffer_desc.width = 1920;
            state.back_buffer_desc.height = 1080;
            state.back_buffer_desc.format = D3DFMT_X8R8G8B8;
            state.texture_caps_flags_5e0 = 4;
            state.max_render_targets_894 = 4;
            for table in [1, 3] {
                state.pixel_format_tables[table][5] = Some(pixel_format_from_d3d(21));
                state.pixel_format_tables[table][4] = Some(pixel_format_from_d3d(D3DFMT_R5G6B5));
            }
            let renderer = RendererGlobals {
                hdr: true,
                fp16_filtering: true,
                anisotropic_min_filter: true,
                non_pow2_textures: true,
                texture_formats: [true; 13],
                ..RendererGlobals::default()
            };
            let mut world = World::new();
            let depth = world.spawn(DepthStencilBuffer::default()).id();
            let mut manager = TextureManager {
                depth_stencil_40: Some(depth),
                ..TextureManager::default()
            };
            let mut globals = RenderTargetGlobals::default();
            let (mut b3ff00, mut discard) = (0, 1);
            let mut statistics = RenderMemoryStatistics::default();
            let mut calls = Vec::new();
            let mut services = RenderTargetServices {
                texture: TextureManagerServices::without_device(Some(&state), &mut calls),
                globals: &renderer,
                image_space_effects: window.image_space_effects,
                water_resolution_b45fc8: 128,
                blur_texture_size_b2c2b8: 256,
                format_override_b3ff00: &mut b3ff00,
                depth_discard_b294ec: &mut discard,
                buffer_statistics: &mut statistics,
                warning: None,
            };
            let targets =
                create_render_targets(&mut world, &mut manager, &mut globals, &mut services);
            (
                targets[10].is_some(),
                manager.texture_data_00.last().unwrap().flags_0c,
            )
        };
        assert_eq!(
            targets_for(&hdr_window),
            (true, 0x26),
            "007C2450 must read 00B42F3E"
        );
        assert_eq!(targets_for(&off_window), (false, 0x22));

        // 00406950 reads 00B42F3E: the fallback quad branch only without it.
        let branch = |window: &ShaderSystemGlobals| {
            run_scene(&defaults.ini, window.image_space_effects, None, None)
                .result
                .fallback_branch
        };
        assert!(!branch(&hdr_window), "00406950 must read 00B42F3E");
        assert!(branch(&off_window));
    }

    fn hex_bytes(text: &str) -> Vec<u8> {
        (0..text.len() / 2)
            .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    /// `SCENE|P05-RUN-002|SNAP|<file>|<label>|..` as bytes.
    fn camera_row(file: &str, label: &str) -> Vec<u8> {
        let prefix = format!("SCENE|P05-RUN-002|SNAP|{file}|{label}|");
        let rows = fixtures::rows("phase05");
        let row = rows
            .iter()
            .find(|row| row.starts_with(&prefix))
            .unwrap_or_else(|| panic!("no row {prefix}"));
        hex_bytes(row.rsplit('|').next().unwrap())
    }

    fn le_bytes(values: &[f32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    /// `verify_run002_cameras`: P05-RUN-002's camera values at 0040EAB1 for the same configuration (1920x1080, default INI), bit for bit, with the
    /// expected values lifted to the rows. The hex in the C++ test is these rows.
    #[test]
    fn verify_run002_cameras() {
        let temp = TempRoot::new("oblivion-recomp-phase05-run002");
        let defaults = load_collection(&temp, "Empty.ini", "[General]\n");
        let run = run_scene(&defaults.ini, 1, None, None);
        for (file, graph) in [
            ("camera-world", run.globals.world),
            ("camera-menu", run.globals.menu),
            ("camera-menu3d", run.globals.menu_3d),
        ] {
            let graph = run.world.get::<SceneGraph>(graph.unwrap()).unwrap();
            let camera = run
                .world
                .get::<SceneCamera>(graph.camera_dc.unwrap())
                .unwrap();
            assert_eq!(
                le_bytes(&camera.world_to_clip_ac),
                camera_row(file, "world_to_clip"),
                "{file} world-to-clip"
            );
            let f = camera.frustum_ec;
            assert_eq!(
                le_bytes(&[f.left, f.right, f.top, f.bottom, f.near_plane, f.far_plane]),
                camera_row(file, "frustum"),
                "{file} frustum"
            );
            assert!(!f.orthographic);
            assert_eq!(camera_row(file, "ortho"), [0]);
            assert_eq!(
                le_bytes(&[camera.min_near_plane_108]),
                camera_row(file, "near_min")
            );
            assert_eq!(
                le_bytes(&[camera.max_far_near_ratio_10c]),
                camera_row(file, "ratio")
            );
            let v = camera.viewport_110;
            assert_eq!(
                le_bytes(&[v.left, v.right, v.top, v.bottom]),
                camera_row(file, "viewport"),
                "{file} viewport"
            );
            assert_eq!(le_bytes(&[camera.lod_adjust_120]), camera_row(file, "lod"));
        }
    }

    /// The Phase 06 half: WinMain 0040EAEF..0040EB06 copies the World camera into both aliases.
    #[test]
    fn camera_aliases_name_the_world_camera() {
        let temp = TempRoot::new("oblivion-recomp-phase05-aliases");
        let defaults = load_collection(&temp, "Empty.ini", "[General]\n");
        let run = run_scene(&defaults.ini, 1, None, None);
        let mut aliases = CameraStaticAliases::default();
        publish_camera_aliases(&World::new(), &SceneGraphGlobals::default(), &mut aliases);
        assert_eq!(aliases, CameraStaticAliases::default());
        publish_camera_aliases(&run.world, &run.globals, &mut aliases);
        let camera = run
            .world
            .get::<SceneGraph>(run.globals.world.unwrap())
            .unwrap()
            .camera_dc;
        assert!(camera.is_some());
        assert_eq!(aliases.face_gen_node_camera, camera);
        assert_eq!(aliases.tree_node_camera, camera);
    }
}
