//! NiObjectNET: the name, sorted extra data and controller head every scene object shares, plus
//! NiAlphaProperty, the first property built on it.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};

/// IDA: the stream version below which the single-extra format applies.
pub const MODERN_OBJECT_NET_VERSION: u32 = 0x0A01_0072;

/// IDA: 006FFCE0 what the load queues for the link pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectNetPending {
    pub name: Option<Vec<u8>>,
    pub extra_indices: Vec<u32>,
    pub controller_index: u32,
    pub bytes_consumed: usize,
}

impl Default for ObjectNetPending {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            name: None,
            extra_indices: Vec::new(),
            controller_index: NULL_REFERENCE,
            bytes_consumed: 0,
        }
    }
}

/// IDA: NiObjectNET +0x08 name, +0x10 sorted extra data, +0x0C controller head.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct NiObjectNet {
    pub name: Option<Vec<u8>>,
    pub extra_data: Vec<Entity>,
    pub controller_0c: Option<Entity>,
}

/// IDA: NiExtraData's name, which 006FF8A0 may generate during a link.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct NiExtraDataName(pub Option<Vec<u8>>);

/// IDA: 00713620 a signed-length owned string: a nonpositive length is null.
fn read_name(reader: &mut ByteReader<'_>) -> Result<Option<Vec<u8>>, StreamError> {
    let length = reader.u32("truncated AlphaProperty u32")? as i32;
    if length <= 0 {
        return Ok(None);
    }
    Ok(Some(
        reader
            .take(length as usize, "truncated AlphaProperty name")?
            .to_vec(),
    ))
}

