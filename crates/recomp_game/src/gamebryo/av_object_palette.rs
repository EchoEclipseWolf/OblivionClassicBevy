//! NiDefaultAVObjectPalette: names to NiAVObjects, in an explicit 37-bucket string map.

use bevy_ecs::prelude::*;
use recomp_core::crt::stricmp;
use recomp_core::hash::ni_string_hash;
use recomp_core::reader::{ByteReader, StreamError};

use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};

const TRUNCATED: &str = "truncated NiDefaultAVObjectPalette payload";

/// One 007166B0 entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvObjectPaletteEntryPayload {
    pub name: Option<Vec<u8>>,
    pub object_index: u32,
}

/// IDA: 007166B0's stream fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvObjectPalettePayload {
    pub scene_index: u32,
    pub entries: Vec<AvObjectPaletteEntryPayload>,
    pub bytes_consumed: usize,
}

/// IDA: 007166B0 modern LoadBinary after the empty NiObject prefix: immediate scene reference, u32 count, then per entry a
/// 00713620 owned C-string (signed length <= 0 -> null) and an immediate object reference. Each entry needs at least a
/// length word and a reference. Completion: 100% of the field order and widths.
/// TODO: [ ] The original's FormHeap string allocation failures.
pub fn parse_av_object_palette_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<AvObjectPalettePayload, StreamError> {
    if stream_version < 0x0A01_0072 {
        return Err(StreamError(
            "legacy NiDefaultAVObjectPalette payload unsupported",
        ));
    }
    let mut reader = ByteReader::new(bytes);
    let scene_index = reader.u32(TRUNCATED)?;
    let count = reader.u32(TRUNCATED)?;
    if count as usize > reader.remaining() / 8 {
        return Err(StreamError("truncated NiDefaultAVObjectPalette entries"));
    }
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let length = reader.u32(TRUNCATED)? as i32;
        let name = if length <= 0 {
            None
        } else {
            Some(
                reader
                    .take(length as usize, "truncated NiDefaultAVObjectPalette name")?
                    .to_vec(),
            )
        };
        let object_index = reader.u32(TRUNCATED)?;
        entries.push(AvObjectPaletteEntryPayload { name, object_index });
    }
    Ok(AvObjectPalettePayload {
        scene_index,
        entries,
        bytes_consumed: reader.position(),
    })
}

/// IDA: the map key is the NUL-terminated C string, so an embedded NUL truncates it. Completion: 100%.
fn c_string_key(name: &[u8]) -> &[u8] {
    name.iter()
        .position(|&byte| byte == 0)
        .map_or(name, |end| &name[..end])
}

/// A map node: the key is copied (+0x18 flag) and the value is a raw NiAVObject*, with no refcount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvObjectPaletteEntry {
    pub key: Vec<u8>,
    pub value: Entity,
}

/// IDA: 00716A40's factory, 0x20 bytes via 006C5360: NiTStringPointerMap<NiAVObject*> at +0x08 with 37 buckets (+0x0C),
/// bucket array +0x10, count +0x14 = 0, copy-keys +0x18 = 1, scene +0x1C = factory argument 0 (00716690's rebuild skipped
/// for null). Each bucket lists its entries head to tail. Completion: 100% of the cold state and the map semantics.
/// TODO: [ ] 00716690 / 007165B0's scene rebuild and 00716460 ProcessClone.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct DefaultAvObjectPalette {
    buckets_10: [Vec<AvObjectPaletteEntry>; Self::BUCKET_COUNT],
    count_14: usize,
    scene_1c: Option<Entity>,
}

impl Default for DefaultAvObjectPalette {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            buckets_10: std::array::from_fn(|_| Vec::new()),
            count_14: 0,
            scene_1c: None,
        }
    }
}

impl DefaultAvObjectPalette {
    pub const BUCKET_COUNT: usize = 37;

    /// IDA: 007DAED0 h = 33*h + signed byte, without case folding, modulo 37 here. Completion: 100%.
    fn bucket(key: &[u8]) -> usize {
        ni_string_hash(key) as usize % Self::BUCKET_COUNT
    }

