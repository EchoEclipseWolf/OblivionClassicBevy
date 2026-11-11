//! WinMain 0040E89D..0040EAC1: Phase 05's whole window as one sequence. 0040E898..0040EAA6 (`run_shader_window`), 0040EAAC 00406950 with
//! its injected steps bound (007BA0F0 with 007AB1D0, 007B8990, 007B4290(1..27) and 00803A30; 007C2450; 004814C0), and the 0040EAB1..0040EAC1
//! input flush. The C++ port binds the same steps in `src/app/main.cpp`; here the binding is part of startup, and the host hands in only
//! what is its own (files, the input device).

use std::cell::RefCell;
use std::sync::Arc;

use crate::bs::effect_textures::{
    EffectTextureGlobals, EffectTextureInputs, create_effect_textures,
};
use crate::bs::render_targets::{construct_texture_manager, create_render_targets};
use crate::bs::scene_graphs::SceneGraphGlobals;
use crate::bs::shader::{
    BsShaderContext, BsShaderSlots, ImageSpaceShaderList, create_image_space_shader_list,
    get_bs_shader_slot_in_world,
};
use crate::bs::shader_manager::{
    PackageOpen, ShaderManagerGlobals, ShaderManagerInputs, ShaderManagerResult, TestSphereInputs,
    TestSphereStatus, initialize_shader_manager,
};
use crate::bs::shader_programs::{FileTimeService, OpenFileService};
use crate::bs::task_manager::TaskGlobals;
use crate::gamebryo::dx9_system::{Dx9DeviceDesc, Dx9SystemDesc};
use crate::gamebryo::registration::NiProcessDefaults;
use crate::gamebryo::render_targets::{RenderTargetGlobals, RenderTargetServices};
use crate::gamebryo::renderer::{RendererCreation, RendererState};
use crate::gamebryo::scene_update::SceneUpdateRuntime;
use crate::gamebryo::screen_element_palette::initialize_screen_element_palette;
use crate::gamebryo::texture_loading::{
    FileExistsHook, OpenFileHook, PaletteAddHook, PaletteFindHook, TextureLoadServices,
    TextureMemoryStatistics,
};
use crate::tes::os_globals::{
    InputFlushResult, OsGlobalsPlatform, OsInputGlobals, discard_buffered_input_events,
};
use crate::tes::settings::SettingCollection;
use crate::tes::shader_system::ShaderSystemGlobals;
use crate::tes::tes::TesGlobals;
use crate::tes::winmain::load_test_sphere;
use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::tes::winmain::initialize_scene_graphs::{
    SceneGraphInputs, SceneGraphResult, initialize_scene_graphs,
};
use crate::tes::winmain::winmain_shader::{ShaderWindowInputs, run_shader_window};

/// IDA: D3DDEVTYPE_REF, which picks the adapter's reference description (+0x464).
const D3DDEVTYPE_REF: u32 = 2;

/// IDA: N/A what the window takes from the host and from earlier phases. Files and the palette come as hooks, since their owners
/// (the archive manager, the file finder) are the caller's.
pub struct SceneWindowServices<'a> {
    pub ini: &'a SettingCollection,
    /// The result of 0040E865..0040E88E.
    pub creation: &'a mut RendererCreation,
    /// 00B3F709, set at 0040E876.
    pub load_direct_hint: bool,
    /// 00B3F958.
    pub drop_static_app_data: bool,
    /// 00B35300 is non-null (0040E6AD).
    pub texture_palette: bool,
    /// 00B33A04 is non-null.
    pub file_finder: bool,
    /// j_psub_748110(path, 0, 0x8000).
    pub open_texture_file: OpenFileHook<'a>,
    /// j_psub_748170(path, 0).
    pub texture_file_exists: FileExistsHook<'a>,
    /// 00B42EBC vt+4.
    pub palette_find: Option<PaletteFindHook<'a>>,
    /// 00B42EBC vt+8.
    pub palette_add: Option<PaletteAddHook<'a>>,
    /// 00431690, for the shader package.
    pub open_package: PackageOpen<'a>,
    /// 00431130(path, 0, 10240, 0x20), for a shader program outside the package.
    pub open_program: Option<OpenFileService>,
    /// 004315F0(path, &time, 0x20).
    pub program_file_time: Option<FileTimeService>,
    /// 00B42D80.
    pub shader_cache_directory: String,
    /// g_osGlobals +0x20.
    pub os_input: Option<&'a OsInputGlobals>,
    pub os_platform: &'a mut dyn OsGlobalsPlatform,
    pub runtime: &'a mut dyn SceneUpdateRuntime,
}

