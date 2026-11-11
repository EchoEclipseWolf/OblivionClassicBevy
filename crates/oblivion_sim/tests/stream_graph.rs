//! Ported from the C++ `tests/gamebryo/stream_graph_tests.cpp`, one test per `verify_*`, plus the host rules of a failed
//! load. A C++ `invalid_argument` is `StreamGraphErrorCode::Stream`, and a failed load leaves no object behind.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_sim::gamebryo::animation::{
    FloatInterpolator, FloatKeyData, update_float_interpolator,
};
use oblivion_sim::gamebryo::bool_animation::{
    BoolKeyData, BoolTimelineInterpolator, update_bool_timeline_interpolator,
};
use oblivion_sim::gamebryo::color_animation::ColorKeyData;
use oblivion_sim::gamebryo::extra_data::TextKeyExtraData;
use oblivion_sim::gamebryo::object_net::{AlphaProperty, NiExtraDataName, NiObjectNet};
use oblivion_sim::gamebryo::particle_color::{ParticleColorModifier, ParticleColorSample};
use oblivion_sim::gamebryo::particle_controller::{
    ParticleEmitterController, ParticleUpdateController,
};
use oblivion_sim::gamebryo::particle_data::{ParticleData, ParticleInfo};
use oblivion_sim::gamebryo::particle_drag::ParticleDragModifier;
use oblivion_sim::gamebryo::particle_dynamics::{
    ParticleAgeDeathModifier, ParticleBoundUpdateModifier, ParticleGrowFadeModifier,
    ParticlePositionModifier,
};
use oblivion_sim::gamebryo::particle_gravity::ParticleGravityModifier;
use oblivion_sim::gamebryo::particle_mesh_emitter::ParticleMeshEmitter;
use oblivion_sim::gamebryo::particle_rotation::ParticleRotationModifier;
use oblivion_sim::gamebryo::particle_spawn::{ParticleSpawnModifier, ParticleSpawnServices};
use oblivion_sim::gamebryo::particle_system::ParticleSystem;
use oblivion_sim::gamebryo::position_animation::{PositionKeyData, PositionKeyType};
use oblivion_sim::gamebryo::properties::{
    MaterialProperty, StencilProperty, VertexColorProperty, ZBufferProperty,
};
use oblivion_sim::gamebryo::registration::{FactoryCreate, NiFactoryRegistry};
use oblivion_sim::gamebryo::scene_node::NiAvWorld;
use oblivion_sim::gamebryo::stream_callbacks::{
    StreamRootCallbackControl, StreamRootCallbackRuntime,
};
use oblivion_sim::gamebryo::stream_file_pipeline::{
    StreamFileCursors, StreamFilePipelineErrorStage, StreamFilePipelineRuntime, StreamOwnerArray,
    StreamTerminalServices, StreamTerminalState, load_supported_stream_file,
};
use oblivion_sim::gamebryo::stream_graph::{
    StreamGraph, StreamGraphError, StreamGraphErrorCode, StreamGraphLinkRuntime,
    load_stream_graph_payloads, load_stream_graph_with_groups, load_stream_graph_with_roots,
};
use oblivion_sim::gamebryo::stream_objects::{NiClass, StreamObjectErrorCode};
use oblivion_sim::gamebryo::stream_postload::StreamPostLoadRuntime;
use oblivion_sim::gamebryo::stream_roots::read_stream_roots;
use oblivion_sim::gamebryo::texture::{SourceTexture, SourceTextureLoadServices};

const NULL: u32 = 0xFFFF_FFFF;
const V: u32 = 0x1400_0004;

fn u16_le(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend(value.to_le_bytes());
}

fn u32_le(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_le_bytes());
}

fn name(bytes: &mut Vec<u8>, text: &str) {
    u32_le(bytes, text.len() as u32);
    bytes.extend(text.as_bytes());
}

fn table(names: &[&str], indices: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::new();
    u16_le(&mut bytes, names.len() as u16);
    for text in names {
        name(&mut bytes, text);
    }
    for &index in indices {
        u16_le(&mut bytes, index);
    }
    bytes
}

fn registry(entries: &[(&str, u32)]) -> NiFactoryRegistry {
    let mut registry = NiFactoryRegistry::default();
    for (key, constructor) in entries {
        registry.insert_or_assign(key, *constructor);
    }
    registry
}

fn objects(world: &mut World) -> usize {
    world.query::<&NiClass>().iter(world).count()
}

fn load(
    world: &mut World,
    registry: &NiFactoryRegistry,
    table: &[u8],
    payload: &[u8],
    count: u32,
    version: u32,
) -> Result<StreamGraph, StreamGraphError> {
    load_stream_graph_payloads(
        world,
        registry,
        table,
        payload,
        count,
        version,
        &mut StreamGraphLinkRuntime::default(),
    )
}

/// A C++ `invalid_argument`, and nothing left behind.
fn assert_thrown(
    world: &mut World,
    registry: &NiFactoryRegistry,
    table: &[u8],
    payload: &[u8],
    count: u32,
    version: u32,
) {
    let error = load(world, registry, table, payload, count, version).unwrap_err();
    assert!(
        matches!(error.code, StreamGraphErrorCode::Stream(_)),
        "{:?}",
        error.code
    );
    assert_eq!(objects(world), 0, "a failed load publishes nothing");
}

fn entity(graph: &StreamGraph, index: usize) -> Entity {
    graph.table.objects[index].entity
}

/// 00713FF0 object creation, 006D2F60's forward link and 006E3690's key data.
#[test]
fn float_forward_graph() {
    let mut world = World::new();
    let registry = registry(&[
        ("NiFloatInterpolator", 0x006D_2DD0),
        ("NiFloatData", 0x006E_3440),
    ]);
    let table = table(&["NiFloatInterpolator", "NiFloatData"], &[0, 1]);
    let mut payload = Vec::new();
    u32_le(&mut payload, 0); // cached float scalar 0.0
    u32_le(&mut payload, 1); // forward link to object 1
    u32_le(&mut payload, 1); // data key count
    u32_le(&mut payload, 1); // linear
    u32_le(&mut payload, 0); // time 0.0
    u32_le(&mut payload, 0x40E0_0000); // value 7.0
    let consumed = payload.len();
    payload.push(0xAB);
    let graph = load(&mut world, &registry, &table, &payload, 2, V).unwrap();
    assert_eq!(graph.table.bytes_consumed, table.len());
    assert_eq!(graph.payload_bytes_consumed, consumed);
    assert_eq!(graph.table.objects.len(), 2);
    assert_eq!(graph.table.objects[0].requested_name, "NiFloatInterpolator");
    let (interpolator, data) = (entity(&graph, 0), entity(&graph, 1));
    assert_eq!(
        world
            .get::<FloatInterpolator>(interpolator)
            .unwrap()
            .data_10,
        Some(data)
    );
    let keys = world.get::<FloatKeyData>(data).unwrap().keys();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].value, 7.0);
    let mut value = 0.0;
    assert!(update_float_interpolator(
        &mut world,
        interpolator,
        0.0,
        &mut value
    ));
    assert_eq!(value, 7.0);
}

