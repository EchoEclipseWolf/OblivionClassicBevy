//! The stream loads of the scalar and transform interpolators, resolved against the object table.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;

use super::animation::FloatInterpolator;
use super::bool_animation::{BoolInterpolator, BoolTimelineInterpolator};
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};
use super::transform_animation::{TransformInterpolator, decode_quat_transform};

const TRUNCATED: &str = "truncated interpolator payload";

// IDA: N/A host preflight for 007008A0's no-op modern base load
// Completion: 100% of the supported-version gate, with older base payloads rejected
// TODO: [ ] Port older NiObject base fields and link policy separately.
fn require_modern(version: u32) -> Result<(), GameError> {
    if version < 0x0A01_0072 {
        return Err(GameError::unclassified(
            0,
            "legacy interpolator payload unsupported",
        ));
    }
    Ok(())
}

// IDA: 00712A90 NiStream_ReadObjectReference uses FFFFFFFF null or a stream object-array index; the host checks the index
// and the exact class, each failure with the caller's error
// Completion: 100% of the bounded host index and exact class validation
// TODO: [ ] The original's index and type are unchecked here; compare its behaviour if a bad stream reaches it.
pub(super) fn linked_data(
    world: &World,
    table: &StreamObjectTable,
    index: u32,
    class: NiClass,
    out_of_range: GameError,
    wrong_class: GameError,
) -> Result<Option<Entity>, GameError> {
    if index == NULL_REFERENCE {
        return Ok(None);
    }
    let Some(object) = table.objects.get(index as usize) else {
        return Err(out_of_range);
    };
    if world.get::<NiClass>(object.entity) != Some(&class) {
        return Err(wrong_class);
    }
    Ok(Some(object.entity))
}

/// IDA: N/A the errors of this file's `linked_data` calls.
const LINK_OUT_OF_RANGE: GameError =
    GameError::unclassified(0, "interpolator link index out of range");
/// IDA: N/A as `LINK_OUT_OF_RANGE`.
const LINK_WRONG_TYPE: GameError =
    GameError::unclassified(0, "interpolator link has wrong data type");

// IDA: N/A host exact target guard before stream publication
// Completion: 100% of the table index and class validation for an existing interpolator
// TODO: [ ] Bind original virtual object type checks after graph reconstruction.
fn target_interpolator(
    world: &World,
    table: &StreamObjectTable,
    object_index: usize,
    class: NiClass,
) -> Result<Entity, GameError> {
    let Some(object) = table.objects.get(object_index) else {
        return Err(GameError::unclassified(
            0,
            "interpolator object index out of range",
        ));
    };
    if world.get::<NiClass>(object.entity) != Some(&class) {
        return Err(GameError::unclassified(
            0,
            "stream object is not requested interpolator type",
        ));
    }
    Ok(object.entity)
}

// IDA: 006D2F60 float scalar then 00712A90 object link; 007008A0 NiObject base consumes no bytes for >= 0x0A010072
// Completion: 100% of the modern float and link payload; a failure leaves the target unchanged
// TODO: [ ] Integrate with the verified per-object NiStream load order; bind the stream callback and endian.
pub fn load_float_interpolator_payload(
    world: &mut World,
    table: &StreamObjectTable,
    object_index: usize,
    bytes: &[u8],
    version: u32,
) -> Result<usize, GameError> {
    require_modern(version)?;
    let target = target_interpolator(world, table, object_index, NiClass::FloatInterpolator)?;
    let mut reader = ByteReader::new(bytes);
    let scalar = reader.f32(TRUNCATED)?;
    let link = reader.u32(TRUNCATED)?;
    let data = linked_data(
        world,
        table,
        link,
        NiClass::FloatKeyData,
        LINK_OUT_OF_RANGE,
        LINK_WRONG_TYPE,
    )?;
    world
        .get_mut::<FloatInterpolator>(target)
        .expect("a NiFloatInterpolator entity")
        .load_scalar_and_data(scalar, data);
    Ok(reader.position())
}

// IDA: 006E8550 shared by the 006E7B50 timeline jump
// Completion: 100% of the raw byte and link parse, before publication
// TODO: [ ] Bind original callback failure behaviour.
fn read_bool_body(
    world: &World,
    table: &StreamObjectTable,
    bytes: &[u8],
) -> Result<(u8, Option<Entity>, usize), GameError> {
    let mut reader = ByteReader::new(bytes);
    let scalar = reader.u8("truncated bool interpolator scalar")?;
    let link = reader.u32(TRUNCATED)?;
    let data = linked_data(
        world,
        table,
        link,
        NiClass::BoolKeyData,
        LINK_OUT_OF_RANGE,
        LINK_WRONG_TYPE,
    )?;
    Ok((scalar, data, reader.position()))
}