/// IDA: N/A the globals the window leaves behind, and the host records of how it ran. Each original global has its `Resource` type
/// here; the caller installs what later phases read.
pub struct SceneWindow {
    pub shader_system: ShaderSystemGlobals,
    pub scene_graphs: SceneGraphGlobals,
    pub scene_result: SceneGraphResult,
    pub shader_manager: ShaderManagerGlobals,
    pub manager_result: ShaderManagerResult,
    pub effect_textures: EffectTextureGlobals,
    pub texture_statistics: TextureMemoryStatistics,
    pub shader_context: BsShaderContext,
    pub shader_slots: BsShaderSlots,
    pub image_space_list: Option<ImageSpaceShaderList>,
    pub render_targets: RenderTargetGlobals,
    /// 007C2450's 13 locals.
    pub targets: [Option<Entity>; 13],
    pub test_sphere: Option<TestSphereStatus>,
    pub input_flush: InputFlushResult,
    /// 00B42E8C(text, null) from the texture paths and 007C2450, in call order.
    pub warnings: Vec<Vec<u8>>,
    /// Host record: each step's address, as it was entered.
    pub order: Vec<u32>,
}

// IDA: N/A the renderer's device description, as 007C1FF0's renderer +0x5D0 holds it.
fn device_desc<'s>(
    system: Option<&'s Arc<Dx9SystemDesc>>,
    state: &RendererState,
) -> Option<&'s Dx9DeviceDesc> {
    let adapter = system?.adapter(state.adapter_5bc)?;
    if state.device_type_5c0 == D3DDEVTYPE_REF {
        adapter.reference_464.as_ref()
    } else {
        adapter.hal_460.as_ref()
    }
}

// IDA: N/A the slot context main.cpp sets up before 007BA0F0, from the renderer and the 05-B copies.
fn shader_context(
    creation: &RendererCreation,
    shader_system: &ShaderSystemGlobals,
    cache_directory: String,
    open_program: Option<OpenFileService>,
    file_time: Option<FileTimeService>,
) -> BsShaderContext {
    let g = &creation.globals;
    let mut context = BsShaderContext {
        renderer: creation.succeeded.then(|| Arc::new(creation.state.clone())),
        render_path: g.render_path,
        ps2x_path: g.ps2x_path,
        hdr: g.hdr,
        shaders_30: g.shaders_30,
        fp16_filtering: g.fp16_filtering,
        anisotropic_min_filter: g.anisotropic_min_filter,
        sun_brightness: shader_system.sun_brightness,
        sky_brightness: shader_system.sky_brightness,
        shader_model_class: g.shader_model_class,
        specular_multiplier: shader_system.specular_multiplier,
        emissive_multiplier: shader_system.emissive_multiplier,
        ..BsShaderContext::default()
    };
    context.pools.max_texture_blend_stages = creation.state.max_texture_blend_stages as u16;
    context.pools.max_sampler_stages = creation.state.max_sampler_stages;
    context.programs.use_hard_drive_cache = shader_system.use_hard_drive_cache != 0;
    context.programs.hdr = g.hdr;
    context.programs.cache_directory = cache_directory;
    context.programs.render_path = g.render_path;
    context.programs.ps2x_path = g.ps2x_path;
    context.programs.has_device = creation.device.is_some();
    context.programs.has_renderer = creation.succeeded;
    context.programs.open_file = open_program;
    context.programs.file_time = file_time;
    context
}