/// 006E7B50's timeline payload and 006E8A30's bool keys.
#[test]
fn bool_timeline_graph() {
    let mut world = World::new();
    let registry = registry(&[
        ("NiBoolTimelineInterpolator", 0x006E_7E70),
        ("NiBoolData", 0x006E_8740),
    ]);
    let table = table(&["NiBoolTimelineInterpolator", "NiBoolData"], &[0, 1]);
    let mut payload = vec![0]; // timeline raw scalar
    u32_le(&mut payload, 1); // forward link
    u32_le(&mut payload, 1); // bool key count
    u32_le(&mut payload, 5); // step key type
    u32_le(&mut payload, 0); // time 0
    payload.push(1);
    let graph = load(&mut world, &registry, &table, &payload, 2, V).unwrap();
    let (timeline, data) = (entity(&graph, 0), entity(&graph, 1));
    assert!(world.get::<BoolKeyData>(data).is_some());
    assert_eq!(
        world
            .get::<BoolTimelineInterpolator>(timeline)
            .unwrap()
            .base
            .data_10,
        Some(data)
    );
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    let mut value = false;
    assert!(update_bool_timeline_interpolator(
        &mut world, timeline, 0.0, &mut value
    ));
    assert!(value);
}

/// 00713FF0's table and 006E4790's color payload.
#[test]
fn color_graph() {
    let mut world = World::new();
    let registry = registry(&[("NiColorData", 0x006E_4520)]);
    let table = table(&["NiColorData"], &[0]);
    let mut payload = Vec::new();
    u32_le(&mut payload, 2); // two keys
    u32_le(&mut payload, 1); // linear
    for word in [
        0,
        0,
        0,
        0,
        0,
        0x3F80_0000,
        0x3F80_0000,
        0x4000_0000,
        0x4040_0000,
        0x4080_0000,
    ] {
        u32_le(&mut payload, word);
    }
    let consumed = payload.len();
    payload.push(0xA5);
    let graph = load(&mut world, &registry, &table, &payload, 1, V).unwrap();
    let color = world.get::<ColorKeyData>(entity(&graph, 0)).unwrap();
    let mut cached = 0;
    let middle = color.sample(0.5, &mut cached).unwrap();
    assert_eq!(graph.table.bytes_consumed, table.len());
    assert_eq!(graph.payload_bytes_consumed, consumed);
    assert_eq!(color.keys().len(), 2);
    assert_eq!(middle[0], 0.5);
    assert_eq!(middle[3], 2.0);
    assert_eq!(cached, 0);
}

/// 0075A220 / 0075A470's deferred color link after 006E4790.
#[test]
fn particle_color_graph() {
    let mut world = World::new();
    let registry = registry(&[
        ("NiPSysColorModifier", 0x0075_A360),
        ("NiColorData", 0x006E_4520),
        ("NiFloatData", 0x006E_3440),
    ]);
    let good_table = table(&["NiPSysColorModifier", "NiColorData"], &[0, 1]);
    let mut payload = Vec::new();
    name(&mut payload, "Tint");
    u32_le(&mut payload, 9); // order
    u32_le(&mut payload, NULL); // null system target
    payload.push(1); // active
    let link_position = payload.len();
    u32_le(&mut payload, 1); // forward color data owner
    u32_le(&mut payload, 2); // two color keys
    u32_le(&mut payload, 1); // linear
    for word in [
        0x3E80_0000,
        0,
        0,
        0,
        0x3F80_0000,
        0x3F40_0000,
        0x3F80_0000,
        0,
        0,
        0,
    ] {
        u32_le(&mut payload, word);
    }
    let graph = load(&mut world, &registry, &good_table, &payload, 2, V).unwrap();
    let modifier = world
        .get::<ParticleColorModifier>(entity(&graph, 0))
        .unwrap();
    let data = world.get::<ColorKeyData>(entity(&graph, 1)).unwrap();
    assert_eq!(modifier.data_18, Some(entity(&graph, 1)));
    assert_eq!(modifier.base.name_08.as_deref(), Some(&b"Tint"[..]));
    assert_eq!(modifier.base.order_0c, 9);
    assert!(modifier.base.active_14);
    assert_eq!(modifier.min_time_1c, 0.25);
    assert_eq!(modifier.max_time_20, 0.75);
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    let particles = [ParticleColorSample {
        age: 0.5,
        lifespan: 1.0,
    }];
    let mut output = [[0.0; 4]; 1];
    assert!(
        modifier
            .update_colors(&particles, &mut output, Some(data))
            .unwrap()
    );
    assert_eq!(output[0], [0.5, 0.0, 0.0, 0.5]);

    let mut world = World::new();
    let mut bad_index = payload.clone();
    bad_index[link_position] = 2;
    assert_thrown(&mut world, &registry, &good_table, &bad_index, 2, V);

    let wrong_table = table(&["NiPSysColorModifier", "NiFloatData"], &[0, 1]);
    let mut wrong_payload = payload[..link_position + 4].to_vec();
    u32_le(&mut wrong_payload, 0); // valid empty float owner, wrong link type
    assert_thrown(&mut world, &registry, &wrong_table, &wrong_payload, 2, V);
}

/// 00714390 loads both modifiers before the +0x20 deferred link pass.
#[test]
fn particle_dynamics_graph() {
    let mut world = World::new();
    let registry = registry(&[
        ("NiPSysPositionModifier", 0x0075_5690),
        ("NiPSysGrowFadeModifier", 0x0075_6DA0),
    ]);
    let table = table(
        &["NiPSysPositionModifier", "NiPSysGrowFadeModifier"],
        &[0, 1],
    );
    let mut payload = Vec::new();
    name(&mut payload, "Move");
    u32_le(&mut payload, 7);
    let target_offset = payload.len();
    u32_le(&mut payload, 1); // in-range forward target index
    payload.push(0); // inactive
    name(&mut payload, "Size");
    u32_le(&mut payload, 8);
    u32_le(&mut payload, NULL); // null target
    payload.push(0x80); // nonzero active
    u32_le(&mut payload, 0x4000_0000); // grow time 2.0
    u16_le(&mut payload, 3);
    u32_le(&mut payload, 0x3F00_0000); // fade time 0.5
    u16_le(&mut payload, 4);
    let size = payload.len();
    payload.push(0xCC);
    let graph = load(&mut world, &registry, &table, &payload, 2, V).unwrap();
    assert_eq!(graph.payload_bytes_consumed, size);
    let position = world
        .get::<ParticlePositionModifier>(entity(&graph, 0))
        .unwrap();
    assert_eq!(position.base.name_08.as_deref(), Some(&b"Move"[..]));
    assert_eq!(position.base.order_0c, 7);
    assert!(!position.base.active_14);
    let grow = world
        .get::<ParticleGrowFadeModifier>(entity(&graph, 1))
        .unwrap();
    assert_eq!(grow.base.name_08.as_deref(), Some(&b"Size"[..]));
    assert_eq!(grow.base.order_0c, 8);
    assert!(grow.base.active_14);
    assert_eq!(grow.grow_time_18, 2.0);
    assert_eq!(grow.grow_generation_1c, 3);
    assert_eq!(grow.fade_time_20, 0.5);
    assert_eq!(grow.fade_generation_24, 4);

    let mut world = World::new();
    assert_thrown(&mut world, &registry, &table, &payload[..size - 1], 2, V);
    let mut bad_target = payload.clone();
    bad_target[target_offset] = 2;
    assert_thrown(&mut world, &registry, &table, &bad_target, 2, V);
}