// IDA: 006E8550 raw bool byte then 00712A90 object link; 007008A0's modern NiObject base consumes no bytes
// Completion: 100% of the modern raw-bool and link payload
// TODO: [ ] Bind the verified per-object stream load order.
pub fn load_bool_interpolator_payload(
    world: &mut World,
    table: &StreamObjectTable,
    object_index: usize,
    bytes: &[u8],
    version: u32,
) -> Result<usize, GameError> {
    require_modern(version)?;
    let target = target_interpolator(world, table, object_index, NiClass::BoolInterpolator)?;
    let (scalar, data, consumed) = read_bool_body(world, table, bytes)?;
    world
        .get_mut::<BoolInterpolator>(target)
        .expect("a NiBoolInterpolator entity")
        .load_scalar_and_data(scalar, data);
    Ok(consumed)
}

// IDA: 006E7B50 is a direct jump to 006E8550
// Completion: 100% of the inherited raw bool and link payload, preserving the timeline's previous index and raw value
// TODO: [ ] Bind timeline graph scheduling and runtime update validation.
pub fn load_bool_timeline_interpolator_payload(
    world: &mut World,
    table: &StreamObjectTable,
    object_index: usize,
    bytes: &[u8],
    version: u32,
) -> Result<usize, GameError> {
    require_modern(version)?;
    let target = target_interpolator(
        world,
        table,
        object_index,
        NiClass::BoolTimelineInterpolator,
    )?;
    let (scalar, data, consumed) = read_bool_body(world, table, bytes)?;
    world
        .get_mut::<BoolTimelineInterpolator>(target)
        .expect("a NiBoolTimelineInterpolator entity")
        .base
        .load_scalar_and_data(scalar, data);
    Ok(consumed)
}

// IDA: 006D67B0: 006EC2B0 -> 006EBA80 -> NiObject (no modern bytes), 006CB990 NiQuatTransform, then the 00712A90
// immediate reference stored at +0x2C
// Completion: 100% of the bounded modern payload and link order
// TODO: [ ] Bind original refcount release timing on populated owners, and the per-object NiStream load order.
pub fn load_transform_interpolator_payload(
    world: &mut World,
    table: &StreamObjectTable,
    object_index: usize,
    bytes: &[u8],
    version: u32,
) -> Result<usize, GameError> {
    // 006D67B0 has no gate of its own: 10.1.0.106 reads as a modern stream but for 006CB990's validity bytes (below 0A01006E).
    if version != 0x0A01_006A {
        require_modern(version)?;
    }
    let target = target_interpolator(world, table, object_index, NiClass::TransformInterpolator)?;
    let transform = decode_quat_transform(bytes, version)?;
    let mut reader = ByteReader::new(&bytes[transform.bytes_consumed..]);
    let link = reader.u32(TRUNCATED)?;
    let data = linked_data(
        world,
        table,
        link,
        NiClass::TransformKeyData,
        LINK_OUT_OF_RANGE,
        LINK_WRONG_TYPE,
    )?;
    world
        .get_mut::<TransformInterpolator>(target)
        .expect("a NiTransformInterpolator entity")
        .load_transform_and_data(&transform.value, data);
    Ok(transform.bytes_consumed + reader.position())
}

#[cfg(test)]
mod tests {
    use super::super::animation::{
        FloatKey, FloatKeyData, FloatKeyType, INVALID_FLOAT_INTERPOLATOR_VALUE,
        update_float_interpolator,
    };
    use super::super::bool_animation::{
        BoolKey, BoolKeyData, BoolKeyType, INVALID_BOOL_INTERPOLATOR_VALUE,
        update_bool_interpolator, update_bool_timeline_interpolator,
    };
    use super::super::registration::{FactoryCreate, NiFactoryRegistry};
    use super::*;

    /// IDA: 00713FF0 creates all cold objects before payload loading.
    fn make_table(world: &mut World) -> StreamObjectTable {
        let mut registry = NiFactoryRegistry::default();
        registry.insert_or_assign("NiFloatInterpolator", 0x006D_2DD0);
        registry.insert_or_assign("NiFloatData", 0x006E_3440);
        registry.insert_or_assign("NiBoolInterpolator", 0x006E_8370);
        registry.insert_or_assign("NiBoolData", 0x006E_8740);
        registry.insert_or_assign("NiBoolTimelineInterpolator", 0x006E_7E70);
        let mut table = StreamObjectTable::default();
        for name in [
            "NiFloatInterpolator",
            "NiFloatData",
            "NiBoolInterpolator",
            "NiBoolData",
            "NiBoolTimelineInterpolator",
        ] {
            let FactoryCreate::Created {
                original_constructor,
                entity,
                ..
            } = registry.create(world, name)
            else {
                panic!("{name}: cold factory unavailable");
            };
            table.push(name, original_constructor, entity);
        }
        table
    }