/// IDA: 006FFCE0 NiObjectNET_LoadBinary for version >= 0A010072: the name, the count and indices, and
/// the controller link. Completion: 100% of the modern form.
/// TODO: [ ] The older single-extra format.
pub fn parse_object_net_payload(
    bytes: &[u8],
    version: u32,
) -> Result<ObjectNetPending, StreamError> {
    if version < MODERN_OBJECT_NET_VERSION {
        return Err(StreamError("legacy NiObjectNET payload unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    let name = read_name(&mut reader)?;
    let count = reader.u32("truncated AlphaProperty u32")?;
    if count > u32::from(u16::MAX) {
        return Err(StreamError(
            "NiObjectNET extra count exceeds original field",
        ));
    }
    if reader.remaining() < 4 || count as usize > (reader.remaining() - 4) / 4 {
        return Err(StreamError("truncated NiObjectNET links"));
    }
    let mut extra_indices = Vec::with_capacity(count as usize);
    for _ in 0..count {
        extra_indices.push(reader.u32("truncated NiObjectNET links")?);
    }
    let controller_index = reader.u32("truncated NiObjectNET links")?;
    Ok(ObjectNetPending {
        name,
        extra_indices,
        controller_index,
        bytes_consumed: reader.position(),
    })
}

// IDA: N/A a helper, with no address of its own.
fn c_prefix(name: &[u8]) -> &[u8] {
    name.split(|&byte| byte == 0).next().unwrap_or(name)
}

/// IDA: N/A a preflight of 00700050's extra loop. Null and out-of-range are skipped here; the resolver
/// rejects out-of-range.
#[must_use]
pub fn object_net_extras_supported(
    world: &World,
    pending: &ObjectNetPending,
    table: &StreamObjectTable,
) -> bool {
    pending.extra_indices.iter().all(|&index| {
        table.objects.get(index as usize).is_none_or(|entry| {
            index == NULL_REFERENCE
                || world
                    .get::<NiClass>(entry.entity)
                    .is_some_and(|class| class.is_extra_data())
        })
    })
}

/// IDA: 006FF8A0's generated name: the RTTI prefix and `ED%03d`, over the retained count.
fn generated_name(class: NiClass, insertion_count: usize) -> Result<Vec<u8>, StreamError> {
    if insertion_count >= 1000 {
        return Err(StreamError("nameless extra exceeds bounded ED999 format"));
    }
    Ok(format!("{}ED{insertion_count:03}", class.extra_rtti_prefix()).into_bytes())
}

/// IDA: see the words here, which name the addresses this relates to.
/// The extra loop of 00700050: 006FF8A0 names a nameless extra, once per owner; 006FF570 inserts by
/// `strcmp`, dropping a later duplicate name. Names are published only once all links have resolved.
fn resolve_extras(
    world: &mut World,
    pending: &ObjectNetPending,
    table: &StreamObjectTable,
) -> Result<Vec<Entity>, StreamError> {
    let mut staged: Vec<(Entity, Vec<u8>)> = Vec::with_capacity(pending.extra_indices.len());
    let mut generated: Vec<(Entity, Vec<u8>)> = Vec::new();
    for &index in &pending.extra_indices {
        if index == NULL_REFERENCE {
            continue; // 007124A0 returns null; 006FF8A0 rejects it.
        }
        let entry = table
            .objects
            .get(index as usize)
            .ok_or(StreamError("NiObjectNET extra link index out of range"))?;
        let class = *world
            .get::<NiClass>(entry.entity)
            .filter(|class| class.is_extra_data())
            .ok_or(StreamError(
                "AlphaProperty extra link has unsupported owner type",
            ))?;
        let existing = world
            .get::<NiExtraDataName>(entry.entity)
            .and_then(|name| name.0.clone());
        let name = match existing {
            Some(name) => name,
            None => match generated.iter().find(|(owner, _)| *owner == entry.entity) {
                Some((_, name)) => name.clone(),
                None => {
                    let name = generated_name(class, staged.len())?;
                    generated.push((entry.entity, name.clone()));
                    name
                }
            },
        };
        let key = c_prefix(&name);
        let place = staged.partition_point(|(_, other)| c_prefix(other) < key);
        if staged
            .get(place)
            .is_none_or(|(_, other)| c_prefix(other) != key)
        {
            staged.insert(place, (entry.entity, name));
        }
        // Otherwise 006FF480 drops the new duplicate, retaining the first.
    }
    for (owner, name) in generated {
        world.entity_mut(owner).insert(NiExtraDataName(Some(name)));
    }
    Ok(staged.into_iter().map(|(owner, _)| owner).collect())
}

/// IDA: 00700050's name and extra publication without a controller head, which NiAVObject owners bind
/// themselves. Completion: 100% of the bound classes.
/// TODO: [ ] Original refcount timing and the remaining NiExtraData classes.
pub fn resolve_object_net_extra_links(
    world: &mut World,
    pending: &ObjectNetPending,
    table: &StreamObjectTable,
) -> Result<NiObjectNet, StreamError> {
    Ok(NiObjectNet {
        name: pending.name.clone(),
        extra_data: resolve_extras(world, pending, table)?,
        controller_0c: None,
    })
}

/// IDA: 00700050 LinkBinary, with a controller head when one is linked: the head resolves first, then the extras. The
/// heads are the classes the C++ port binds at its sites, over particle, node and texture owners; the original has no type
/// check, so a class bound for another owner's site is accepted here too. Completion: 100% of the bound classes.
pub fn resolve_object_net_links(
    world: &mut World,
    pending: &ObjectNetPending,
    table: &StreamObjectTable,
) -> Result<NiObjectNet, StreamError> {
    let mut controller_0c = None;
    if pending.controller_index != NULL_REFERENCE {
        let entry = table
            .objects
            .get(pending.controller_index as usize)
            .ok_or(StreamError("NiObjectNET controller index out of range"))?;
        if !world
            .get::<NiClass>(entry.entity)
            .is_some_and(|class| class.is_controller_head())
        {
            return Err(StreamError("NiObjectNET controller link unsupported"));
        }
        controller_0c = Some(entry.entity);
    }
    let mut net = resolve_object_net_extra_links(world, pending, table)?;
    net.controller_0c = controller_0c;
    Ok(net)
}

/// IDA: 00718630 NiAlphaProperty: +0x18 flags (cold 00EC), +0x1A test reference (cold 0).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlphaProperty {
    pub flags_18: u16,
    pub alpha_test_ref_1a: u8,
}

impl Default for AlphaProperty {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            flags_18: 0x00EC,
            alpha_test_ref_1a: 0,
        }
    }
}

/// IDA: what 00718730 queues.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlphaPropertyPending {
    pub net: ObjectNetPending,
    pub property: AlphaProperty,
    pub bytes_consumed: usize,
}

/// IDA: 006FFCE0 -> 00700AC0 -> 00718730, the modern payload. Completion: 100%.
pub fn parse_alpha_property_payload(
    bytes: &[u8],
    version: u32,
) -> Result<AlphaPropertyPending, StreamError> {
    let net = parse_object_net_payload(bytes, version)?;
    let mut reader = ByteReader::new(&bytes[net.bytes_consumed..]);
    let flags_18 = reader.u16("truncated AlphaProperty flags")?;
    let alpha_test_ref_1a = reader.u8("truncated AlphaProperty test reference")?;
    let bytes_consumed = net.bytes_consumed + reader.position();
    Ok(AlphaPropertyPending {
        net,
        property: AlphaProperty {
            flags_18,
            alpha_test_ref_1a,
        },
        bytes_consumed,
    })
}