/// 00713FF0's preflight and the bounded object loop: no partial graph.
#[test]
fn failure_boundaries() {
    let mut world = World::new();
    let registry = registry(&[
        ("NiFloatInterpolator", 0x006D_2DD0),
        ("NiFloatData", 0x006E_3440),
        ("NiCamera", 0x0070_D730),
    ]);
    let good = table(&["NiFloatInterpolator", "NiFloatData"], &[0, 1]);
    let mut late_truncated = Vec::new();
    for word in [0, 1, 1, 1, 0] {
        u32_le(&mut late_truncated, word); // missing final key value
    }
    assert_thrown(&mut world, &registry, &good, &late_truncated, 2, V);

    let unsupported_table = table(&["NiCamera"], &[0]);
    let unsupported = load(&mut world, &registry, &unsupported_table, &[], 1, V).unwrap_err();
    assert_eq!(unsupported.code, StreamGraphErrorCode::TypeTable);
    assert_eq!(
        unsupported.table_error.unwrap().code,
        StreamObjectErrorCode::UnsupportedFactory
    );
    let old = load(&mut world, &registry, &good, &[], 2, 0x0500_0001).unwrap_err();
    assert_eq!(old.code, StreamGraphErrorCode::UnsupportedPayloadVersion);
    assert_eq!(objects(&mut world), 0);
}

/// Host rule: a factory failing midway through the table publishes nothing, where the C++ drops the table.
#[test]
fn table_failure_midway_publishes_nothing() {
    let mut world = World::new();
    let registry = registry(&[("NiFloatData", 0x006E_3440), ("NiCamera", 0x0070_D730)]);
    let bytes = table(&["NiFloatData", "NiCamera"], &[0, 0, 1]);
    let error = load(&mut world, &registry, &bytes, &[], 3, V).unwrap_err();
    assert_eq!(error.code, StreamGraphErrorCode::TypeTable);
    let table_error = error.table_error.unwrap();
    assert_eq!(table_error.code, StreamObjectErrorCode::UnsupportedFactory);
    assert_eq!(table_error.object_index, Some(2));
    assert_eq!(objects(&mut world), 0);
}

/// 00714390 separates LoadBinary and LinkObject; 00700050 retains extras.
#[test]
fn alpha_deferred_graph() {
    let mut world = World::new();
    let registry = registry(&[("NiAlphaProperty", 0x0071_85B0), ("TextAlias", 0x006D_7450)]);
    let table = table(&["NiAlphaProperty", "TextAlias"], &[0, 1]);
    let mut payload = Vec::new();
    name(&mut payload, "property");
    u32_le(&mut payload, 1);
    u32_le(&mut payload, 1); // Later extra owner, whose name has not loaded yet.
    u32_le(&mut payload, NULL);
    u16_le(&mut payload, 0x1234);
    payload.push(127);
    name(&mut payload, "late-name");
    u32_le(&mut payload, 1);
    u32_le(&mut payload, 0x3F80_0000);
    name(&mut payload, "start");
    let graph = load(&mut world, &registry, &table, &payload, 2, V).unwrap();
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    let (alpha, keys) = (entity(&graph, 0), entity(&graph, 1));
    let state = world.get::<AlphaProperty>(alpha).unwrap();
    assert_eq!(state.flags_18, 0x1234);
    assert_eq!(state.alpha_test_ref_1a, 127);
    assert_eq!(world.get::<NiObjectNet>(alpha).unwrap().extra_data, [keys]);
    assert_eq!(
        world.get::<NiExtraDataName>(keys).unwrap().0.as_deref(),
        Some(&b"late-name"[..])
    );
    assert_eq!(
        world.get::<TextKeyExtraData>(keys).unwrap().keys[0]
            .text
            .as_deref(),
        Some(&b"start"[..])
    );
}

/// 007099A0 followed by 00741340 / 00700050.
#[test]
fn material_graph() {
    let mut world = World::new();
    let registry = registry(&[
        ("NiMaterialProperty", 0x0070_9710),
        ("TextAlias", 0x006D_7450),
    ]);
    let table = table(&["NiMaterialProperty", "TextAlias"], &[0, 1]);
    let mut payload = Vec::new();
    name(&mut payload, "material");
    u32_le(&mut payload, 1); // one forward extra
    u32_le(&mut payload, 1);
    u32_le(&mut payload, NULL); // null controller
    for index in 0..14 {
        u32_le(&mut payload, 0x3F00_0000 + index);
    }
    name(&mut payload, "late-extra");
    u32_le(&mut payload, 0); // no text keys
    let graph = load(&mut world, &registry, &table, &payload, 2, V).unwrap();
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    let (material, extra) = (entity(&graph, 0), entity(&graph, 1));
    let net = world.get::<NiObjectNet>(material).unwrap();
    assert_eq!(net.name.as_deref(), Some(&b"material"[..]));
    assert_eq!(net.extra_data, [extra]);
    assert_eq!(
        world.get::<NiExtraDataName>(extra).unwrap().0.as_deref(),
        Some(&b"late-extra"[..])
    );
    assert_eq!(
        world
            .get::<MaterialProperty>(material)
            .unwrap()
            .values
            .ambient[0],
        0.5
    );
}

fn empty_particle_data(group: u32, flags_present: bool) -> (Vec<u8>, usize) {
    let mut payload = Vec::new();
    u32_le(&mut payload, group); // allocation group
    u16_le(&mut payload, 0); // zero vertex capacity
    payload.extend([0, 0]); // two geometry format bytes
    payload.push(u8::from(flags_present)); // positions
    u16_le(&mut payload, u16::from(flags_present)); // geometry flags / UV sets
    payload.push(u8::from(flags_present)); // normals
    for _ in 0..4 {
        u32_le(&mut payload, 0); // bounds
    }
    payload.push(u8::from(flags_present)); // colors
    u16_le(&mut payload, 0x8000); // extra geometry flags
    let link_offset = payload.len();
    u32_le(&mut payload, NULL); // null additional geometry
    payload.push(0); // absent radius array
    u16_le(&mut payload, 0); // active count
    payload.extend([0; 4]); // size + three rotation flags
    payload.push(0); // absent rotation speeds
    u16_le(&mut payload, 0); // added count
    u16_le(&mut payload, 0); // added base
    (payload, link_offset)
}

