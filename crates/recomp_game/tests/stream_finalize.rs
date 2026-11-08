//! Ported from the C++ `tests/gamebryo/stream_finalize_tests.cpp`.

use std::cell::{Cell, RefCell};

use bevy_ecs::prelude::*;
use recomp_game::gamebryo::animation::FloatKeyData;
use recomp_game::gamebryo::particle_controller::ParticleEmitterController;
use recomp_game::gamebryo::particle_dynamics::ParticleModifierPending;
use recomp_game::gamebryo::particle_emitter::ParticleEmitterPending;
use recomp_game::gamebryo::particle_mesh_emitter::{
    ParticleMeshEmitter, ParticleMeshEmitterPending, resolve_particle_mesh_emitter_link,
};
use recomp_game::gamebryo::particle_system::{ParticleSystem, spawn_particle_system};
use recomp_game::gamebryo::properties::new_material_property;
use recomp_game::gamebryo::stream_callbacks::{
    StreamRootCallbackControl, StreamRootCallbackRuntime, StreamRootCallbackStatus,
    dispatch_stream_root_callbacks,
};
use recomp_game::gamebryo::stream_graph::StreamGraph;
use recomp_game::gamebryo::stream_objects::{NiClass, StreamObjectTable};
use recomp_game::gamebryo::stream_postload::{
    StreamFinalizeResult, StreamPostLoadRuntime, StreamPostLoadStatus, dispatch_stream_post_load,
    finalize_stream_graph,
};
use recomp_game::gamebryo::stream_roots::StreamRoots;

const V: u32 = 0x1400_0004;

fn float_data(world: &mut World) -> Entity {
    world
        .spawn((NiClass::FloatKeyData, FloatKeyData::default()))
        .id()
}

fn make_graph(world: &mut World) -> (StreamGraph, Entity) {
    let owner = float_data(world);
    let mut graph = StreamGraph::default();
    graph.table.push("NiFloatData", 0x1234_5678, owner);
    graph.roots = StreamRoots {
        original_indices: vec![0, 0xFFFF_FFFF, 0],
        owners: vec![Some(owner), None, Some(owner)],
        bytes_consumed: 0,
    };
    (graph, owner)
}

#[test]
fn postload_order_pause_and_registry_reload() {
    let mut world = World::new();
    let (graph, owner) = make_graph(&mut world);
    let order = RefCell::new(Vec::new());
    let mut callbacks = Vec::new();
    let mut next = 0;
    let registry_reads = Cell::new(0);

    let mut wait = |state: &mut u32| {
        order.borrow_mut().push('W');
        assert_eq!(*state, 4, "the pause wait saw the wrong stream state");
    };
    let mut control = StreamRootCallbackControl {
        load_state: 3,
        wait_for_resume: Some(&mut wait),
    };
    let mut unported = |_: &mut World, entry: &_, _: u32| {
        order.borrow_mut().push('P');
        let entry: &recomp_game::gamebryo::stream_objects::StreamObjectEntry = entry;
        assert_eq!(
            entry.original_constructor, 0x1234_5678,
            "wrong PostLoad object"
        );
        true
    };
    let mut post = StreamPostLoadRuntime {
        invoke_unported: Some(&mut unported),
        ..StreamPostLoadRuntime::default()
    };
    let mut current = |_: &World| {
        registry_reads.set(registry_reads.get() + 1);
        if registry_reads.get() == 1 {
            vec![0x1111_1111]
        } else {
            vec![0x2222_2222]
        }
    };
    let mut invoke = |_: &mut World, original: u32, root: Entity, _: u32| {
        order.borrow_mut().push('C');
        callbacks.push(original);
        assert_eq!(root, owner, "repeated roots lost their identity");
        true
    };
    let mut callback = StreamRootCallbackRuntime {
        current_callbacks: Some(&mut current),
        invoke: Some(&mut invoke),
    };
    let result = finalize_stream_graph(
        &mut world,
        &graph,
        V,
        &mut next,
        &mut control,
        &mut post,
        &mut callback,
    )
    .unwrap();
    let StreamFinalizeResult::Callbacks(result) = result else {
        panic!("the callback tail did not run");
    };
    assert_eq!(result.status, StreamRootCallbackStatus::Complete);
    assert_eq!(next, 1);
    assert_eq!(*order.borrow(), ['W', 'P', 'C', 'C']);
    assert_eq!(callbacks, [0x1111_1111, 0x2222_2222]);
    assert_eq!(registry_reads.get(), 3);
}

