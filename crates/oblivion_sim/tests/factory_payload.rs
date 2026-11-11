//! Ported from the C++ `tests/gamebryo/factory_payload_tests.cpp`.

use bevy_ecs::prelude::*;
use oblivion_sim::gamebryo::animation::{FloatKey, FloatKeyData, FloatKeyType};
use oblivion_sim::gamebryo::bool_animation::{BoolKey, BoolKeyData, BoolKeyType};
use oblivion_sim::gamebryo::color_animation::{ColorKey, ColorKeyData, ColorKeyType};
use oblivion_sim::gamebryo::extra_data::{
    BinaryExtraData, StringExtraData, TextKey, TextKeyExtraData,
};
use oblivion_sim::gamebryo::factory_payload::{
    FactoryPayloadResult, FactoryPayloadStatus, FactoryPendingPayload,
    decode_registered_factory_payload, load_factory_payload_into_owner,
};
use oblivion_sim::gamebryo::object_net::NiExtraDataName;
use oblivion_sim::gamebryo::particle_controller::{
    ParticleEmitterController, ParticleUpdateController,
};
use oblivion_sim::gamebryo::particle_mesh_emitter::ParticleMeshEmitter;
use oblivion_sim::gamebryo::position_animation::{PositionKeyData, PositionKeyType};
use oblivion_sim::gamebryo::registration::{FactoryCreate, NiFactoryRegistry};
use oblivion_sim::gamebryo::stream_objects::NiClass;
use oblivion_sim::gamebryo::string_palette::StringPalette;

const V: u32 = 0x1400_0004;
const NULL: u32 = 0xFFFF_FFFF;
const FLOATING: [u8; 17] = [1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 0xAB];
const PALETTE: [u8; 12] = [3, 0, 0, 0, b'A', 0, b'Z', 2, 0, 0, 0, 0xAB];

fn decode(
    world: &mut World,
    registry: &NiFactoryRegistry,
    class: &str,
    bytes: &[u8],
    version: u32,
) -> FactoryPayloadResult {
    decode_registered_factory_payload(world, registry, class, bytes, version).unwrap()
}

fn objects(world: &mut World) -> usize {
    world.query::<&NiClass>().iter(world).count()
}

fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_le_bytes());
}

fn cold(world: &mut World, registry: &NiFactoryRegistry, class: &str) -> Entity {
    match registry.create(world, class) {
        FactoryCreate::Created { entity, .. } => entity,
        other => panic!("{class} not created: {other:?}"),
    }
}