/// 00759C50 / 00759DE0's owner and 00728F20's additional geometry reference.
#[test]
fn particle_data_graph() {
    let mut world = World::new();
    let registry = registry(&[("NiPSysData", 0x0075_9C50)]);
    let table = table(&["NiPSysData"], &[0]);
    let (payload, link_offset) = empty_particle_data(0, false);
    let graph = load(&mut world, &registry, &table, &payload, 1, V).unwrap();
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    let owner = world.get::<ParticleData>(entity(&graph, 0)).unwrap();
    assert_eq!(owner.vertex_count, 0);
    assert_eq!(owner.active_count_48, 0);
    assert_eq!(owner.geometry_extra_flags_2e, 0x8000);
    assert_eq!(owner.deferred_geometry_link, Some(NULL));

    let mut world = World::new();
    let mut nonnull = payload.clone();
    nonnull[link_offset..link_offset + 4].fill(0);
    let unresolved = load(&mut world, &registry, &table, &nonnull, 1, V).unwrap_err();
    assert_eq!(unresolved.code, StreamGraphErrorCode::UnsupportedLinkTarget);
    assert_eq!(unresolved.object_index, Some(0));
    assert_eq!(objects(&mut world), 0);
}

/// 0075BF00 / 0075B100 and 0075BDD0 / 00752CB0 through the post-load pass.
#[test]
fn age_bound_graph() {
    let mut world = World::new();
    let registry = registry(&[
        ("NiPSysAgeDeathModifier", 0x0075_BD90),
        ("NiPSysBoundUpdateModifier", 0x0075_A9E0),
    ]);
    let table = table(
        &["NiPSysAgeDeathModifier", "NiPSysBoundUpdateModifier"],
        &[0, 1],
    );
    let mut payload = Vec::new();
    name(&mut payload, "Age");
    u32_le(&mut payload, 4);
    u32_le(&mut payload, 1); // forward target index (type binding remains open)
    payload.push(1);
    payload.push(0); // no death spawn
    u32_le(&mut payload, NULL); // no spawn link
    name(&mut payload, "Bound");
    u32_le(&mut payload, 5);
    u32_le(&mut payload, NULL);
    payload.push(0);
    u16_le(&mut payload, 0xFFFF); // auto skip raw word
    let graph = load(&mut world, &registry, &table, &payload, 2, V).unwrap();
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    let age = world
        .get::<ParticleAgeDeathModifier>(entity(&graph, 0))
        .unwrap();
    assert_eq!(age.base.name_08.as_deref(), Some(&b"Age"[..]));
    assert_eq!(age.base.order_0c, 4);
    assert!(age.base.active_14);
    assert!(!age.spawn_on_death_18);
    assert!(!age.spawn_linked);
    let bound = world
        .get::<ParticleBoundUpdateModifier>(entity(&graph, 1))
        .unwrap();
    assert_eq!(bound.base.name_08.as_deref(), Some(&b"Bound"[..]));
    assert_eq!(bound.base.order_0c, 5);
    assert!(!bound.base.active_14);
    assert_eq!(bound.skip_count_18, 0xFFFF);
    assert_eq!(bound.cache_1c.len(), 1, "cached_sphere_count");
}

/// 00758D70's payload and 0075E870's raw object link.
#[test]
fn drag_graph() {
    let mut world = World::new();
    let registry = registry(&[("NiPSysDragModifier", 0x0075_8960)]);
    let table = table(&["NiPSysDragModifier"], &[0]);
    let mut payload = Vec::new();
    name(&mut payload, "Drag");
    u32_le(&mut payload, 7);
    u32_le(&mut payload, NULL); // null system target
    payload.push(1);
    let link_offset = payload.len();
    u32_le(&mut payload, NULL); // null drag object
    for word in [0x3F80_0000, 0, 0, 0x3D4C_CCCD, 0x3F80_0000, 0x4040_0000] {
        u32_le(&mut payload, word);
    }
    let graph = load(&mut world, &registry, &table, &payload, 1, V).unwrap();
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    let drag = world
        .get::<ParticleDragModifier>(entity(&graph, 0))
        .unwrap();
    assert_eq!(drag.base.name_08.as_deref(), Some(&b"Drag"[..]));
    assert_eq!(drag.base.order_0c, 7);
    assert!(drag.base.active_14);
    assert!(!drag.drag_object_linked);
    assert_eq!(drag.minimum_distance_2c, 1.0);
    assert_eq!(drag.maximum_distance_30, 3.0);
    let mut linked = payload.clone();
    linked[link_offset..link_offset + 4].fill(0);
    // 0075E870 stores the linked object (here the modifier itself) as a raw +0x18 pointer.
    let graph = load(&mut world, &registry, &table, &linked, 1, V).unwrap();
    let this = entity(&graph, 0);
    let drag = world.get::<ParticleDragModifier>(this).unwrap();
    assert_eq!(drag.drag_object_18.0, Some(this));
}

/// 0075BF00 / 00752780, 0075BDD0's typed forward link and 0075BC50's age store, spawn callback, then remove.
#[test]
fn spawn_forward_graph() {
    let mut world = World::new();
    let registry = registry(&[
        ("NiPSysAgeDeathModifier", 0x0075_BD90),
        ("NiPSysSpawnModifier", 0x0075_1D10),
    ]);
    let good = table(&["NiPSysAgeDeathModifier", "NiPSysSpawnModifier"], &[0, 1]);
    let mut payload = Vec::new();
    name(&mut payload, "Age");
    u32_le(&mut payload, 4);
    u32_le(&mut payload, NULL);
    payload.push(1);
    payload.push(1); // spawn on death
    u32_le(&mut payload, 1); // forward Spawn owner
    name(&mut payload, "Spawn");
    u32_le(&mut payload, 5);
    u32_le(&mut payload, NULL);
    payload.push(1);
    u16_le(&mut payload, 1); // generation limit
    u32_le(&mut payload, 0x3F80_0000); // probability 1
    u16_le(&mut payload, 1); // one child
    u16_le(&mut payload, 1);
    for _ in 0..4 {
        u32_le(&mut payload, 0); // chaos/lifespan fields
    }
    let graph = load(&mut world, &registry, &good, &payload, 2, V).unwrap();
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    let age = world
        .get::<ParticleAgeDeathModifier>(entity(&graph, 0))
        .unwrap()
        .clone();
    let spawn = world
        .get::<ParticleSpawnModifier>(entity(&graph, 1))
        .unwrap()
        .clone();
    assert!(age.spawn_on_death_18);
    assert!(age.spawn_linked);
    assert_eq!(age.spawn_1c, Some(entity(&graph, 1)));
    assert_eq!(spawn.base.name_08.as_deref(), Some(&b"Spawn"[..]));
    assert_eq!(spawn.generation_limit_18, 1);

    #[derive(Default)]
    struct Services {
        draws: usize,
        calls: usize,
        death_time: f32,
        index: u16,
        active_at_call: u16,
        age_at_call: f32,
    }
    impl ParticleSpawnServices for Services {
        fn draw_rand15(&mut self) -> u16 {
            self.draws += 1;
            0
        }
        fn spawn_child(
            &mut self,
            current_time: f32,
            death_time: f32,
            parent_index: u16,
            source: &mut ParticleData,
        ) -> Result<(), GameError> {
            assert_eq!(current_time, 3.0, "Age/Spawn callback current time differs");
            self.calls += 1;
            self.death_time = death_time;
            self.index = parent_index;
            self.active_at_call = source.active_count_48;
            self.age_at_call = source.info_5c[usize::from(parent_index)].age_0c;
            Ok(())
        }
    }
    let mut services = Services::default();
    let mut data = ParticleData {
        active_count_48: 2,
        positions: vec![[0.0; 3]; 2],
        info_5c: vec![
            ParticleInfo {
                age_0c: 0.0,
                lifespan_10: 10.0,
                last_update_time_14: 2.0,
                ..ParticleInfo::default()
            },
            ParticleInfo {
                age_0c: 0.5,
                lifespan_10: 1.0,
                last_update_time_14: 2.0,
                ..ParticleInfo::default()
            },
        ],
        ..ParticleData::default()
    };
    age.update_ages_with_spawn(3.0, &mut data, Some(&spawn), &mut services)
        .unwrap();
    assert_eq!(services.draws, 2);
    assert_eq!(services.calls, 1);
    assert_eq!(services.index, 1);
    assert_eq!(services.active_at_call, 2);
    assert_eq!(services.age_at_call, 1.5);
    assert_eq!(services.death_time, 2.5);
    assert_eq!(data.active_count_48, 1);
    assert_eq!(data.info_5c[0].age_0c, 1.0);

    data.active_count_48 = 1;
    data.info_5c[0].age_0c = 0.5;
    data.info_5c[0].lifespan_10 = 1.0;
    data.info_5c[0].last_update_time_14 = 2.0;
    data.info_5c[0].generation_18 = 1;
    age.update_ages_with_spawn(3.0, &mut data, Some(&spawn), &mut services)
        .unwrap();
    assert_eq!(services.draws, 2, "the generation gate consumes no RNG");
    assert_eq!(services.calls, 1);
    assert_eq!(data.active_count_48, 0);

    // The same type index now points both entries to AgeDeath; the link must reject it.
    let mut world = World::new();
    let mut wrong = good.clone();
    let end = wrong.len();
    wrong[end - 2] = 0;
    wrong[end - 1] = 0;
    assert_thrown(&mut world, &registry, &wrong, &payload, 2, V);
}