// IDA: WinMain 0040E898..0040EAC1, with 00406950's and 007BA0F0's steps bound in their original positions. 0040E89D Startup_LogInfo is
// obsolete, as in `run_shader_window`. 00B3608F and 00B333A0 are still zero here: their writers run in later phases.
// Completion: 100% of the order; each step's completion is at its own port.
// TODO: [ ] 00403160's caller-side `esi = g_osGlobals`, when the startup sequence owns `OsGlobalsData`.
pub fn run_scene_window(
    world: &mut World,
    services: SceneWindowServices<'_>,
) -> Result<SceneWindow, GameError> {
    let SceneWindowServices {
        ini,
        creation,
        load_direct_hint,
        drop_static_app_data,
        texture_palette,
        file_finder,
        open_texture_file,
        texture_file_exists,
        palette_find,
        palette_add,
        open_package,
        open_program,
        program_file_time,
        shader_cache_directory,
        os_input,
        os_platform,
        runtime,
    } = services;

    // 0040E898..0040EAA6.
    let mut shader_system = ShaderSystemGlobals::default();
    run_shader_window(
        &mut shader_system,
        &ShaderWindowInputs {
            ini,
            texture_palette,
            renderer_hdr: creation.globals.hdr,
        },
    );
    let image_space_effects = shader_system.image_space_effects;
    let property_defaults = world
        .get_resource::<NiProcessDefaults>()
        .map(NiProcessDefaults::scene_property_defaults);

    let context = RefCell::new(shader_context(
        creation,
        &shader_system,
        shader_cache_directory,
        open_program,
        program_file_time,
    ));
    let (width, height, render_path, succeeded) = (
        creation.globals.width,
        creation.globals.height,
        creation.globals.render_path,
        creation.succeeded,
    );
    let max_texture_blend_stages = creation.state.max_texture_blend_stages;
    let creation = RefCell::new(creation);
    let manager = RefCell::new(ShaderManagerGlobals::default());
    let effect_textures = RefCell::new(EffectTextureGlobals::default());
    let statistics = RefCell::new(TextureMemoryStatistics::default());
    let slots = RefCell::new(BsShaderSlots::default());
    let image_space_list = RefCell::new(None);
    let render_targets =
        RefCell::new(crate::gamebryo::render_targets::RenderTargetGlobals::default());
    let targets = RefCell::new([None; 13]);
    let test_sphere = RefCell::new(None);
    let warnings = RefCell::new(Vec::new());
    let order = RefCell::new(Vec::new());
    // The one-shot services 007B8990 and 007BA0F0 consume.
    let texture_hooks = RefCell::new(Some((
        open_texture_file,
        texture_file_exists,
        palette_find,
        palette_add,
    )));
    let open_package = RefCell::new(Some(open_package));

    let tes_present = world
        .get_resource::<TesGlobals>()
        .is_some_and(|globals| globals.tes.is_some());
    let model_loader = world
        .get_resource::<TaskGlobals>()
        .is_some_and(|tasks| tasks.model_loader_b33a1c.is_some());
    let mut scene_graphs = SceneGraphGlobals::default();
    let manager_result = RefCell::new(ShaderManagerResult::default());
    let scene_result = {
        let mut inputs = SceneGraphInputs {
            ini,
            renderer_width: width,
            renderer_height: height,
            render_path,
            store_shader_flags: Box::new(|flags| {
                creation.borrow_mut().globals.shader_flags = flags;
            }),
            renderer_present: succeeded,
            image_space_effects,
            // 00B3608F (004E9F40) has no writer before Phase 14's world, and 00B333A0 is written in the Phase 09 TES block, after this read: each reads its owner, at whichever moment this runs.
            lod_far_mode: false,
            tes_present,
            // 00406A83 007BA0F0.
            initialize_shader_manager: Some(Box::new(|world, scene| {
                order.borrow_mut().push(0x007B_A0F0);
                let renderer_globals = creation.borrow().globals.clone();
                let mut inputs = ShaderManagerInputs {
                    // 007BA122 007C1FF0.
                    construct_texture_manager: Box::new(|world, discard, memory| {
                        let mut creation = creation.borrow_mut();
                        let system = creation.system.clone();
                        let desc = device_desc(system.as_ref(), &creation.state);
                        let mut services =
                            crate::gamebryo::render_targets::TextureManagerServices::from_creation(
                                &mut creation,
                                desc,
                            );
                        construct_texture_manager(world, &mut services, discard, memory)
                    }),
                    renderer_globals: &renderer_globals,
                    max_texture_blend_stages,
                    renderer_reference: scene.renderer_reference,
                    image_space_effects,
                    open_file: open_package
                        .borrow_mut()
                        .take()
                        .ok_or(GameError::unclassified(0x007BA0F0, "007BA0F0 ran twice"))?,
                    // 007BA1FE 007AB1D0(1).
                    initialize_screen_element_palette: Some(Box::new(|world, _| {
                        order.borrow_mut().push(0x007A_B1D0);
                        initialize_screen_element_palette(world)
                    })),
                    // 007BA208 007B8990.
                    create_effect_textures: Some(Box::new(|world, _| {
                        order.borrow_mut().push(0x007B_8990);
                        let (open, exists, find, add) = texture_hooks
                            .borrow_mut()
                            .take()
                            .ok_or(GameError::unclassified(0x007B8990, "007B8990 ran twice"))?;
                        let mut creation = creation.borrow_mut();
                        let g = &creation.globals;
                        let (create_on_create, mip_skip, mip_minimum) = (
                            g.texture_create_renderer_data,
                            g.mip_skip as u32,
                            g.mip_minimum as u32,
                        );
                        let mut statistics = statistics.borrow_mut();
                        let mut services = TextureLoadServices {
                            open_file: Some(open),
                            file_exists: Some(exists),
                            palette_find: find,
                            palette_add: add,
                            // 00B42E8C 00405150(text, null).
                            warning: Some(Box::new(|text| {
                                warnings.borrow_mut().push(text.to_vec());
                            })),
                            renderer: Some(&mut **creation),
                            mip_limits: Some((mip_skip, mip_minimum)),
                            create_renderer_data_on_create: create_on_create,
                            drop_static_app_data,
                            direct_to_renderer_hint: load_direct_hint,
                            statistics: Some(&mut statistics),
                        };
                        create_effect_textures(
                            world,
                            &mut effect_textures.borrow_mut(),
                            &EffectTextureInputs {
                                local_map_shader: shader_system.local_map_shader != 0,
                                image_space_effects: image_space_effects != 0,
                                render_path,
                                property_defaults,
                            },
                            &mut services,
                        );
                        Ok(())
                    })),
                    // 007BA254..007BA26F 007B4290(1..27).
                    create_shader: Some(Box::new(|world, manager, slot| {
                        order.borrow_mut().push(0x007B_4290);
                        let mut context = context.borrow_mut();
                        // The package 00B430B8 is live only inside 007BA0F0, and 00B42F68 is 00406A68's.
                        context.programs.package.clone_from(&manager.package);
                        context
                            .programs
                            .shader_directory
                            .clone_from(&scene.shader_directory);
                        // 0076C910 reads the effect globals; 007B8990 has filled them by now.
                        let textures = effect_textures.borrow().clone();
                        context.effect_texture =
                            Some(Box::new(move |address| textures.stage_texture(address)));
                        let _ = get_bs_shader_slot_in_world(
                            world,
                            &mut slots.borrow_mut(),
                            slot as u32,
                            &mut context,
                        )?;
                        Ok(())
                    })),
                    // 007BA2A7..007BA2CC FormHeap(0x1C) 00803A30 into 00B42D7C.
                    create_image_space_shaders: Some(Box::new(|_, _| {
                        order.borrow_mut().push(0x0080_3A30);
                        let refraction = creation.borrow().globals.refraction;
                        *image_space_list.borrow_mut() = Some(create_image_space_shader_list(
                            &mut slots.borrow_mut(),
                            &mut context.borrow_mut(),
                            refraction,
                        ));
                        Ok(())
                    })),
                };
                let mut manager = manager.borrow_mut();
                *manager_result.borrow_mut() =
                    initialize_shader_manager(world, &mut manager, &mut inputs)?;
                Ok(())
            })),
            // 00406B4E 007C2450 on 00B42F50.
            create_render_targets: Some(Box::new(|world, _| {
                order.borrow_mut().push(0x007C_2450);
                let mut creation = creation.borrow_mut();
                let globals = creation.globals.clone();
                let system = creation.system.clone();
                let desc = device_desc(system.as_ref(), &creation.state);
                let mut context = context.borrow_mut();
                let water_resolution_b45fc8 = context.globals.water_resolution_b45fc8;
                let mut manager = manager.borrow_mut();
                let manager = &mut *manager;
                // 007C2450 would dereference a null 00B42F50; 007BA0F0 always stores one.
                let Some(texture_manager) = manager.texture_manager.as_mut() else {
                    return Ok(());
                };
                let mut services = RenderTargetServices {
                    texture: crate::gamebryo::render_targets::TextureManagerServices::from_creation(
                        &mut creation,
                        desc,
                    ),
                    globals: &globals,
                    image_space_effects,
                    water_resolution_b45fc8,
                    blur_texture_size_b2c2b8: shader_system.blur_texture_size,
                    format_override_b3ff00: &mut context.globals.water_b3ff00,
                    depth_discard_b294ec: &mut manager.depth_discard_flag,
                    buffer_statistics: &mut manager.memory,
                    warning: Some(Box::new(|text| {
                        warnings.borrow_mut().push(text.to_vec());
                    })),
                };
                *targets.borrow_mut() = create_render_targets(
                    world,
                    texture_manager,
                    &mut render_targets.borrow_mut(),
                    &mut services,
                );
                Ok(())
            })),
            // 00406B53 004814C0: 00B33A1C is written by TES_Constructor (Phase 09), so on the startup path the NIF branch is not reached.
            load_test_sphere: Some(Box::new(|_, _| {
                order.borrow_mut().push(0x0048_14C0);
                *test_sphere.borrow_mut() = Some(load_test_sphere(
                    &mut manager.borrow_mut(),
                    &mut TestSphereInputs {
                        max_texture_blend_stages,
                        model_loader,
                        file_finder,
                        find_file: None,
                    },
                ));
                Ok(())
            })),
            property_defaults,
            fallback_base_texture: None,
        };
        // 0040EAAC.
        initialize_scene_graphs(world, &mut scene_graphs, &mut inputs, runtime)?
    };

    // 0040EAB1..0040EAC1: esi = g_osGlobals; `jz` on a null input, else 00403160.
    let input_flush = discard_buffered_input_events(os_input, os_platform);
    let mut order = order.into_inner();
    if input_flush.called {
        order.push(0x0040_3160);
    }

    Ok(SceneWindow {
        shader_system,
        scene_graphs,
        scene_result,
        shader_manager: manager.into_inner(),
        manager_result: manager_result.into_inner(),
        effect_textures: effect_textures.into_inner(),
        texture_statistics: statistics.into_inner(),
        shader_context: context.into_inner(),
        shader_slots: slots.into_inner(),
        image_space_list: image_space_list.into_inner(),
        render_targets: render_targets.into_inner(),
        targets: targets.into_inner(),
        test_sphere: test_sphere.into_inner(),
        input_flush,
        warnings: warnings.into_inner(),
        order,
    })
}