/// 00712590 and seven verified leaf LoadBinary routines.
#[test]
fn seven_leaf_payloads() {
    let mut world = World::new();
    let mut registry = NiFactoryRegistry::default();
    registry.insert_or_assign("NiFloatData", 0x006E_3440);
    registry.insert_or_assign("NiBoolData", 0x006E_8740);
    registry.insert_or_assign("NiColorData", 0x006E_4520);
    registry.insert_or_assign("NiStringPalette", 0x006D_7AD0);
    registry.insert_or_assign("NiStringExtraData", 0x0071_6B20);
    registry.insert_or_assign("NiBinaryExtraData", 0x0072_7F20);
    registry.insert_or_assign("NiTextKeyExtraData", 0x006D_7450);

    let f = decode(&mut world, &registry, "NiFloatData", &FLOATING, V);
    assert_eq!(f.status, FactoryPayloadStatus::Decoded);
    assert_eq!(f.bytes_consumed, 16);
    assert_eq!(f.original_constructor, Some(0x006E_3440));
    let f_owner = f.entity.unwrap();
    assert_eq!(
        world.get::<FloatKeyData>(f_owner).unwrap().keys()[0].value,
        2.0
    );

    let boolean = [1, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xAB];
    let b = decode(&mut world, &registry, "NiBoolData", &boolean, V);
    assert_eq!(b.status, FactoryPayloadStatus::Decoded);
    assert_eq!(b.bytes_consumed, 13);
    let b_owner = b.entity.unwrap();
    assert_eq!(
        world.get::<BoolKeyData>(b_owner).unwrap().keys()[0].value,
        1
    );

    let color = [
        1, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x3F, 0, 0, 0, 0x40, 0, 0, 0x40, 0x40, 0,
        0, 0x80, 0x40, 0xAB,
    ];
    let c = decode(&mut world, &registry, "NiColorData", &color, V);
    assert_eq!(c.status, FactoryPayloadStatus::Decoded);
    assert_eq!(c.bytes_consumed, 28);
    assert_eq!(c.original_constructor, Some(0x006E_4520));
    let color_owner = world.get::<ColorKeyData>(c.entity.unwrap()).unwrap();
    assert_eq!(color_owner.key_type(), ColorKeyType::STEP);
    let mut color_index = 7;
    let sample = color_owner.sample(4.0, &mut color_index).unwrap();
    assert_eq!(sample[0], 1.0);
    assert_eq!(sample[3], 4.0);
    assert_eq!(color_index, 7);

    let p = decode(&mut world, &registry, "NiStringPalette", &PALETTE, V);
    assert_eq!(p.status, FactoryPayloadStatus::Decoded);
    assert_eq!(p.bytes_consumed, 11);
    let palette = world.get::<StringPalette>(p.entity.unwrap()).unwrap();
    assert_eq!(palette.capacity(), 3);
    assert_eq!(palette.used_length(), 2);

    let string_extra = [0, 0, 0, 0, 3, 0, 0, 0, b'a', b'b', b'c', 0xAB];
    let s = decode(&mut world, &registry, "NiStringExtraData", &string_extra, V);
    assert_eq!(s.status, FactoryPayloadStatus::Decoded);
    assert_eq!(s.bytes_consumed, 11);
    let s_owner = s.entity.unwrap();
    assert_eq!(world.get::<NiExtraDataName>(s_owner).unwrap().0, None);
    assert_eq!(
        world
            .get::<StringExtraData>(s_owner)
            .unwrap()
            .value
            .as_deref(),
        Some(&b"abc"[..])
    );

    let binary_extra = [0, 0, 0, 0, 3, 0, 0, 0, 0, 0xFF, 0x41, 0xAB];
    let x = decode(&mut world, &registry, "NiBinaryExtraData", &binary_extra, V);
    assert_eq!(x.status, FactoryPayloadStatus::Decoded);
    assert_eq!(x.bytes_consumed, 11);
    assert_eq!(
        world
            .get::<BinaryExtraData>(x.entity.unwrap())
            .unwrap()
            .bytes,
        [0, 0xFF, 0x41]
    );

    let text_extra = [
        0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0x80, 0x3F, 4, 0, 0, 0, b't', b'e', b'x', b't', 0xAB,
    ];
    let t = decode(&mut world, &registry, "NiTextKeyExtraData", &text_extra, V);
    assert_eq!(t.status, FactoryPayloadStatus::Decoded);
    assert_eq!(t.bytes_consumed, 20);
    assert_eq!(t.original_constructor, Some(0x006D_7450));
    let keys = &world
        .get::<TextKeyExtraData>(t.entity.unwrap())
        .unwrap()
        .keys;
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].text.as_deref(), Some(&b"text"[..]));
}