/// 0074E1C0's factory and 0074E390's version-gated LoadBinary.
#[test]
fn rotation_graph() {
    let mut world = World::new();
    let registry = registry(&[("NiPSysRotationModifier", 0x0074_E1C0)]);
    let FactoryCreate::Created { entity: cold, .. } =
        registry.create(&mut world, "NiPSysRotationModifier")
    else {
        panic!("rotation factory did not construct a cold typed owner");
    };
    assert!(
        world
            .get::<ParticleRotationModifier>(cold)
            .unwrap()
            .random_initial_axis
    );
    world.despawn(cold);
    let table = table(&["NiPSysRotationModifier"], &[0]);
    let mut modern = Vec::new();
    name(&mut modern, "Rot");
    u32_le(&mut modern, 3);
    let target_at = modern.len();
    u32_le(&mut modern, NULL);
    modern.push(1);
    u32_le(&mut modern, 0x3F80_0000); // speed
    u32_le(&mut modern, 0x4000_0000); // speed variation
    u32_le(&mut modern, 0x4040_0000); // angle
    u32_le(&mut modern, 0x4080_0000); // angle variation
    modern.push(1); // random speed sign
    modern.push(0); // fixed axis
    u32_le(&mut modern, 0x3F80_0000);
    u32_le(&mut modern, 0);
    u32_le(&mut modern, 0);
    let graph = load(&mut world, &registry, &table, &modern, 1, 0x1400_0002).unwrap();
    assert_eq!(graph.payload_bytes_consumed, modern.len());
    let owner = world
        .get::<ParticleRotationModifier>(entity(&graph, 0))
        .unwrap();
    assert_eq!(owner.base.name_08.as_deref(), Some(&b"Rot"[..]));
    assert_eq!(owner.base.order_0c, 3);
    assert_eq!(owner.initial_speed, 1.0);
    assert_eq!(owner.speed_variation, 2.0);
    assert_eq!(owner.initial_angle, 3.0);
    assert_eq!(owner.angle_variation, 4.0);
    assert!(owner.random_speed_sign);
    assert!(!owner.random_initial_axis);

    let mut legacy = Vec::new();
    name(&mut legacy, "Old");
    u32_le(&mut legacy, 4);
    u32_le(&mut legacy, NULL);
    legacy.push(0);
    u32_le(&mut legacy, 0x40A0_0000); // initial speed 5
    legacy.push(1);
    u32_le(&mut legacy, 0x3F80_0000);
    u32_le(&mut legacy, 0);
    u32_le(&mut legacy, 0);
    let old = load(&mut world, &registry, &table, &legacy, 1, 0x0A01_0072).unwrap();
    assert_eq!(old.payload_bytes_consumed, legacy.len());
    let owner = world
        .get::<ParticleRotationModifier>(entity(&old, 0))
        .unwrap();
    assert_eq!(owner.base.name_08.as_deref(), Some(&b"Old"[..]));
    assert_eq!(owner.initial_speed, 5.0);
    assert_eq!(owner.speed_variation, 0.0);
    assert!(!owner.random_speed_sign);
    assert!(owner.random_initial_axis);

    let mut world = World::new();
    let mut invalid = modern.clone();
    invalid[target_at] = 1;
    invalid[target_at + 1..target_at + 4].fill(0);
    assert_thrown(&mut world, &registry, &table, &invalid, 1, 0x1400_0002);
}

/// 00713030's header before two NiPSysData; each present zero-vertex flag still claims its zero-width allocation.
#[test]
fn grouped_particle_graph() {
    let mut world = World::new();
    let registry = registry(&[("NiPSysData", 0x0075_9C50)]);
    let table = table(&["NiPSysData"], &[0, 0]);
    let mut header = Vec::new();
    u32_le(&mut header, 1);
    u32_le(&mut header, 0); // zero-size group at index 1
    let (one, _) = empty_particle_data(1, true);
    let mut payload = one.clone();
    payload.extend(&one);
    let graph = load_stream_graph_with_groups(
        &mut world,
        &registry,
        &table,
        &header,
        &payload,
        2,
        V,
        &mut StreamGraphLinkRuntime::default(),
    )
    .unwrap();
    let group = graph.allocation_groups.resolve(1).unwrap().unwrap();
    let first = world.get::<ParticleData>(entity(&graph, 0)).unwrap();
    let second = world.get::<ParticleData>(entity(&graph, 1)).unwrap();
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    assert_eq!(graph.allocation_groups.bytes_consumed, header.len());
    assert_eq!(group.size(), 0);
    assert_eq!(group.used_bytes(), 0);
    assert_eq!(group.allocation_count(), 8);
    assert_eq!(first.allocation_group, Some(1));
    assert_eq!(second.allocation_group, Some(1));
    assert!(first.geometry_positions_present);
    assert!(first.geometry_normals_present);
    assert!(first.geometry_colors_present);
    assert_eq!(first.geometry_group_arrays[3].size, 0);

    let mut root_record = Vec::new();
    for word in [4, 1, NULL, 1, 0] {
        u32_le(&mut root_record, word);
    }
    let rooted = load_stream_graph_with_roots(
        &mut world,
        &registry,
        &table,
        &header,
        &payload,
        &root_record,
        2,
        V,
        &mut StreamGraphLinkRuntime::default(),
    )
    .unwrap();
    let roots = &rooted.roots;
    let second_owner = entity(&rooted, 1);
    assert_eq!(roots.bytes_consumed, root_record.len());
    assert_eq!(roots.owners.len(), 4);
    assert_eq!(roots.original_indices, [1, NULL, 1, 0]);
    assert_eq!(roots.owners[0], Some(second_owner));
    assert_eq!(roots.owners[1], None);
    assert_eq!(roots.owners[2], Some(second_owner));
    assert_eq!(roots.owners[3], Some(entity(&rooted, 0)));

    let empty = read_stream_roots(&[0, 0, 0, 0], &rooted.table).unwrap();
    assert_eq!(empty.bytes_consumed, 4);
    assert!(empty.owners.is_empty());
    for length in 0..root_record.len() {
        assert!(read_stream_roots(&root_record[..length], &rooted.table).is_err());
    }
    let mut invalid_root = root_record.clone();
    invalid_root[4] = 2;
    assert!(read_stream_roots(&invalid_root, &rooted.table).is_err());
}