/// IDA: the 00700050 link completing a parsed 00718730. A rejected link leaves `target` unchanged.
pub fn resolve_alpha_property_links(
    world: &mut World,
    target: Entity,
    pending: &AlphaPropertyPending,
    table: &StreamObjectTable,
) -> Result<(), StreamError> {
    let net = resolve_object_net_links(world, &pending.net, table)?;
    world.entity_mut(target).insert((net, pending.property));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_payload(name: Option<&str>, extras: &[u32]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let name = name.unwrap_or("");
        bytes.extend((name.len() as u32).to_le_bytes());
        bytes.extend(name.as_bytes());
        bytes.extend((extras.len() as u32).to_le_bytes());
        for index in extras {
            bytes.extend(index.to_le_bytes());
        }
        bytes.extend(NULL_REFERENCE.to_le_bytes());
        bytes.extend(0x1234u16.to_le_bytes());
        bytes.push(0x7F);
        bytes
    }

    fn extra(world: &mut World, class: NiClass, name: Option<&str>) -> Entity {
        world
            .spawn((
                class,
                NiExtraDataName(name.map(|text| text.as_bytes().to_vec())),
            ))
            .id()
    }

    /// Ported from `gamebryo_object_net`.
    #[test]
    fn payload_fields_and_counts() {
        assert_eq!(AlphaProperty::default().flags_18, 0x00EC);
        let mut bytes = alpha_payload(Some("Effect"), &[1, NULL_REFERENCE]);
        let size = bytes.len();
        bytes.push(0xA5);
        let parsed = parse_alpha_property_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(parsed.net.name.as_deref(), Some(&b"Effect"[..]));
        assert_eq!(parsed.net.extra_indices, [1, NULL_REFERENCE]);
        assert_eq!(parsed.net.controller_index, NULL_REFERENCE);
        assert_eq!(
            parsed.property,
            AlphaProperty {
                flags_18: 0x1234,
                alpha_test_ref_1a: 0x7F
            }
        );
        assert_eq!(parsed.bytes_consumed, size);
        let base = parse_object_net_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(base.bytes_consumed, size - 3);
        assert_eq!(base.extra_indices, parsed.net.extra_indices);
    }

    /// IDA: 00700050, 006FF8A0's generated names and 006FF570's sort and duplicate rule.
    #[test]
    fn extras_sort_generate_names_and_drop_duplicates() {
        let mut world = World::new();
        let string_data = extra(&mut world, NiClass::StringExtraData, Some("Zulu"));
        let binary_data = extra(&mut world, NiClass::BinaryExtraData, None);
        let text_data = extra(&mut world, NiClass::TextKeyExtraData, Some("Alpha"));
        let duplicate = extra(&mut world, NiClass::StringExtraData, Some("Alpha"));
        let mut table = StreamObjectTable::default();
        table.push("NiStringExtraData", 0x0071_6B20, string_data);
        table.push("AliasBinary", 0x0072_7F20, binary_data);
        table.push("NiTextKeyExtraData", 0x006D_7450, text_data);
        table.push("NiStringExtraData", 0x0071_6B20, duplicate);
        let property = world.spawn_empty().id();
        let pending = AlphaPropertyPending {
            net: ObjectNetPending {
                name: Some(b"GeometryAlpha".to_vec()),
                extra_indices: vec![0, 1, 2, 3, NULL_REFERENCE],
                ..ObjectNetPending::default()
            },
            property: AlphaProperty {
                flags_18: 0x0444,
                alpha_test_ref_1a: 17,
            },
            bytes_consumed: 0,
        };
        resolve_alpha_property_links(&mut world, property, &pending, &table).unwrap();
        assert_eq!(
            world
                .get::<NiExtraDataName>(binary_data)
                .unwrap()
                .0
                .as_deref(),
            Some(&b"NiBinaryED001"[..])
        );
        let net = world.get::<NiObjectNet>(property).unwrap();
        assert_eq!(net.extra_data, [text_data, binary_data, string_data]);
        assert_eq!(net.name.as_deref(), Some(&b"GeometryAlpha"[..]));
        assert_eq!(
            world
                .get::<AlphaProperty>(property)
                .unwrap()
                .alpha_test_ref_1a,
            17
        );
    }

    /// IDA: 006FF8A0 mutates one shared name; 006FF570 removes the repeat. An empty name is not null.
    #[test]
    fn repeated_owners_and_empty_names() {
        let mut world = World::new();
        let nameless = extra(&mut world, NiClass::StringExtraData, None);
        let mut table = StreamObjectTable::default();
        table.push("Alias", 0x0071_6B20, nameless);
        let pending = ObjectNetPending {
            extra_indices: vec![0, 0],
            ..ObjectNetPending::default()
        };
        let net = resolve_object_net_links(&mut world, &pending, &table).unwrap();
        assert_eq!(net.extra_data.len(), 1);
        assert_eq!(
            world.get::<NiExtraDataName>(nameless).unwrap().0.as_deref(),
            Some(&b"NiStringED000"[..])
        );

        let empty = extra(&mut world, NiClass::StringExtraData, Some(""));
        let mut table = StreamObjectTable::default();
        table.push("NiStringExtraData", 0x0071_6B20, empty);
        let pending = ObjectNetPending {
            extra_indices: vec![0],
            ..ObjectNetPending::default()
        };
        let net = resolve_object_net_links(&mut world, &pending, &table).unwrap();
        assert_eq!(net.extra_data, [empty]);
        assert_eq!(
            world.get::<NiExtraDataName>(empty).unwrap().0.as_deref(),
            Some(&b""[..])
        );
    }

    /// The head classes the C++ port binds at its 00700050 sites: particle, node and texture controllers.
    #[test]
    fn controller_heads() {
        let mut world = World::new();
        let mut table = StreamObjectTable::default();
        for class in [
            NiClass::ParticleUpdateController,
            NiClass::TransformController,
            NiClass::VisController,
            NiClass::MultiTargetTransformController,
            NiClass::ControllerManager,
            NiClass::FlipController,
            NiClass::TextureTransformController,
        ] {
            let head = world.spawn(class).id();
            table.push("Controller", 0, head);
            let pending = ObjectNetPending {
                controller_index: table.objects.len() as u32 - 1,
                ..ObjectNetPending::default()
            };
            let net = resolve_object_net_links(&mut world, &pending, &table).unwrap();
            assert_eq!(net.controller_0c, Some(head), "{class:?}");
        }
        let sequence = world.spawn(NiClass::ControllerSequence).id();
        table.push("NiControllerSequence", 0, sequence);
        let pending = ObjectNetPending {
            controller_index: table.objects.len() as u32 - 1,
            ..ObjectNetPending::default()
        };
        assert!(
            resolve_object_net_links(&mut world, &pending, &table).is_err(),
            "a sequence is not a NiTimeController"
        );
    }

    #[test]
    fn malformed_payloads_and_links_are_rejected() {
        let bytes = alpha_payload(None, &[0]);
        for length in 0..bytes.len() {
            assert!(
                parse_alpha_property_payload(&bytes[..length], 0x1400_0004).is_err(),
                "{length}"
            );
        }
        let mut excessive = bytes.clone();
        excessive[4..8].copy_from_slice(&0x1_0000u32.to_le_bytes());
        assert!(
            parse_alpha_property_payload(&excessive, 0x1400_0004).is_err(),
            "beyond the u16 field"
        );
        assert!(
            parse_alpha_property_payload(&bytes, 0x0A01_0071).is_err(),
            "legacy"
        );

        let mut world = World::new();
        let data = world.spawn(NiClass::FloatKeyData).id();
        let mut table = StreamObjectTable::default();
        table.push("NiFloatData", 0x006E_3440, data);
        let property = world.spawn(AlphaProperty::default()).id();
        let mut pending = AlphaPropertyPending {
            net: ObjectNetPending {
                extra_indices: vec![0],
                ..ObjectNetPending::default()
            },
            ..AlphaPropertyPending::default()
        };
        assert!(resolve_alpha_property_links(&mut world, property, &pending, &table).is_err());
        assert!(
            world.get::<NiObjectNet>(property).is_none(),
            "the wrong extra type changes nothing"
        );
        assert_eq!(
            world.get::<AlphaProperty>(property).unwrap().flags_18,
            0x00EC
        );
        pending.net.extra_indices.clear();
        pending.net.controller_index = 0;
        assert!(resolve_alpha_property_links(&mut world, property, &pending, &table).is_err());
        assert!(
            world.get::<NiObjectNet>(property).is_none(),
            "a non-controller link is rejected"
        );
    }
}