/// The host adapter's statuses: catalog, version and error stay separate.
#[test]
fn status_and_failure() {
    let mut world = World::new();
    let mut registry = NiFactoryRegistry::default();
    let missing = decode(&mut world, &registry, "NiFloatData", &FLOATING, V);
    assert_eq!(missing.status, FactoryPayloadStatus::Unregistered);
    assert_eq!(missing.requested_name, "NiFloatData");
    assert_eq!(missing.original_constructor, None);
    assert_eq!(missing.bytes_consumed, 0);
    assert_eq!(missing.entity, None);

    registry.insert_or_assign("NiCamera", 0x0070_D730);
    registry.insert_or_assign("NiFloatInterpolator", 0x006D_2DD0);
    for name in ["NiCamera", "NiFloatInterpolator"] {
        let unsupported = decode(&mut world, &registry, name, &FLOATING, V);
        assert_eq!(unsupported.status, FactoryPayloadStatus::UnsupportedTarget);
        assert!(unsupported.original_constructor.is_some());
        assert_eq!(unsupported.bytes_consumed, 0);
        assert_eq!(unsupported.entity, None);
    }

    registry.insert_or_assign("NiFloatData", 0x006E_3440);
    registry.insert_or_assign("AliasFloat", 0x006E_3440);
    let old = decode(&mut world, &registry, "NiFloatData", &FLOATING, 0x0A01_0071);
    assert_eq!(old.status, FactoryPayloadStatus::UnsupportedVersion);
    assert_eq!(old.original_constructor, Some(0x006E_3440));
    assert_eq!(old.bytes_consumed, 0);
    assert_eq!(old.entity, None);
    assert_eq!(objects(&mut world), 0);
    let alias = decode(&mut world, &registry, "AliasFloat", &FLOATING, 0x0A01_0072);
    assert_eq!(alias.status, FactoryPayloadStatus::Decoded);
    assert_eq!(alias.requested_name, "AliasFloat");
    assert_eq!(alias.bytes_consumed, 16);
    world.despawn(alias.entity.unwrap());

    for length in 0..16 {
        assert!(
            decode_registered_factory_payload(
                &mut world,
                &registry,
                "NiFloatData",
                &FLOATING[..length],
                V
            )
            .is_err(),
            "a malformed payload is not published"
        );
        assert_eq!(objects(&mut world), 0, "the cold owner goes with it");
    }
    registry.insert_or_assign("NiFloatData", 0x0070_D730);
    let rebound = decode(&mut world, &registry, "NiFloatData", &FLOATING, V);
    assert_eq!(rebound.status, FactoryPayloadStatus::UnsupportedTarget);
    assert_eq!(rebound.entity, None);
}

/// 007008A0's leaf LoadBinary into an owner 00713FF0 made earlier: a temporary parse and a stable identity.
#[test]
fn existing_owner_load() {
    let mut world = World::new();
    let owner = world
        .spawn((
            NiClass::StringPalette,
            StringPalette::new(b"OLD".to_vec(), 3).unwrap(),
        ))
        .id();
    let load = |world: &mut World, constructor, entity, bytes: &[u8], version| {
        load_factory_payload_into_owner(world, constructor, entity, bytes, version, None)
    };
    let loaded = load(&mut world, 0x006D_7AD0, owner, &PALETTE, V).unwrap();
    assert_eq!(loaded.status, FactoryPayloadStatus::Decoded);
    assert_eq!(loaded.bytes_consumed, 11);
    let palette = world.get::<StringPalette>(owner).unwrap();
    assert_eq!(palette.capacity(), 3);
    assert_eq!(palette.used_length(), 2);
    assert_eq!(palette.bytes()[0], b'A');
    assert_eq!(palette.bytes()[1], 0);

    let before = palette.clone();
    for length in 0..11 {
        assert!(load(&mut world, 0x006D_7AD0, owner, &PALETTE[..length], V).is_err());
        assert_eq!(world.get::<StringPalette>(owner), Some(&before));
    }
    let invalid_used = [3, 0, 0, 0, b'A', 0, b'Z', 4, 0, 0, 0];
    assert!(load(&mut world, 0x006D_7AD0, owner, &invalid_used, V).is_err());
    assert_eq!(world.get::<StringPalette>(owner), Some(&before));

    let wrong = world
        .spawn((NiClass::FloatKeyData, FloatKeyData::default()))
        .id();
    let mismatch = load(&mut world, 0x006D_7AD0, wrong, &[], V).unwrap();
    assert_eq!(mismatch.status, FactoryPayloadStatus::OwnerMismatch);
    assert_eq!(mismatch.bytes_consumed, 0);
    assert!(world.get::<FloatKeyData>(wrong).unwrap().keys().is_empty());
    assert_eq!(world.get::<StringPalette>(owner), Some(&before));
    let null_owner = world.spawn_empty().id();
    let null_result = load(&mut world, 0x006D_7AD0, null_owner, &PALETTE, V).unwrap();
    assert_eq!(null_result.status, FactoryPayloadStatus::OwnerMismatch);
    assert_eq!(null_result.bytes_consumed, 0);
    let old = load(&mut world, 0x006D_7AD0, owner, &PALETTE, 0x0A01_0071).unwrap();
    let unsupported = load(&mut world, 0x006D_2DD0, owner, &PALETTE, V).unwrap();
    assert_eq!(old.status, FactoryPayloadStatus::UnsupportedVersion);
    assert_eq!(unsupported.status, FactoryPayloadStatus::UnsupportedTarget);
    assert_eq!(old.bytes_consumed, 0);
    assert_eq!(unsupported.bytes_consumed, 0);
    assert_eq!(world.get::<StringPalette>(owner), Some(&before));
}