/// 00750B50's factory and 00751920's payload; a nonnull object is stored raw by 0075E870.
#[test]
fn gravity_graph() {
    let mut world = World::new();
    let registry = registry(&[("NiPSysGravityModifier", 0x0075_0BA0)]);
    let FactoryCreate::Created { entity: cold, .. } =
        registry.create(&mut world, "NiPSysGravityModifier")
    else {
        panic!("gravity factory did not construct a typed owner");
    };
    assert!(world.get::<ParticleGravityModifier>(cold).is_some());
    let table = table(&["NiPSysGravityModifier"], &[0]);
    let mut payload = Vec::new();
    name(&mut payload, "Gravity");
    u32_le(&mut payload, 5);
    u32_le(&mut payload, NULL);
    payload.push(1);
    let object_at = payload.len();
    u32_le(&mut payload, NULL); // null object
    for word in [0x3F80_0000, 0, 0, 0, 0x3F80_0000, 0, 0, 0x3F80_0000] {
        u32_le(&mut payload, word);
    }
    let graph = load(&mut world, &registry, &table, &payload, 1, V).unwrap();
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    let owner = world
        .get::<ParticleGravityModifier>(entity(&graph, 0))
        .unwrap();
    assert_eq!(owner.base.name_08.as_deref(), Some(&b"Gravity"[..]));
    assert_eq!(owner.base.order_0c, 5);
    assert_eq!(owner.gravity_object_index, NULL);
    let mut linked = payload.clone();
    linked[object_at..object_at + 4].fill(0);
    let graph = load(&mut world, &registry, &table, &linked, 1, V).unwrap();
    let this = entity(&graph, 0);
    let linked = world.get::<ParticleGravityModifier>(this).unwrap();
    assert_eq!(linked.gravity_object_18.0, Some(this));
    assert_eq!(linked.gravity_object_index, 0);
}

fn system_prefix(payload: &mut Vec<u8>, controller: u32, data: u32) {
    name(payload, "System");
    u32_le(payload, 0); // no extras
    u32_le(payload, controller);
    u16_le(payload, 0); // AV flags
    for index in 0..13 {
        u32_le(payload, if index == 12 { 0x3F80_0000 } else { 0 });
    }
    u32_le(payload, 0); // no properties
    u32_le(payload, NULL); // null collision
    u32_le(payload, data);
    u32_le(payload, NULL); // null skin
    payload.push(0); // no shader
    payload.push(1); // world space
}

/// 00714390 loads every scalar before 00749BC0 attaches; equal +0x0C orders keep the raw modifier list.
#[test]
fn system_forward_graph() {
    let mut world = World::new();
    let registry = registry(&[
        ("NiParticleSystem", 0x0074_9FE0),
        ("NiPSysData", 0x0075_9C50),
        ("NiPSysRotationModifier", 0x0074_E1C0),
    ]);
    let table = table(
        &["NiParticleSystem", "NiPSysData", "NiPSysRotationModifier"],
        &[0, 1, 2, 2],
    );
    let mut payload = Vec::new();
    system_prefix(&mut payload, NULL, 1);
    u32_le(&mut payload, 2);
    u32_le(&mut payload, 3);
    u32_le(&mut payload, 2);
    let (data, _) = empty_particle_data(0, false);
    payload.extend(&data);
    for text in ["B", "A"] {
        name(&mut payload, text);
        u32_le(&mut payload, 5); // equal order
        u32_le(&mut payload, 0); // system target
        payload.push(1);
        for _ in 0..4 {
            u32_le(&mut payload, 0);
        }
        payload.push(0); // sign
        payload.push(0); // fixed axis
        u32_le(&mut payload, 0x3F80_0000);
        u32_le(&mut payload, 0);
        u32_le(&mut payload, 0);
    }
    let groups = [0, 0, 0, 0];
    let mut roots = Vec::new();
    u32_le(&mut roots, 1);
    u32_le(&mut roots, 0);
    let graph = load_stream_graph_with_roots(
        &mut world,
        &registry,
        &table,
        &groups,
        &payload,
        &roots,
        4,
        V,
        &mut StreamGraphLinkRuntime::default(),
    )
    .unwrap();
    let (system, data) = (entity(&graph, 0), entity(&graph, 1));
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    let state = world.get::<ParticleSystem>(system).unwrap();
    assert_eq!(state.data_b4, Some(data));
    assert_eq!(state.modifiers_c4.len(), 2);
    assert_eq!(state.modifiers_c4[0].original_name, b"A");
    assert_eq!(state.modifiers_c4[1].original_name, b"B");
    assert_eq!(state.modifiers_c4[0].original_order, 5);
    assert_eq!(graph.roots.owners, [Some(system)]);
    // The scene passes and `particle_mesh_emitter` read this on a real particle system target.
    assert!(world.get::<NiAvWorld>(system).is_some());
}

/// 006D9D40 / 006D9F90's NiPosData leaf.
#[test]
fn position_data_graph() {
    let mut world = World::new();
    let registry = registry(&[("NiPosData", 0x006D_9D40)]);
    let table = table(&["NiPosData"], &[0]);
    let mut payload = Vec::new();
    u32_le(&mut payload, 1); // one key
    u32_le(&mut payload, 1); // linear type
    u32_le(&mut payload, 0x3F00_0000); // time 0.5
    u32_le(&mut payload, 0x3F80_0000); // position x 1
    u32_le(&mut payload, 0x4000_0000); // y 2
    u32_le(&mut payload, 0x4040_0000); // z 3
    let graph = load(&mut world, &registry, &table, &payload, 1, V).unwrap();
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    let owner = world.get::<PositionKeyData>(entity(&graph, 0)).unwrap();
    assert_eq!(owner.key_type(), PositionKeyType::LINEAR);
    assert_eq!(owner.stride(), 16);
    assert_eq!(owner.keys().len(), 1);
    assert_eq!(owner.keys()[0].time, 0.5);
    assert_eq!(owner.keys()[0].position, [1.0, 2.0, 3.0]);
}