#[test]
fn unsupported_and_cancel() {
    let mut world = World::new();
    let (graph, _) = make_graph(&mut world);
    let mut next = 0;
    let mut control = StreamRootCallbackControl::default();
    let callbacks = Cell::new(0);
    let mut current = |_: &World| vec![0xDEAD_BEEF];
    let mut invoke = |_: &mut World, _: u32, _: Entity, _: u32| {
        callbacks.set(callbacks.get() + 1);
        false
    };
    let mut runtime = StreamRootCallbackRuntime {
        current_callbacks: Some(&mut current),
        invoke: Some(&mut invoke),
    };
    let unsupported = finalize_stream_graph(
        &mut world,
        &graph,
        V,
        &mut next,
        &mut control,
        &mut StreamPostLoadRuntime::default(),
        &mut runtime,
    )
    .unwrap();
    let StreamFinalizeResult::PostLoad(unsupported) = unsupported else {
        panic!("an unported PostLoad was skipped before root callbacks");
    };
    assert_eq!(unsupported.status, StreamPostLoadStatus::UnsupportedObject);
    assert_eq!(unsupported.object_index, Some(0));
    assert_eq!(next, 0);
    assert_eq!(callbacks.get(), 0);

    let mut unported = |_: &mut World, _: &_, _: u32| true;
    let mut post = StreamPostLoadRuntime {
        invoke_unported: Some(&mut unported),
        ..StreamPostLoadRuntime::default()
    };
    control.load_state = 2;
    let cancelled = finalize_stream_graph(
        &mut world,
        &graph,
        V,
        &mut next,
        &mut control,
        &mut post,
        &mut runtime,
    )
    .unwrap();
    let StreamFinalizeResult::Callbacks(cancelled) = cancelled else {
        panic!("state 2 cancelled the PostLoad pass");
    };
    assert_eq!(cancelled.status, StreamRootCallbackStatus::Cancelled);
    assert_eq!(next, 1);
    assert_eq!(callbacks.get(), 0);

    control.load_state = 0;
    let missing =
        dispatch_stream_root_callbacks(&mut world, &graph.roots, V, &mut control, &mut runtime)
            .unwrap();
    assert_eq!(
        missing.status,
        StreamRootCallbackStatus::UnsupportedCallback
    );
    assert_eq!(missing.root_index, Some(0));
    assert_eq!(missing.callback_slot, Some(0));
    assert_eq!(missing.original_callback, Some(0xDEAD_BEEF));
    assert_eq!(callbacks.get(), 1);
}

#[test]
fn builtin_postload_and_pause_cancel_tail() {
    let mut world = World::new();
    let mut table = StreamObjectTable::default();
    let data = float_data(&mut world);
    table.push("NiFloatData", 0x006E_3440, data);
    let material = new_material_property(&mut world);
    // The C++ fixture labels 00718F80 "NiMaterialProperty"; that address is NiStencilProperty's. Both are 00700160.
    let property = world.spawn((NiClass::MaterialProperty, material)).id();
    table.push("NiMaterialProperty", 0x0071_8F80, property);
    let unknown = float_data(&mut world);
    table.push("Unknown", 0x1234_5678, unknown);
    let mut control = StreamRootCallbackControl::default();
    let mut next = 0;
    let unsupported = dispatch_stream_post_load(
        &mut world,
        &table,
        V,
        &mut next,
        &mut control,
        &mut StreamPostLoadRuntime::default(),
    )
    .unwrap();
    assert_eq!(unsupported.status, StreamPostLoadStatus::UnsupportedObject);
    assert_eq!(unsupported.object_index, Some(2));
    assert_eq!(next, 2);

    let fallback_calls = Cell::new(0);
    let mut unported = |_: &mut World, entry: &_, _: u32| {
        fallback_calls.set(fallback_calls.get() + 1);
        let entry: &recomp_game::gamebryo::stream_objects::StreamObjectEntry = entry;
        assert_eq!(
            entry.original_constructor, 0x1234_5678,
            "a built-in PostLoad leaked into the unported adapter"
        );
        true
    };
    let mut adapter = StreamPostLoadRuntime {
        invoke_unported: Some(&mut unported),
        ..StreamPostLoadRuntime::default()
    };
    let resumed =
        dispatch_stream_post_load(&mut world, &table, V, &mut next, &mut control, &mut adapter)
            .unwrap();
    assert_eq!(resumed.status, StreamPostLoadStatus::Complete);
    assert_eq!(next, 3);
    assert_eq!(fallback_calls.get(), 1);

    let invoked = Cell::new(0);
    let mut current = |_: &World| vec![0x006D_43E0];
    let mut invoke = |_: &mut World, _: u32, _: Entity, _: u32| {
        invoked.set(invoked.get() + 1);
        true
    };
    let mut runtime = StreamRootCallbackRuntime {
        current_callbacks: Some(&mut current),
        invoke: Some(&mut invoke),
    };
    let (graph, _) = make_graph(&mut world);
    let waits = Cell::new(0);
    let mut wait = |state: &mut u32| {
        waits.set(waits.get() + 1);
        *state = 2;
    };
    let mut control = StreamRootCallbackControl {
        load_state: 3,
        wait_for_resume: Some(&mut wait),
    };
    let cancelled =
        dispatch_stream_root_callbacks(&mut world, &graph.roots, V, &mut control, &mut runtime)
            .unwrap();
    assert_eq!(cancelled.status, StreamRootCallbackStatus::Cancelled);
    assert_eq!(waits.get(), 1);
    assert_eq!(invoked.get(), 2, "post-wait root callbacks still run");
}