/// 006E3690 / 006E8A30 / 006D7780's zero-count branches: identity, conditional key retention and a base-name update.
#[test]
fn zero_count_existing_owners() {
    let mut world = World::new();
    let floating = world
        .spawn((
            NiClass::FloatKeyData,
            FloatKeyData::new(
                FloatKeyType::LINEAR,
                vec![FloatKey {
                    time: 1.0,
                    value: 8.0,
                    ..FloatKey::default()
                }],
            )
            .unwrap(),
        ))
        .id();
    let boolean = world
        .spawn((
            NiClass::BoolKeyData,
            BoolKeyData::new(
                BoolKeyType::STEP,
                vec![BoolKey {
                    time: 2.0,
                    value: 1,
                }],
            )
            .unwrap(),
        ))
        .id();
    let color = world
        .spawn((
            NiClass::ColorKeyData,
            ColorKeyData::new(
                ColorKeyType::LINEAR,
                vec![ColorKey {
                    time: 2.0,
                    rgba: [1.0, 2.0, 3.0, 4.0],
                }],
            )
            .unwrap(),
        ))
        .id();
    let text = world
        .spawn((
            NiClass::TextKeyExtraData,
            NiExtraDataName(Some(b"old".to_vec())),
            TextKeyExtraData {
                keys: vec![TextKey {
                    time_bits: 3.0f32.to_bits(),
                    text: Some(b"kept".to_vec()),
                }],
            },
        ))
        .id();
    let zero_count = [0, 0, 0, 0];
    let renamed_text = [3, 0, 0, 0, b'n', b'e', b'w', 0, 0, 0, 0];
    let mut load = |constructor, entity, bytes: &[u8]| {
        load_factory_payload_into_owner(&mut world, constructor, entity, bytes, V, None).unwrap()
    };
    let f = load(0x006E_3440, floating, &zero_count);
    let b = load(0x006E_8740, boolean, &zero_count);
    let c = load(0x006E_4520, color, &zero_count);
    let t = load(0x006D_7450, text, &renamed_text);
    for (result, consumed) in [(&f, 4), (&b, 4), (&c, 4), (&t, 11)] {
        assert_eq!(result.status, FactoryPayloadStatus::Decoded);
        assert_eq!(result.bytes_consumed, consumed);
    }
    let keys = world.get::<FloatKeyData>(floating).unwrap().keys();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].value, 8.0);
    let keys = world.get::<BoolKeyData>(boolean).unwrap().keys();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].value, 1);
    let owner = world.get::<ColorKeyData>(color).unwrap();
    assert_eq!(owner.key_type(), ColorKeyType::LINEAR);
    assert_eq!(owner.keys().len(), 1);
    assert_eq!(owner.keys()[0].rgba[2], 3.0);
    assert_eq!(
        world.get::<NiExtraDataName>(text).unwrap().0.as_deref(),
        Some(&b"new"[..])
    );
    let keys = &world.get::<TextKeyExtraData>(text).unwrap().keys;
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].text.as_deref(), Some(&b"kept"[..]));
}