/// 00706E10 / 00706690 / 00719110 through 00714390's passes.
#[test]
fn property_graph() {
    let mut world = World::new();
    let registry = registry(&[
        ("NiZBufferProperty", 0x0070_6CB0),
        ("NiVertexColorProperty", 0x0070_6530),
        ("NiStencilProperty", 0x0071_8F80),
    ]);
    let table = table(
        &[
            "NiZBufferProperty",
            "NiVertexColorProperty",
            "NiStencilProperty",
        ],
        &[0, 1, 2],
    );
    let mut payload = Vec::new();
    // 006FFCE0's modern empty NET prefix.
    let base = |payload: &mut Vec<u8>| {
        u32_le(payload, 0);
        u32_le(payload, 0);
        u32_le(payload, NULL);
    };
    base(&mut payload);
    u16_le(&mut payload, 3);
    u32_le(&mut payload, 3);
    base(&mut payload);
    u16_le(&mut payload, 8);
    u32_le(&mut payload, 2);
    u32_le(&mut payload, 1);
    base(&mut payload);
    payload.push(1);
    for field in [7, 0xAA, 0xFFFF, 1, 2, 3, 2] {
        u32_le(&mut payload, field);
    }
    let graph = load(&mut world, &registry, &table, &payload, 3, V).unwrap();
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    assert_eq!(
        world
            .get::<ZBufferProperty>(entity(&graph, 0))
            .unwrap()
            .flags_18,
        0x000F
    );
    assert_eq!(
        world
            .get::<VertexColorProperty>(entity(&graph, 1))
            .unwrap()
            .flags_18,
        0x0028
    );
    let stencil = world.get::<StencilProperty>(entity(&graph, 2)).unwrap();
    assert_eq!(stencil.flags_18, 0x79A3);
    assert_eq!(stencil.reference_1c, 0xAA);
    assert_eq!(stencil.mask_20, 0xFFFF);
}

/// 006CE320's immediate +0x3C, 00715E70's next and target, 0074FE30's deferred +0x48.
#[test]
fn controller_forward_graph() {
    let mut world = World::new();
    let registry = registry(&[
        ("NiPSysEmitterCtlr", 0x0074_FDE0),
        ("NiFloatInterpolator", 0x006D_2DD0),
        ("NiBoolTimelineInterpolator", 0x006E_7E70),
        ("NiPSysUpdateCtlr", 0x0075_3BB0),
    ]);
    let table = table(
        &[
            "NiPSysEmitterCtlr",
            "NiFloatInterpolator",
            "NiBoolTimelineInterpolator",
            "NiPSysUpdateCtlr",
        ],
        &[0, 1, 2, 3],
    );
    let mut payload = Vec::new();
    u32_le(&mut payload, 3); // forward next controller
    u16_le(&mut payload, 8); // phase-randomized, active controller
    for value in [0x3F80_0000, 0, 0, 0x3F80_0000] {
        u32_le(&mut payload, value);
    }
    u32_le(&mut payload, NULL); // null system target
    u32_le(&mut payload, 1); // forward immediate float interpolator
    u32_le(&mut payload, 0); // absent modifier name
    u32_le(&mut payload, 2); // forward deferred active timeline
    u32_le(&mut payload, 0x40A0_0000); // float scalar 5
    u32_le(&mut payload, NULL); // no key data
    payload.push(1); // active timeline scalar
    u32_le(&mut payload, NULL); // no bool key data
    u32_le(&mut payload, NULL); // update next null
    u16_le(&mut payload, 0x0A); // no random phase branch
    for value in [0x3F80_0000, 0x3F80_0000, 0, 0x3F80_0000] {
        u32_le(&mut payload, value);
    }
    u32_le(&mut payload, NULL); // update target null
    let missing = load(&mut world, &registry, &table, &payload, 4, V).unwrap_err();
    assert_eq!(missing.code, StreamGraphErrorCode::UnsupportedLinkService);
    assert_eq!(missing.object_index, Some(0));
    assert_eq!(objects(&mut world), 0);

    let mut draws = 0;
    let mut draw = || {
        draws += 1;
        16384
    };
    let graph = load_stream_graph_payloads(
        &mut world,
        &registry,
        &table,
        &payload,
        4,
        V,
        &mut StreamGraphLinkRuntime {
            draw_rand: Some(&mut draw),
            draw_on_io_thread: false,
            ..StreamGraphLinkRuntime::default()
        },
    )
    .unwrap();
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    assert_eq!(draws, 1);
    let emitter = world
        .get::<ParticleEmitterController>(entity(&graph, 0))
        .unwrap();
    assert_eq!(emitter.birth_rate_interpolator_3c, Some(entity(&graph, 1)));
    assert!(emitter.field_48_linked);
    assert_eq!(emitter.active_interpolator_48, Some(entity(&graph, 2)));
    assert_eq!(emitter.time.next_34, Some(entity(&graph, 3)));
    assert!(emitter.time.next_linked);
    assert!(!emitter.time.target_linked);
    assert!(!emitter.cached_modifier_linked);
    assert!(emitter.time.fields.phase > 0.0);
}

/// 00714390 finishes Mesh LoadBinary metadata before System LinkObject.
#[test]
fn system_forward_mesh_graph() {
    let mut world = World::new();
    let registry = registry(&[
        ("NiParticleSystem", 0x0074_9FE0),
        ("NiPSysMeshEmitter", 0x0074_BB20),
    ]);
    let table = table(&["NiParticleSystem", "NiPSysMeshEmitter"], &[0, 1]);
    let mut payload = Vec::new();
    system_prefix(&mut payload, NULL, NULL);
    u32_le(&mut payload, 1); // one modifier
    u32_le(&mut payload, 1); // forward Mesh owner
    name(&mut payload, "Mesh");
    u32_le(&mut payload, 7); // order
    u32_le(&mut payload, 0); // System target
    payload.push(1); // active
    for _ in 0..14 {
        u32_le(&mut payload, 0); // emitter scalars
    }
    u32_le(&mut payload, 2); // two raw mesh refs, both null
    u32_le(&mut payload, NULL);
    u32_le(&mut payload, NULL);
    u32_le(&mut payload, 0); // velocity mode
    u32_le(&mut payload, 0); // emission mode
    u32_le(&mut payload, 0x3F80_0000); // axis x
    u32_le(&mut payload, 0);
    u32_le(&mut payload, 0);
    let graph = load(&mut world, &registry, &table, &payload, 2, V).unwrap();
    assert_eq!(graph.payload_bytes_consumed, payload.len());
    let system = world.get::<ParticleSystem>(entity(&graph, 0)).unwrap();
    assert_eq!(system.modifiers_c4.len(), 1);
    assert_eq!(system.modifiers_c4[0].original_name, b"Mesh");
    assert_eq!(system.modifiers_c4[0].original_order, 7);
    let mesh = world.get::<ParticleMeshEmitter>(entity(&graph, 1)).unwrap();
    assert_eq!(mesh.state.base.name.as_deref(), Some(&b"Mesh"[..]));
    assert_eq!(mesh.raw_mesh_indices, [NULL, NULL]);
    assert!(mesh.mesh_indices.is_empty());
}