    /// IDA: 006C5420 SetScene (vtable +0x54) stores +0x1C only, with no rebuild. Completion: 100%.
    pub const fn set_scene(&mut self, scene: Option<Entity>) {
        self.scene_1c = scene;
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn scene(&self) -> Option<Entity> {
        self.scene_1c
    }

    /// IDA: 006C5460 SetAVObject (vtable +0x50): nonnull -> 00412D30 insert or assign (an existing key keeps its stored
    /// spelling, new nodes go to the bucket head); null -> 004524C0 remove by key. The compare is 00584D10's `_stricmp`,
    /// inside the bucket that the case-sensitive hash chose. Completion: 100% of the hash, compare and order.
    pub fn set_av_object(&mut self, name: &[u8], value: Option<Entity>) {
        let key = c_string_key(name);
        let bucket = &mut self.buckets_10[Self::bucket(key)];
        if let Some(at) = bucket
            .iter()
            .position(|entry| stricmp(&entry.key, key) == 0)
        {
            if let Some(value) = value {
                bucket[at].value = value;
            } else {
                bucket.remove(at);
                self.count_14 -= 1;
            }
            return;
        }
        let Some(value) = value else {
            return;
        };
        bucket.insert(
            0,
            AvObjectPaletteEntry {
                key: key.to_vec(),
                value,
            },
        );
        self.count_14 += 1;
    }

    /// IDA: 006C5430 GetAVObject (vtable +0x4C): a bucket-scoped lookup. Completion: 100%.
    #[must_use]
    pub fn get_av_object(&self, name: &[u8]) -> Option<Entity> {
        let key = c_string_key(name);
        self.buckets_10[Self::bucket(key)]
            .iter()
            .find(|entry| stricmp(&entry.key, key) == 0)
            .map(|entry| entry.value)
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count_14
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count_14 == 0
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn buckets(&self) -> &[Vec<AvObjectPaletteEntry>; Self::BUCKET_COUNT] {
        &self.buckets_10
    }
}

/// The outcome of 007166B0's immediate references.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvObjectPaletteLinkStatus {
    Linked,
    /// The C++ graph's `unsupported_link_target`.
    UnsupportedTarget,
}

/// IDA: 007166B0's raw NiAVObject* references, typed to the families the C++ graph binds: NiNode, NiBillboardNode,
/// NiTriShape, NiTriStrips, NiParticleSystem and NiDirectionalLight. NiCamera is not among them.
/// TODO: [ ] Further NiAVObject classes as they are ported.
fn palette_target(world: &World, entity: Entity) -> bool {
    matches!(
        world.get::<NiClass>(entity),
        Some(
            NiClass::Node
                | NiClass::BillboardNode
                | NiClass::TriShape
                | NiClass::TriStrips
                | NiClass::ParticleSystem
                | NiClass::DirectionalLight
        )
    )
}

/// IDA: 007166B0's immediate references as the stream graph runs them: +0x1C scene directly (no 00716690 rebuild), then each
/// entry through 006C5460 only when both name and object are nonnull. The entries go on a copy, so an unsupported target
/// publishes nothing. Completion: 100% of the order.
pub fn apply_av_object_palette_payload(
    world: &mut World,
    owner: Entity,
    payload: &AvObjectPalettePayload,
    table: &StreamObjectTable,
) -> Result<AvObjectPaletteLinkStatus, StreamError> {
    let mut staged = world
        .get::<DefaultAvObjectPalette>(owner)
        .ok_or(StreamError("NiDefaultAVObjectPalette owner mismatch"))?
        .clone();
    let mut scene = None;
    if payload.scene_index != NULL_REFERENCE {
        let entry = table
            .objects
            .get(payload.scene_index as usize)
            .ok_or(StreamError("AV object palette scene out of range"))?;
        if !palette_target(world, entry.entity) {
            return Ok(AvObjectPaletteLinkStatus::UnsupportedTarget);
        }
        scene = Some(entry.entity);
    }
    staged.set_scene(scene);
    for entry in &payload.entries {
        let Some(name) = &entry.name else {
            continue;
        };
        if entry.object_index == NULL_REFERENCE {
            continue;
        }
        let object = table
            .objects
            .get(entry.object_index as usize)
            .ok_or(StreamError("AV object palette entry out of range"))?;
        if !palette_target(world, object.entity) {
            return Ok(AvObjectPaletteLinkStatus::UnsupportedTarget);
        }
        staged.set_av_object(name, Some(object.entity));
    }
    *world
        .get_mut::<DefaultAvObjectPalette>(owner)
        .expect("read above") = staged;
    Ok(AvObjectPaletteLinkStatus::Linked)
}

/// IDA: 00716690 NiDefaultAVObjectPalette_RebuildFromScene: a nonnull +0x1C scene empties the map (NiTMapBase_RemoveAll) and
/// walks it (007165B0): each object with a +0x08 name goes in through vt+0x50 SetAVObject, then a node's (vt+8) children in
/// slot order, null slots included (they return at once). Completion: 100%.
pub fn rebuild_from_scene(world: &mut World, palette: Entity) {
    let Some(scene) = world
        .get::<DefaultAvObjectPalette>(palette)
        .expect("a NiDefaultAVObjectPalette")
        .scene_1c
    else {
        return;
    };
    let mut entries = Vec::new();
    collect_scene(world, Some(scene), &mut entries);
    let mut state = world
        .get_mut::<DefaultAvObjectPalette>(palette)
        .expect("a NiDefaultAVObjectPalette");
    state.buckets_10 = std::array::from_fn(|_| Vec::new());
    state.count_14 = 0;
    for (name, object) in entries {
        state.set_av_object(&name, Some(object));
    }
}

// IDA: 007165B0(object, palette), recursive: the SetAVObject calls in walk order.
fn collect_scene(world: &World, object: Option<Entity>, out: &mut Vec<(Vec<u8>, Entity)>) {
    let Some(object) = object else {
        return;
    };
    if let Some(name) = world
        .get::<super::object_net::NiObjectNet>(object)
        .and_then(|net| net.name.clone())
    {
        out.push((name, object));
    }
    if let Some(children) = world.get::<super::scene_node::NiNodeChildren>(object) {
        for &child in &children.slots_ac {
            collect_scene(world, child, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODERN: u32 = 0x1400_0004;

    fn dword(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }

    fn name(bytes: &mut Vec<u8>, text: &str) {
        dword(bytes, text.len() as u32);
        bytes.extend(text.as_bytes());
    }

    /// Ported from `gamebryo_av_object_palette` `verify_payload`.
    #[test]
    fn payload() {
        let mut bytes = Vec::new();
        dword(&mut bytes, 3);
        dword(&mut bytes, 2);
        name(&mut bytes, "Root");
        dword(&mut bytes, 0);
        dword(&mut bytes, 0); // null name
        dword(&mut bytes, 1);
        bytes.push(0xA5);
        let parsed = parse_av_object_palette_payload(&bytes, MODERN).unwrap();
        assert_eq!(parsed.scene_index, 3);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].name.as_deref(), Some(&b"Root"[..]));
        assert_eq!(parsed.entries[0].object_index, 0);
        assert!(parsed.entries[1].name.is_none());
        assert_eq!(parsed.entries[1].object_index, 1);
        assert_eq!(parsed.bytes_consumed, bytes.len() - 1);
        assert!(
            parse_av_object_palette_payload(&bytes[..14], MODERN).is_err(),
            "truncated palette entries were accepted"
        );
        assert!(
            parse_av_object_palette_payload(&bytes, 0x0A01_0071).is_err(),
            "legacy palette payload was accepted"
        );
    }

    /// Ported from `gamebryo_av_object_palette` `verify_map_semantics`.
    #[test]
    fn map_semantics() {
        let mut world = World::new();
        let mut palette = DefaultAvObjectPalette::default();
        assert_eq!(palette.len(), 0);
        assert!(palette.scene().is_none());
        let first = world.spawn_empty().id();
        let second = world.spawn_empty().id();
        let third = world.spawn_empty().id();
        palette.set_scene(Some(first));
        assert_eq!(palette.scene(), Some(first), "SetScene did not store +0x1C");

        // The hash is case-sensitive but the compare is not: "bloCK" shares "Block"'s bucket (1) and matches, while
        // "block" hashes elsewhere.
        palette.set_av_object(b"Block", Some(first));
        assert_eq!(palette.get_av_object(b"bloCK"), Some(first));
        assert!(palette.get_av_object(b"block").is_none());
        assert_eq!(palette.len(), 1);
        palette.set_av_object(b"bloCK", Some(second));
        assert_eq!(palette.get_av_object(b"Block"), Some(second));
        assert_eq!(palette.len(), 1);
        assert_eq!(palette.buckets()[1].len(), 1);
        assert_eq!(palette.buckets()[1][0].key, b"Block");
        palette.set_av_object(b"Bip01", Some(first));

        // "ar" hashes to Bip01's bucket (22) and is inserted at its head.
        palette.set_av_object(b"ar", Some(third));
        assert_eq!(palette.len(), 3);
        assert_eq!(palette.buckets()[22].len(), 2);
        assert_eq!(palette.buckets()[22][0].key, b"ar");
        assert_eq!(palette.buckets()[22][1].key, b"Bip01");

        palette.set_av_object(b"Node\0tail", Some(first));
        assert_eq!(palette.get_av_object(b"Node"), Some(first));
        assert!(palette.get_av_object(b"Nod").is_none());

        palette.set_av_object(b"ar", None);
        palette.set_av_object(b"missing", None);
        assert_eq!(palette.len(), 3);
        assert!(palette.get_av_object(b"ar").is_none());
        assert_eq!(palette.buckets()[22].len(), 1);
    }

    /// Beyond the C++ test: the graph's immediate references, skipping null names and null objects.
    #[test]
    fn immediate_references_publish_or_not_at_all() {
        let mut world = World::new();
        let node = world.spawn(NiClass::Node).id();
        let strips = world.spawn(NiClass::TriStrips).id();
        let camera = world.spawn(NiClass::Camera).id();
        let mut table = StreamObjectTable::default();
        table.push("NiNode", 0, node);
        table.push("NiTriStrips", 0, strips);
        table.push("NiCamera", 0, camera);
        let entry = |text: Option<&[u8]>, object_index| AvObjectPaletteEntryPayload {
            name: text.map(<[u8]>::to_vec),
            object_index,
        };
        let owner = world.spawn(DefaultAvObjectPalette::default()).id();
        let payload = AvObjectPalettePayload {
            scene_index: 0,
            entries: vec![
                entry(Some(b"Mesh"), 1),
                entry(None, 0),
                entry(Some(b"Absent"), NULL_REFERENCE),
            ],
            bytes_consumed: 0,
        };
        assert_eq!(
            apply_av_object_palette_payload(&mut world, owner, &payload, &table).unwrap(),
            AvObjectPaletteLinkStatus::Linked
        );
        let palette = world.get::<DefaultAvObjectPalette>(owner).unwrap();
        assert_eq!(palette.scene(), Some(node));
        assert_eq!(palette.len(), 1);
        assert_eq!(palette.get_av_object(b"mesh"), None);
        assert_eq!(palette.get_av_object(b"Mesh"), Some(strips));

        let fresh = world.spawn(DefaultAvObjectPalette::default()).id();
        let refused = AvObjectPalettePayload {
            entries: vec![entry(Some(b"Mesh"), 1), entry(Some(b"Eye"), 2)],
            ..payload.clone()
        };
        assert_eq!(
            apply_av_object_palette_payload(&mut world, fresh, &refused, &table).unwrap(),
            AvObjectPaletteLinkStatus::UnsupportedTarget
        );
        let palette = world.get::<DefaultAvObjectPalette>(fresh).unwrap();
        assert!(palette.is_empty() && palette.scene().is_none());
        let far = AvObjectPalettePayload {
            scene_index: 9,
            ..payload
        };
        assert!(apply_av_object_palette_payload(&mut world, fresh, &far, &table).is_err());
    }
}