/// 006D9D40 / 006D9F90 and 00759DE0's tighter version floor.
#[test]
fn position_and_particle_status() {
    let mut world = World::new();
    let mut registry = NiFactoryRegistry::default();
    registry.insert_or_assign("NiPosData", 0x006D_9D40);
    registry.insert_or_assign("NiPSysData", 0x0075_9C50);
    let position = [
        1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x3F, 0, 0, 0, 0x40, 0, 0, 0x40, 0x40, 0xAB,
    ];
    let p = decode(&mut world, &registry, "NiPosData", &position, V);
    assert_eq!(p.status, FactoryPayloadStatus::Decoded);
    assert_eq!(p.bytes_consumed, 24);
    assert_eq!(p.original_constructor, Some(0x006D_9D40));
    let owner = p.entity.unwrap();
    let data = world.get::<PositionKeyData>(owner).unwrap();
    assert_eq!(data.key_type(), PositionKeyType::LINEAR);
    assert_eq!(data.keys().len(), 1);
    assert_eq!(data.keys()[0].position, [1.0, 2.0, 3.0]);
    let zero = [0, 0, 0, 0];
    let no_op =
        load_factory_payload_into_owner(&mut world, 0x006D_9D40, owner, &zero, V, None).unwrap();
    assert_eq!(no_op.status, FactoryPayloadStatus::Decoded);
    assert_eq!(no_op.bytes_consumed, 4);
    assert_eq!(world.get::<PositionKeyData>(owner).unwrap().keys().len(), 1);
    let unsupported = decode(&mut world, &registry, "NiPSysData", &zero, 0x0A01_0071);
    assert_eq!(unsupported.status, FactoryPayloadStatus::UnsupportedVersion);
    assert_eq!(unsupported.original_constructor, Some(0x0075_9C50));
    assert_eq!(unsupported.bytes_consumed, 0);
}

