//! NiObjectNET: the name, sorted extra data and controller head every scene object shares, plus
//! NiAlphaProperty, the first property built on it.

use bevy_ecs::prelude::*;
use oblivion_core::crt::c_prefix;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;

use super::controller_time::{controller_next, is_time_controller, set_next_controller};
use super::particle_controller::controller_time_mut;
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable, read_owned_c_string};

/// IDA: the stream version below which the single-extra format applies.
pub const MODERN_OBJECT_NET_VERSION: u32 = 0x0500_000B;

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

/// IDA: 006FFCE0 NiObjectNET_LoadBinary from 0500000B, its only gate: the name, the count and indices, and
/// the controller link. Completion: 100% of the >= 0500000B form.
/// TODO: [ ] The older single-extra format.
pub fn parse_object_net_payload(bytes: &[u8], version: u32) -> Result<ObjectNetPending, GameError> {
    if version < MODERN_OBJECT_NET_VERSION {
        return Err(GameError::unclassified(
            0,
            "legacy NiObjectNET payload unsupported",
        ));
    }
    let mut reader = ByteReader::new(bytes);
    let name = read_owned_c_string(
        &mut reader,
        "truncated AlphaProperty u32",
        "truncated AlphaProperty name",
    )?;
    let count = reader.u32("truncated AlphaProperty u32")?;
    if count > u32::from(u16::MAX) {
        return Err(GameError::unclassified(
            0,
            "NiObjectNET extra count exceeds original field",
        ));
    }
    if reader.remaining() < 4 || count as usize > (reader.remaining() - 4) / 4 {
        return Err(GameError::stream("truncated NiObjectNET links"));
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
fn generated_name(class: NiClass, insertion_count: usize) -> Result<Vec<u8>, GameError> {
    if insertion_count >= 1000 {
        return Err(GameError::unclassified(
            0,
            "nameless extra exceeds bounded ED999 format",
        ));
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
) -> Result<Vec<Entity>, GameError> {
    let mut staged: Vec<(Entity, Vec<u8>)> = Vec::with_capacity(pending.extra_indices.len());
    let mut generated: Vec<(Entity, Vec<u8>)> = Vec::new();
    for &index in &pending.extra_indices {
        if index == NULL_REFERENCE {
            continue; // 007124A0 returns null; 006FF8A0 rejects it.
        }
        let entry = table
            .objects
            .get(index as usize)
            .ok_or(GameError::unclassified(
                0,
                "NiObjectNET extra link index out of range",
            ))?;
        let class = *world
            .get::<NiClass>(entry.entity)
            .filter(|class| class.is_extra_data())
            .ok_or(GameError::unclassified(
                0,
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
) -> Result<NiObjectNet, GameError> {
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
) -> Result<NiObjectNet, GameError> {
    let mut controller_0c = None;
    if pending.controller_index != NULL_REFERENCE {
        let entry =
            table
                .objects
                .get(pending.controller_index as usize)
                .ok_or(GameError::unclassified(
                    0,
                    "NiObjectNET controller index out of range",
                ))?;
        if !world
            .get::<NiClass>(entry.entity)
            .is_some_and(|class| class.is_controller_head())
        {
            return Err(GameError::unclassified(
                0,
                "NiObjectNET controller link unsupported",
            ));
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

/// IDA: 006FFCE0 -> 00700AC0 -> 00718730, the modern payload. Completion: 100% from 0A010072.
/// TODO: [ ] Below it, 00700AC0's u16 into stream+302 and 00718730's older flag branch.
pub fn parse_alpha_property_payload(
    bytes: &[u8],
    version: u32,
) -> Result<AlphaPropertyPending, GameError> {
    if version < 0x0A01_0072 {
        return Err(GameError::unclassified(
            0,
            "legacy NiAlphaProperty payload unsupported",
        ));
    }
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
) -> Result<(), GameError> {
    let net = resolve_object_net_links(world, &pending.net, table)?;
    world.entity_mut(target).insert((net, pending.property));
    Ok(())
}

// IDA: 006FF420 NiObjectNET::SetName: FormHeap_Free(+0x08), then a strcpy_s copy of the source, or null for a null source.
// Completion: 100% of the host string owner.
pub fn set_object_name(name: &mut Option<Vec<u8>>, source: Option<&[u8]>) {
    *name = source.map(<[u8]>::to_vec);
}

// IDA: 006FFAC0 NiObjectNET_RemoveExtraDataByName(name): the sorted list's binary search, the entry removed (0070001A).
pub(crate) fn remove_extra_data(world: &mut World, object: Entity, name: &[u8]) {
    let extras = world
        .get::<NiObjectNet>(object)
        .map(|net| net.extra_data.clone())
        .unwrap_or_default();
    let found = extras.iter().position(|&extra| {
        world
            .get::<NiExtraDataName>(extra)
            .is_some_and(|extra_name| extra_name.0.as_deref() == Some(name))
    });
    if let Some(index) = found
        && let Some(mut net) = world.get_mut::<NiObjectNet>(object)
    {
        net.extra_data.remove(index);
    }
}

// IDA: 006FF9C0 NiObjectNET_GetExtraData(name): a null name is null. Under the 00B3F600 lock, a binary search of the +0x10 array by `strcmp` of each +0x08 name with 16-bit signed indexes: equal returns the extra, a name above takes `middle + 1` and one below `middle - 1`. Completion: 100%; a null extra name faults in the original, so it stops.
pub(crate) fn find_extra_data(
    world: &World,
    object: Entity,
    name: &[u8],
) -> Result<Option<Entity>, GameError> {
    let extras = &world
        .get::<NiObjectNet>(object)
        .ok_or(GameError::unclassified(
            0x006FF9C0,
            "006FF9C0: no NiObjectNET",
        ))?
        .extra_data;
    let mut high = extras.len() as i16 - 1;
    let mut low = 0_i16;
    while low <= high {
        let middle = ((i32::from(low) + i32::from(high)) >> 1) as i16;
        let extra = extras[middle as usize];
        let extra_name = world
            .get::<NiExtraDataName>(extra)
            .and_then(|name| name.0.as_deref())
            .ok_or(GameError::unclassified(
                0x006FFA21,
                "006FFA21: `strcmp` of a null extra data name",
            ))?;
        match name.cmp(extra_name) {
            std::cmp::Ordering::Equal => return Ok(Some(extra)),
            std::cmp::Ordering::Greater => low = middle + 1,
            std::cmp::Ordering::Less => high = middle - 1,
        }
    }
    Ok(None)
}

// IDA: a NiControllerManager's +0x7C NiDefaultAVObjectPalette vt+0x4C GetAVObject (006C5430); no palette answers null.
pub(crate) fn palette_object(world: &World, manager: Entity, name: &[u8]) -> Option<Entity> {
    world
        .get::<crate::gamebryo::controller_sequence::ControllerManager>(manager)
        .and_then(|manager| manager.object_palette_7c)
        .and_then(|palette| {
            world.get::<crate::gamebryo::av_object_palette::DefaultAvObjectPalette>(palette)
        })
        .and_then(|palette| palette.get_av_object(name))
}

/// IDA: 006F94A0 NiObjectNET_GetObjectByNameWithPalette(node, name): a NiControllerManager in the node's chain (00B3CAC0)
/// looks in its palette first; a miss, or no manager, is the node's GetObjectByName (vt+0x58). Completion: 100%.
pub fn object_by_name_with_palette(world: &World, node: Entity, name: &[u8]) -> Option<Entity> {
    get_controller(world, node, NiClass::ControllerManager)
        .and_then(|manager| palette_object(world, manager, name))
        .or_else(|| crate::gamebryo::scene_node::object_by_name(world, node, name))
}

// IDA: N/A the +0x0C controller head of an object; an entity with no NiObjectNET part has none.
#[must_use]
pub(crate) fn controller_head(world: &World, object: Entity) -> Option<Entity> {
    world
        .get::<NiObjectNet>(object)
        .and_then(|net| net.controller_0c)
}

// IDA: N/A the +0x0C store (00478300's pointer assign in the original).
pub(crate) fn set_controller_head(world: &mut World, object: Entity, head: Option<Entity>) {
    world
        .get_mut::<NiObjectNet>(object)
        .expect("a NiObjectNET owner")
        .controller_0c = head;
}

// IDA: 006FFE60 NiObjectNET_PrependController(controller): a null controller returns (006FFE67); controller->SetNext(head)
// (006C61E0), then +0x0C = controller.
// Completion: 100% of the order; the reference counts have no host form.
pub(crate) fn prepend_controller(world: &mut World, object: Entity, controller: Entity) {
    if !is_time_controller(world, Some(controller)) {
        return;
    }
    let head = controller_head(world, object);
    set_next_controller(world, controller, head);
    set_controller_head(world, object, Some(controller));
}

// IDA: 006FFE90 NiObjectNET_RemoveController(controller): a null controller (006FFEBA) or a null head (006FFEC8) returns. A
// head match stores +0x0C = the controller's +0x34 (006FFEEE); otherwise the walk from the head's +0x34 returns on a null
// link (006FFF43, 006FFF52), the controller not being in the chain, and the predecessor takes SetNext(the controller's
// +0x34) (006FFF74). In both the removed controller's +0x34 is then cleared.
// Completion: 100% of the order; the reference counts have no host form.
pub(crate) fn remove_controller(world: &mut World, object: Entity, controller: Entity) {
    if !is_time_controller(world, Some(controller))
        || !is_time_controller(world, controller_head(world, object))
    {
        return;
    }
    let removed_next = controller_next(world, controller);
    let clear = |world: &mut World| {
        let mut removed = controller_time_mut(world, controller).expect("checked above");
        removed.next_34 = None;
        removed.next_linked = false;
    };
    let head = controller_head(world, object).expect("checked above");
    if head == controller {
        set_controller_head(world, object, removed_next);
        clear(world);
        return;
    }
    let mut previous = head;
    loop {
        let next = controller_next(world, previous);
        if !is_time_controller(world, next) {
            return;
        }
        if next == Some(controller) {
            break;
        }
        previous = next.expect("checked above");
    }
    set_next_controller(world, previous, removed_next);
    clear(world);
}

// IDA: 00700010 NiObjectNET_GetController(rtti): the first entry from +0x0C by +0x34 whose RTTI chain (vtable +0x04, then
// each +0x04 parent) holds rtti. The host matches the class tag: of the classes asked for (NiControllerManager 00B3CAC0,
// NiTransformController B3CA58, NiMultiTargetTransformController B3CD7C, NiGeomMorpherController 00B3CA90) none has a bound
// subclass.
// Completion: 100% of the bound classes.
#[must_use]
pub fn get_controller(world: &World, object: Entity, class: NiClass) -> Option<Entity> {
    let mut current = controller_head(world, object);
    while let Some(entry) = current {
        if world.get::<NiClass>(entry) == Some(&class) {
            return Some(entry);
        }
        current = controller_next(world, entry);
    }
    None
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