/// 00700050 retains the cold typed controller head before its own LinkObject.
#[test]
fn system_controller_head_graph() {
    let mut world = World::new();
    let registry = registry(&[
        ("NiParticleSystem", 0x0074_9FE0),
        ("NiPSysUpdateCtlr", 0x0075_3BB0),
    ]);
    let table = table(&["NiParticleSystem", "NiPSysUpdateCtlr"], &[0, 1]);
    let mut payload = Vec::new();
    system_prefix(&mut payload, 1, NULL);
    u32_le(&mut payload, 0); // no modifiers
    u32_le(&mut payload, NULL); // controller next null
    u16_le(&mut payload, 0x0A);
    for value in [0x3F80_0000, 0x3F80_0000, 0, 0x3F80_0000] {
        u32_le(&mut payload, value);
    }
    u32_le(&mut payload, 0); // controller target System
    let graph = load(&mut world, &registry, &table, &payload, 2, V).unwrap();
    let (system, update) = (entity(&graph, 0), entity(&graph, 1));
    assert_eq!(
        world.get::<NiObjectNet>(system).unwrap().controller_0c,
        Some(update)
    );
    let time = &world.get::<ParticleUpdateController>(update).unwrap().time;
    assert_eq!(time.target_30, Some(system));
    assert!(time.target_linked);
    assert_eq!(graph.payload_bytes_consumed, payload.len());
}

/// A source texture's +0x18.. after its NET prefix: a non-external texture named `path`, with a null NiPixelData.
fn source_texture(payload: &mut Vec<u8>, path: &str) {
    u32_le(payload, 0);
    u32_le(payload, 0);
    u32_le(payload, NULL);
    payload.push(0); // not external
    name(payload, path);
    u32_le(payload, NULL); // no NiPixelData
    u32_le(payload, 6); // pixel layout
    u32_le(payload, 2); // mip mapped
    u32_le(payload, 3); // alpha format
    payload.push(1); // static
    payload.push(0); // direct to renderer
}

#[derive(Default)]
struct Palette {
    found: Option<Entity>,
    registered: Vec<Entity>,
}

impl SourceTextureLoadServices for Palette {
    fn search_candidates(&mut self, _: &[u8]) -> Vec<Vec<u8>> {
        Vec::new()
    }
    fn find_texture(&mut self, _: &[u8]) -> Option<Entity> {
        self.found
    }
    fn register_texture(&mut self, _: &[u8], texture: Entity) {
        self.registered.push(texture);
    }
    fn file_exists(&mut self, _: &[u8]) -> bool {
        false
    }
}

/// Host rule: a failed load drops its own objects, but a texture the palette holds stays, as its C++ `shared_ptr` keeps
/// it. A slot's shared replacement was not spawned by this load, and an object just registered is the palette's too.
#[test]
fn failed_load_keeps_what_the_palette_holds() {
    let registry = registry(&[
        ("NiSourceTexture", 0x0070_21F0),
        ("NiFloatData", 0x006E_3440),
    ]);
    let table = table(&["NiSourceTexture", "NiFloatData"], &[0, 1]);
    let mut payload = Vec::new();
    source_texture(&mut payload, "a.dds");
    u32_le(&mut payload, 1); // one float key, truncated: the load fails after the texture's slot

    let mut world = World::new();
    let shared = world
        .spawn((NiClass::SourceTexture, SourceTexture::default()))
        .id();
    let mut palette = Palette {
        found: Some(shared),
        ..Palette::default()
    };
    let error = load_stream_graph_payloads(
        &mut world,
        &registry,
        &table,
        &payload,
        2,
        V,
        &mut StreamGraphLinkRuntime {
            source_textures: Some(&mut palette),
            ..StreamGraphLinkRuntime::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error.code, StreamGraphErrorCode::Stream(_)));
    assert!(world.get_entity(shared).is_ok(), "the palette's texture");
    assert_eq!(objects(&mut world), 1, "the rest of the table is gone");

    let mut world = World::new();
    let mut palette = Palette::default();
    let error = load_stream_graph_payloads(
        &mut world,
        &registry,
        &table,
        &payload,
        2,
        V,
        &mut StreamGraphLinkRuntime {
            source_textures: Some(&mut palette),
            ..StreamGraphLinkRuntime::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error.code, StreamGraphErrorCode::Stream(_)));
    assert_eq!(palette.registered.len(), 1);
    assert!(world.get_entity(palette.registered[0]).is_ok());
    assert_eq!(objects(&mut world), 1);
}

/// A one-object NIF: an empty NiFloatData, the root.
fn one_object_file() -> Vec<u8> {
    let mut file = b"Gamebryo File Format, Version 20.0.0.4\n".to_vec();
    u32_le(&mut file, V);
    file.push(1); // little endian
    u32_le(&mut file, 11); // user version
    u32_le(&mut file, 1); // objects
    u32_le(&mut file, 0); // auxiliary
    file.extend([0, 0, 0]); // three empty metadata fields
    file.extend(table(&["NiFloatData"], &[0]));
    u32_le(&mut file, 0); // no allocation groups
    u32_le(&mut file, 0); // no keys
    u32_le(&mut file, 1); // one root
    u32_le(&mut file, 0);
    file
}

/// Host rule: a pass after the graph failing drops the graph, as the C++ `shared_ptr` owners go out of scope with the
/// returned error. The caller never receives its table, so nothing else could despawn it.
#[test]
fn a_failed_callback_tail_drops_the_finished_graph() {
    let mut world = World::new();
    let registry = registry(&[("NiFloatData", 0x006E_3440)]);
    let file = one_object_file();
    let mut preload = StreamOwnerArray::default();
    let mut terminal = StreamTerminalState::default();
    let (mut enter, mut thread, mut leave) = (|| {}, || 0u32, || {});
    let mut services = StreamTerminalServices {
        enter_critical_section: Some(&mut enter),
        current_thread_id: Some(&mut thread),
        leave_critical_section: Some(&mut leave),
        ..Default::default()
    };
    let mut cursors = StreamFileCursors::default();
    let mut control = StreamRootCallbackControl::default();
    let mut current = |_: &World| vec![0xDEAD_BEEF];
    let error = load_supported_stream_file(
        &mut world,
        &registry,
        &file,
        StreamFilePipelineRuntime {
            preload: &mut preload,
            terminal: &mut terminal,
            owner_services: &mut services,
            cursors: &mut cursors,
            control: &mut control,
            link_runtime: StreamGraphLinkRuntime::default(),
            post_load_runtime: StreamPostLoadRuntime::default(),
            callback_runtime: StreamRootCallbackRuntime {
                current_callbacks: Some(&mut current),
                invoke: None,
            },
            publish_loaded_object: None,
        },
    )
    .unwrap_err();
    assert_eq!(error.stage, StreamFilePipelineErrorStage::Callback);
    assert_eq!(objects(&mut world), 0);
}