/// 00753BB0 / 0074FDE0 / 0074BB20's factories and 00715F40 / 0074F980 / 0074BB40's loaders. Raw controller references and
/// mesh indexes stay typed pending link data.
#[test]
fn controller_and_mesh_factory_payloads() {
    let mut world = World::new();
    let mut registry = NiFactoryRegistry::default();
    registry.insert_or_assign("NiPSysUpdateCtlr", 0x0075_3BB0);
    registry.insert_or_assign("NiPSysEmitterCtlr", 0x0074_FDE0);
    registry.insert_or_assign("NiPSysMeshEmitter", 0x0074_BB20);
    let update_cold = cold(&mut world, &registry, "NiPSysUpdateCtlr");
    let emitter_cold = cold(&mut world, &registry, "NiPSysEmitterCtlr");
    let mesh_cold = cold(&mut world, &registry, "NiPSysMeshEmitter");
    assert_eq!(
        world
            .get::<ParticleUpdateController>(update_cold)
            .unwrap()
            .time
            .fields
            .flags,
        12
    );
    assert_eq!(
        world
            .get::<ParticleEmitterController>(emitter_cold)
            .unwrap()
            .time
            .fields
            .flags,
        12
    );
    assert!(
        world
            .get::<ParticleMeshEmitter>(mesh_cold)
            .unwrap()
            .mesh_indices
            .is_empty()
    );

    let mut time = Vec::new();
    word(&mut time, 3); // deferred next
    time.extend([0x34, 0x12]); // u16 flags
    for raw in [0x3F80_0000, 0x4000_0000, 0x4040_0000, 0x4080_0000] {
        word(&mut time, raw);
    }
    word(&mut time, 7); // deferred target
    let mut update_bytes = time.clone();
    update_bytes.push(0xA5);
    let update = decode(&mut world, &registry, "NiPSysUpdateCtlr", &update_bytes, V);
    assert_eq!(update.status, FactoryPayloadStatus::Decoded);
    assert_eq!(update.bytes_consumed, 26);
    let FactoryPendingPayload::TimeController(pending) = &update.pending else {
        panic!("update controller lost its pending links");
    };
    assert_eq!(pending.next_index, 3);
    assert_eq!(pending.target_index, 7);
    assert_eq!(pending.flags, 0x1234);
    assert_eq!(
        world
            .get::<ParticleUpdateController>(update.entity.unwrap())
            .unwrap()
            .time
            .fields
            .flags,
        0x1234
    );

    let mut emitter_bytes = time.clone();
    word(&mut emitter_bytes, 9); // immediate birth-rate owner
    word(&mut emitter_bytes, 0); // absent name
    word(&mut emitter_bytes, 11); // deferred active owner
    emitter_bytes.push(0xA5);
    let emitter = decode(
        &mut world,
        &registry,
        "NiPSysEmitterCtlr",
        &emitter_bytes,
        V,
    );
    assert_eq!(emitter.status, FactoryPayloadStatus::Decoded);
    assert_eq!(emitter.bytes_consumed, 38);
    let FactoryPendingPayload::EmitterController(pending) = &emitter.pending else {
        panic!("emitter controller lost its pending links");
    };
    assert_eq!(pending.birth_rate_interpolator_index, 9);
    assert_eq!(pending.active_interpolator_index, 11);
    let owner = world
        .get::<ParticleEmitterController>(emitter.entity.unwrap())
        .unwrap();
    assert_eq!(owner.pending_birth_rate_interpolator_index, 9);
    assert_eq!(owner.pending_active_interpolator_index, 11);

    let mut mesh_bytes = Vec::new();
    word(&mut mesh_bytes, 4);
    mesh_bytes.extend(b"Mesh");
    word(&mut mesh_bytes, 3); // order
    word(&mut mesh_bytes, NULL); // target
    mesh_bytes.push(1); // active
    for _ in 0..14 {
        word(&mut mesh_bytes, 0); // emitter scalars
    }
    word(&mut mesh_bytes, 2); // mesh count
    word(&mut mesh_bytes, 4);
    word(&mut mesh_bytes, NULL);
    word(&mut mesh_bytes, 7); // velocity mode
    word(&mut mesh_bytes, 3); // emission mode
    word(&mut mesh_bytes, 0x3F80_0000);
    word(&mut mesh_bytes, 0x8000_0000);
    word(&mut mesh_bytes, 0);
    let mesh_size = mesh_bytes.len();
    mesh_bytes.push(0xA5);
    let mesh = decode(&mut world, &registry, "NiPSysMeshEmitter", &mesh_bytes, V);
    assert_eq!(mesh.status, FactoryPayloadStatus::Decoded);
    assert_eq!(mesh.bytes_consumed, mesh_size);
    let FactoryPendingPayload::MeshEmitter(pending) = &mesh.pending else {
        panic!("mesh emitter lost its pending links");
    };
    assert_eq!(pending.mesh_indices, [4, NULL]);
    assert_eq!(pending.initial_velocity_type, 7);
    assert_eq!(pending.emission_type, 3);
    assert!(
        world
            .get::<ParticleMeshEmitter>(mesh.entity.unwrap())
            .unwrap()
            .mesh_indices
            .is_empty(),
        "links wait for the link pass"
    );

    let loaded = load_factory_payload_into_owner(
        &mut world,
        0x0075_3BB0,
        update_cold,
        &update_bytes,
        V,
        None,
    )
    .unwrap();
    assert_eq!(loaded.status, FactoryPayloadStatus::Decoded);
    assert_eq!(loaded.bytes_consumed, 26);
    assert_eq!(
        world
            .get::<ParticleUpdateController>(update_cold)
            .unwrap()
            .time
            .fields
            .flags,
        0x1234
    );
    assert!(matches!(
        loaded.pending,
        FactoryPendingPayload::TimeController(_)
    ));
    for (constructor, owner, bytes, extent) in [
        (0x0075_3BB0, update_cold, &update_bytes, 26),
        (0x0074_FDE0, emitter_cold, &emitter_bytes, 38),
        (0x0074_BB20, mesh_cold, &mesh_bytes, mesh_size),
    ] {
        assert!(
            load_factory_payload_into_owner(
                &mut world,
                constructor,
                owner,
                &bytes[..extent - 1],
                V,
                None
            )
            .is_err(),
            "a truncated payload is not accepted"
        );
    }
}