    /// Ported from `interpolator_stream_tests`: IDA 006D2F60 / 006E8550 / 00712A90's forward owners and fields.
    #[test]
    fn forward_links_and_owner_identity() {
        let mut world = World::new();
        let table = make_table(&mut world);
        let (floating, float_data) = (table.objects[0].entity, table.objects[1].entity);
        let (boolean, bool_data) = (table.objects[2].entity, table.objects[3].entity);
        // 2.5, forward link to object 1.
        let float_payload: [u8; 9] = [0, 0, 0x20, 0x40, 1, 0, 0, 0, 0xAB];
        // A raw noncanonical byte, forward link to object 3.
        let bool_payload: [u8; 6] = [0xFE, 3, 0, 0, 0, 0xAB];
        assert_eq!(
            load_float_interpolator_payload(&mut world, &table, 0, &float_payload, 0x0A01_0072),
            Ok(8)
        );
        assert_eq!(
            load_bool_interpolator_payload(&mut world, &table, 2, &bool_payload, 0x1400_0004),
            Ok(5)
        );
        let float_state = *world.get::<FloatInterpolator>(floating).unwrap();
        let bool_state = *world.get::<BoolInterpolator>(boolean).unwrap();
        assert_eq!(table.objects[0].entity, floating);
        assert_eq!(table.objects[2].entity, boolean);
        assert_eq!(float_state.data_10, Some(float_data));
        assert_eq!(bool_state.data_10, Some(bool_data));
        assert!(
            world
                .get::<FloatKeyData>(float_data)
                .unwrap()
                .keys()
                .is_empty()
        );
        assert!(
            world
                .get::<BoolKeyData>(bool_data)
                .unwrap()
                .keys()
                .is_empty()
        );
        assert_eq!(float_state.cached_value_0c.to_bits(), 0x4020_0000);
        assert_eq!(bool_state.cached_value_0c, 0xFE);
        let mut float_output = 0.0;
        let mut bool_output = false;
        assert!(update_float_interpolator(
            &mut world,
            floating,
            0.0,
            &mut float_output
        ));
        assert_eq!(float_output, 2.5);
        assert!(update_bool_interpolator(
            &mut world,
            boolean,
            0.0,
            &mut bool_output
        ));
        assert!(bool_output);

        *world.get_mut::<FloatKeyData>(float_data).unwrap() = FloatKeyData::new(
            FloatKeyType::LINEAR,
            vec![FloatKey {
                time: 0.0,
                value: 7.0,
                ..FloatKey::default()
            }],
        )
        .unwrap();
        *world.get_mut::<BoolKeyData>(bool_data).unwrap() = BoolKeyData::new(
            BoolKeyType::STEP,
            vec![BoolKey {
                time: 0.0,
                value: 0,
            }],
        )
        .unwrap();
        assert!(update_float_interpolator(
            &mut world,
            floating,
            1.0,
            &mut float_output
        ));
        assert_eq!(float_output, 7.0);
        assert!(update_bool_interpolator(
            &mut world,
            boolean,
            1.0,
            &mut bool_output
        ));
        assert!(!bool_output);
    }