#[test]
fn emitter_controller_postload_cache() {
    let mut world = World::new();
    let system = spawn_particle_system(&mut world);
    let mut emitter = ParticleMeshEmitter::default();
    let fields = ParticleMeshEmitterPending {
        base: ParticleEmitterPending {
            base: ParticleModifierPending {
                name: Some(b"Mesh".to_vec()),
                order: 4,
                ..ParticleModifierPending::default()
            },
            ..ParticleEmitterPending::default()
        },
        ..ParticleMeshEmitterPending::default()
    };
    resolve_particle_mesh_emitter_link(&mut emitter, &fields, 0).unwrap();
    let mesh = world.spawn((NiClass::ParticleMeshEmitter, emitter)).id();
    let mut state = world.get::<ParticleSystem>(system).unwrap().clone();
    state.attach_modifier(&world, mesh).unwrap();
    *world.get_mut::<ParticleSystem>(system).unwrap() = state;

    let mut controller = ParticleEmitterController {
        modifier_name_40: Some(b"Mesh".to_vec()),
        ..ParticleEmitterController::default()
    };
    controller.time.target_30 = Some(system);
    controller.time.target_linked = true;
    assert!(!controller.cached_modifier_linked);
    assert_eq!(controller.cached_modifier_44, None);
    let controller = world
        .spawn((NiClass::ParticleEmitterController, controller))
        .id();
    let mut table = StreamObjectTable::default();
    table.push("NiPSysEmitterCtlr", 0x0074_FDE0, controller);
    let mut next = 0;
    let mut control = StreamRootCallbackControl::default();
    let result = dispatch_stream_post_load(
        &mut world,
        &table,
        V,
        &mut next,
        &mut control,
        &mut StreamPostLoadRuntime::default(),
    )
    .unwrap();
    assert_eq!(result.status, StreamPostLoadStatus::Complete);
    assert_eq!(next, 1);
    let state = world.get::<ParticleEmitterController>(controller).unwrap();
    assert!(state.cached_modifier_linked);
    assert_eq!(state.cached_modifier_44, Some(mesh));

    let mut state = world
        .get_mut::<ParticleEmitterController>(controller)
        .unwrap();
    state.cached_modifier_linked = false;
    state.cached_modifier_44 = None;
    state.time.target_30 = None;
    next = 0;
    let result = dispatch_stream_post_load(
        &mut world,
        &table,
        V,
        &mut next,
        &mut control,
        &mut StreamPostLoadRuntime::default(),
    )
    .unwrap();
    assert_eq!(result.status, StreamPostLoadStatus::Complete);
    let state = world.get::<ParticleEmitterController>(controller).unwrap();
    assert!(!state.cached_modifier_linked);
    assert_eq!(state.cached_modifier_44, None);
}