    /// IDA: 00712A90's FFFFFFFF null link; 006E7B50 jumps to 006E8550.
    #[test]
    fn null_and_timeline() {
        let mut world = World::new();
        let table = make_table(&mut world);
        let floating = table.objects[0].entity;
        let bool_data = table.objects[3].entity;
        let timeline = table.objects[4].entity;
        *world.get_mut::<BoolKeyData>(bool_data).unwrap() = BoolKeyData::new(
            BoolKeyType::STEP,
            vec![
                BoolKey {
                    time: 0.0,
                    value: 0,
                },
                BoolKey {
                    time: 1.0,
                    value: 1,
                },
            ],
        )
        .unwrap();
        world
            .get_mut::<BoolTimelineInterpolator>(timeline)
            .unwrap()
            .set_timeline_data(Some(bool_data));
        let mut output = false;
        assert!(update_bool_timeline_interpolator(
            &mut world,
            timeline,
            1.0,
            &mut output
        ));
        assert!(output);
        let before = *world.get::<BoolTimelineInterpolator>(timeline).unwrap();
        assert_eq!(before.previous_raw_value, 1);
        let timeline_payload: [u8; 6] = [0x7F, 3, 0, 0, 0, 0xAB];
        assert_eq!(
            load_bool_timeline_interpolator_payload(
                &mut world,
                &table,
                4,
                &timeline_payload,
                0x1400_0004
            ),
            Ok(5)
        );
        let after = *world.get::<BoolTimelineInterpolator>(timeline).unwrap();
        assert_eq!(table.objects[4].entity, timeline);
        assert_eq!(after.base.data_10, Some(bool_data));
        assert_eq!(after.base.cached_value_0c, 0x7F);
        assert_eq!(after.base.last_time_08, before.base.last_time_08);
        assert_eq!(after.previous_key_index, before.previous_key_index);
        assert_eq!(after.previous_raw_value, 1);

        let null_payload: [u8; 8] = [0, 0, 0x80, 0x3F, 0xFF, 0xFF, 0xFF, 0xFF];
        let held = world
            .spawn(
                FloatKeyData::new(
                    FloatKeyType::LINEAR,
                    vec![FloatKey {
                        time: 0.0,
                        value: 9.0,
                        ..FloatKey::default()
                    }],
                )
                .unwrap(),
            )
            .id();
        world
            .get_mut::<FloatInterpolator>(floating)
            .unwrap()
            .set_data(Some(held));
        assert_eq!(
            load_float_interpolator_payload(&mut world, &table, 0, &null_payload, 0x1400_0004),
            Ok(8)
        );
        let state = world.get::<FloatInterpolator>(floating).unwrap();
        assert!(state.data_10.is_none());
        assert_eq!(state.cached_value_0c, 1.0);
    }

    /// IDA: N/A checked host guards around the original's unchecked 00712A90 index and type.
    #[test]
    fn failure_is_atomic() {
        let mut world = World::new();
        let table = make_table(&mut world);
        let floating = table.objects[0].entity;
        let boolean = table.objects[2].entity;
        // Object 3 is NiBoolData.
        let wrong_type: [u8; 8] = [0, 0, 0x20, 0x40, 3, 0, 0, 0];
        let out_of_range: [u8; 8] = [0, 0, 0x20, 0x40, 99, 0, 0, 0];
        // Object 1 is NiFloatData.
        let bool_wrong_type: [u8; 5] = [1, 1, 0, 0, 0];
        let unchanged = |world: &World| {
            let float_state = world.get::<FloatInterpolator>(floating).unwrap();
            let bool_state = world.get::<BoolInterpolator>(boolean).unwrap();
            assert_eq!(
                float_state.cached_value_0c,
                INVALID_FLOAT_INTERPOLATOR_VALUE
            );
            assert!(float_state.data_10.is_none());
            assert_eq!(bool_state.cached_value_0c, INVALID_BOOL_INTERPOLATOR_VALUE);
            assert!(bool_state.data_10.is_none());
        };
        assert!(
            load_float_interpolator_payload(&mut world, &table, 0, &wrong_type, 0x0A01_0071)
                .is_err()
        );
        unchanged(&world);
        assert!(
            load_float_interpolator_payload(&mut world, &table, 5, &wrong_type, 0x1400_0004)
                .is_err()
        );
        unchanged(&world);
        assert!(
            load_float_interpolator_payload(&mut world, &table, 2, &wrong_type, 0x1400_0004)
                .is_err()
        );
        unchanged(&world);
        assert!(
            load_float_interpolator_payload(&mut world, &table, 0, &wrong_type, 0x1400_0004)
                .is_err()
        );
        unchanged(&world);
        assert!(
            load_float_interpolator_payload(&mut world, &table, 0, &out_of_range, 0x1400_0004)
                .is_err()
        );
        unchanged(&world);
        assert!(
            load_bool_interpolator_payload(&mut world, &table, 2, &bool_wrong_type, 0x1400_0004)
                .is_err()
        );
        unchanged(&world);
        assert!(
            load_bool_timeline_interpolator_payload(
                &mut world,
                &table,
                2,
                &bool_wrong_type,
                0x1400_0004
            )
            .is_err()
        );
        unchanged(&world);
        for length in 0..8 {
            assert!(
                load_float_interpolator_payload(
                    &mut world,
                    &table,
                    0,
                    &out_of_range[..length],
                    0x1400_0004
                )
                .is_err()
            );
            unchanged(&world);
        }
        for length in 0..5 {
            assert!(
                load_bool_interpolator_payload(
                    &mut world,
                    &table,
                    2,
                    &bool_wrong_type[..length],
                    0x1400_0004
                )
                .is_err()
            );
            unchanged(&world);
        }
    }
}
