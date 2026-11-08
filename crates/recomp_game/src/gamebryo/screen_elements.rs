//! NiScreenElements (0xC0, vtable A7E29C; 00709D10 -> NiTriShape 00717570) over NiScreenElementsData (0x70, vtable A7D94C;
//! 00703230): the value state shared by the 05-D-4 DebugShader quad (007FA580) and the 05-E-3 image-space fallback quad
//! (00406950 at 0040715C..004073B3, 00B333EC). Neither is drawn here; D3D9 stops at this state.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::av_object::{NiAvParent, NiAvProperties};
use super::mesh_geometry::NiGeometry;
use super::object_net::NiObjectNet;
use super::properties::{VertexColorProperty, ZBufferProperty, new_material_property};
use super::scene_node::NiAvWorld;
use super::scene_state::{ScenePropertyDefaults, refresh_property_state};
use super::stream_objects::NiClass;
use super::texture::{TexturingMap, TexturingProperty};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScreenPolygon {
    pub vertex_count: u16,
    pub first_vertex: u16,
    pub triangle_count: u16,
    pub first_index: u16,
}

/// IDA: NiScreenElementsData's host value state, not the original layout; the constructor chain is 00709D10 -> 00717570
/// NiTriShape -> 00722690 -> 007227B0, with the data 00703230 over 0071FBB0. Normals and vertex colors are null (the 0/0
/// constructor arguments). The AV state is `NiAvProperties` and `NiAvWorld` on the same entity.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ScreenElementsQuad {
    pub polygon_capacity: u16,
    pub polygon_grow: u16,
    pub polygon_count: u16,
    pub polygons: Vec<ScreenPolygon>,
    /// 0xFFFF is free.
    pub polygon_index: Vec<u16>,
    pub vertex_capacity: u16,
    pub vertex_grow: u16,
    pub vertex_count: u16,
    pub vertices: Vec<[f32; 3]>,
    /// flags & 0x3F.
    pub texture_sets: u8,
    pub texture_coordinates: Vec<[f32; 2]>,
    pub index_capacity: u16,
    pub index_grow: u16,
    pub triangle_count: u16,
    pub index_count: u16,
    pub indices: Vec<u16>,
    pub bound_dirty: u8,
    /// 00728770 = 0.
    pub dirty_flags: u16,
    pub bound_center: [f32; 3],
    pub bound_radius: f32,
}

/// IDA: 00703230(hasNormals 0, hasColors 0, textureSets 1, polygons 1/1, vertices 4/1, triangles 2/1): capacities clamp to
/// 1, the polygon index fills 0xFF, index capacity 3 * triangles (grow 3 * 1), counts 0, bound clean. Completion: 100% for
/// these arguments; both callers pass them.
pub fn construct_screen_elements_data(quad: &mut ScreenElementsQuad) {
    quad.polygon_capacity = 1;
    quad.polygon_grow = 1;
    quad.polygon_count = 0;
    quad.polygons = vec![ScreenPolygon::default(); 1];
    quad.polygon_index = vec![0xFFFF; 1];
    quad.vertex_capacity = 4;
    quad.vertex_grow = 1;
    quad.vertex_count = 0;
    quad.vertices = vec![[0.0; 3]; 4];
    quad.texture_sets = 1;
    quad.texture_coordinates = vec![[0.0; 2]; 4];
    quad.index_capacity = 6;
    quad.index_grow = 3;
    quad.triangle_count = 0;
    quad.index_count = 0;
    quad.indices = vec![0; 6];
    quad.bound_dirty = 0;
}

/// IDA: 00702970 InsertPolygon(vertexCount, 0, null) for a polygon that fits the current capacities: fewer than 3 vertices
/// give -1; the triangle count is vertexCount - 2; the polygon takes the next vertex and index ranges and the first free
/// index slot; the fan indices are (first, first + i + 1, first + i + 2). Completion: 100% for the reached path.
/// TODO: [ ] The polygon, vertex and index growth branches, with a caller that inserts past the constructor capacities;
/// past them this indexes out of bounds where the original grows.
pub fn insert_screen_polygon(quad: &mut ScreenElementsQuad, vertex_count: u16) -> i32 {
    if vertex_count < 3 {
        return -1;
    }
    let triangles = vertex_count - 2;
    let polygon = quad.polygon_count;
    quad.polygon_count = quad.polygon_count.wrapping_add(1);
    let entry = ScreenPolygon {
        vertex_count,
        first_vertex: quad.vertex_count,
        triangle_count: triangles,
        first_index: quad.triangle_count.wrapping_mul(3),
    };
    quad.polygons[usize::from(polygon)] = entry;
    let mut result = i32::from(polygon);
    for slot in 0..usize::from(quad.polygon_count) {
        if quad.polygon_index[slot] == 0xFFFF {
            quad.polygon_index[slot] = polygon;
            result = slot as i32;
            break;
        }
    }
    quad.vertex_count = quad.vertex_count.wrapping_add(vertex_count);
    quad.triangle_count = quad.triangle_count.wrapping_add(triangles);
    quad.index_count = quad.index_count.wrapping_add(triangles.wrapping_mul(3));
    let mut cursor = usize::from(entry.first_index);
    for triangle in 0..triangles {
        quad.indices[cursor] = entry.first_vertex;
        quad.indices[cursor + 1] = triangle.wrapping_add(entry.first_vertex).wrapping_add(1);
        quad.indices[cursor + 2] = triangle.wrapping_add(entry.first_vertex).wrapping_add(2);
        cursor += 3;
    }
    result
}

/// IDA: N/A a helper, with no address of its own.
/// The polygon behind a public index, when it is a live 4-vertex one.
fn rectangle_polygon(quad: &ScreenElementsQuad, polygon: i32) -> Option<ScreenPolygon> {
    if polygon < 0 || polygon >= i32::from(quad.polygon_capacity) {
        return None;
    }
    let index = quad.polygon_index[polygon as usize];
    if index == 0xFFFF {
        return None;
    }
    let entry = quad.polygons[usize::from(index)];
    (entry.vertex_count == 4).then_some(entry)
}

/// IDA: 00702EC0 SetRectangle(polygon, x, y, width, height): a 4-vertex polygon gets (x, y), (x, y + h), (x + w, y + h),
/// (x + w, y), z 0; dirty flags |= 1, the bound marked dirty. Completion: 100%.
pub fn set_screen_rectangle(
    quad: &mut ScreenElementsQuad,
    polygon: i32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> bool {
    let Some(entry) = rectangle_polygon(quad, polygon) else {
        return false;
    };
    let first = usize::from(entry.first_vertex);
    let right = x + width;
    let bottom = y + height;
    quad.vertices[first] = [x, y, 0.0];
    quad.vertices[first + 1] = [x, bottom, 0.0];
    quad.vertices[first + 2] = [right, bottom, 0.0];
    quad.vertices[first + 3] = [right, y, 0.0];
    quad.dirty_flags |= 1;
    quad.bound_dirty = 1;
    true
}

/// IDA: 0072A0F0 NiBound::ComputeFromVertices: the axis-aligned box centre and the square root of the largest squared
/// distance (y*y + x*x + z*z, float steps); no vertices give the zero point and radius 0. Completion: 100%.
#[must_use]
pub fn compute_bound_from_vertices(vertices: &[[f32; 3]], count: usize) -> ([f32; 3], f32) {
    if count == 0 {
        return ([0.0; 3], 0.0);
    }
    let mut low = vertices[0];
    let mut high = vertices[0];
    for vertex in &vertices[1..count] {
        for axis in 0..3 {
            let value = vertex[axis];
            if low[axis] > value {
                low[axis] = value;
            }
            if high[axis] < value {
                high[axis] = value;
            }
        }
    }
    let center = [
        (high[0] + low[0]) * 0.5,
        (high[1] + low[1]) * 0.5,
        0.5 * (high[2] + low[2]),
    ];
    let mut largest = 0.0f32;
    for vertex in &vertices[..count] {
        let dx = vertex[0] - center[0];
        let dy = vertex[1] - center[1];
        let dz = vertex[2] - center[2];
        let squared = dy * dy + dx * dx + dz * dz;
        if largest < squared {
            largest = squared;
        }
    }
    (center, largest.sqrt())
}

/// IDA: 00703050 UpdateBound: clean, then 0072A0F0 when there are vertices. Completion: 100%.
pub fn update_screen_bound(quad: &mut ScreenElementsQuad) {
    quad.bound_dirty = 0;
    if quad.vertex_count != 0 {
        let (center, radius) =
            compute_bound_from_vertices(&quad.vertices, usize::from(quad.vertex_count));
        quad.bound_center = center;
        quad.bound_radius = radius;
    }
}

/// IDA: 00702FC0 SetRectangleUV(polygon, set, u0, v0, u1, v1): with texture coordinates, a valid 4-vertex polygon and set
/// below the flag count, the set (007282F0) gets (u0, v0), (u0, v1), (u1, v1), (u1, v0). Completion: 100%.
pub fn set_screen_rectangle_uv(
    quad: &mut ScreenElementsQuad,
    polygon: i32,
    set: u16,
    u0: f32,
    v0: f32,
    u1: f32,
    v1: f32,
) -> bool {
    if quad.texture_coordinates.is_empty() {
        return false;
    }
    let Some(entry) = rectangle_polygon(quad, polygon) else {
        return false;
    };
    if set >= u16::from(quad.texture_sets) {
        return false;
    }
    let first = usize::from(set) * usize::from(quad.vertex_count) + usize::from(entry.first_vertex);
    quad.texture_coordinates[first] = [u0, v0];
    quad.texture_coordinates[first + 1] = [u0, v1];
    quad.texture_coordinates[first + 2] = [u1, v1];
    quad.texture_coordinates[first + 3] = [u1, v0];
    true
}

/// IDA: 00405680 NiAVObject_AttachProperty: property type < 10 -> AddHead on +0x98; the refcount bump and drop around it
/// leave the count at +1. Every class bound here is below 10. Completion: 100%.
pub fn attach_screen_property(properties: &mut NiAvProperties, property: Entity) {
    properties.0.insert(0, Some(property));
}

/// IDA: 00707370 Update(0.0, 1) on an uncontrolled root (vt+0x60 00707580): identity world transform, world bound = model
/// bound. Completion: 100% of the reached state; the screen elements are not drawn here.
pub fn update_screen_elements_world(world: &mut World, quad: Entity) {
    let owner = world
        .get::<ScreenElementsQuad>(quad)
        .expect("a screen element has its data");
    let bound = [
        owner.bound_center[0],
        owner.bound_center[1],
        owner.bound_center[2],
        owner.bound_radius,
    ];
    world
        .get_mut::<NiAvWorld>(quad)
        .expect("a screen element has world state")
        .bound = bound;
}

/// IDA: N/A a finished NiScreenElements, with the AV components its update passes read.
pub fn spawn_screen_elements(
    world: &mut World,
    quad: ScreenElementsQuad,
    properties: NiAvProperties,
) -> Entity {
    world
        .spawn((
            NiClass::ScreenElements,
            quad,
            properties,
            NiGeometry::default(),
            NiAvWorld::default(),
            NiObjectNet::default(),
            NiAvParent::default(),
        ))
        .id()
}

/// IDA: N/A a property object with its NiObjectNET base, as the factory's `spawn_property` makes one.
pub(super) fn spawn_screen_property(
    world: &mut World,
    class: NiClass,
    property: impl Bundle,
) -> Entity {
    world.spawn((class, NiObjectNet::default(), property)).id()
}

/// IDA: 0040715C..004073B3 inside 00406950 (00B333EC null and g_shaderImageSpaceEffects 0): FormHeap(0xC0) NiScreenElements
/// over FormHeap(0x70) 00703230(0, 0, 1, 1, 1, 4, 1, 2, 1); InsertPolygon(4, 0, null), SetRectangle(0, 0, 0, 1, 1),
/// UpdateBound, SetRectangleUV(0, 0, 0, 0, 1, 1); NiVertexColorProperty (flags 8 & 0xFFC7), NiMaterialProperty 007095A0
/// (emissive from 00B25AC4..CC = 1.0, +0x54 incremented between the y and z stores), NiZBufferProperty (flags 15 & 0xFFFC),
/// NiTexturingProperty 007043B0 with 004057B0 base texture = 00405A30 (null at startup), 00405900 SetBaseFilterMode(0) and
/// 00405870 SetBaseClampMode(0), each attached in that order; 00707610 and 00707370(0.0, 1). Returns the quad for 00B333EC.
/// `defaults` is `NiProcessDefaults::scene_property_defaults`. Completion: 100%; not reached at startup (image-space
/// effects on).
pub fn build_image_space_fallback_quad(
    world: &mut World,
    defaults: &ScenePropertyDefaults,
    base_texture: Option<Entity>,
) -> Result<Entity, StreamError> {
    // 0040715C..004071D9.
    let mut quad = ScreenElementsQuad::default();
    construct_screen_elements_data(&mut quad);
    // 004071DE..00407253.
    insert_screen_polygon(&mut quad, 4);
    set_screen_rectangle(&mut quad, 0, 0.0, 0.0, 1.0, 1.0);
    update_screen_bound(&mut quad);
    set_screen_rectangle_uv(&mut quad, 0, 0, 0.0, 0.0, 1.0, 1.0);
    let mut properties = NiAvProperties::default();
    // 0040725A..0040729C.
    let vertex_color = VertexColorProperty {
        flags_18: VertexColorProperty::default().flags_18 & 0xFFC7,
    };
    let vertex_color = spawn_screen_property(world, NiClass::VertexColorProperty, vertex_color);
    attach_screen_property(&mut properties, vertex_color);
    // 004072A3..004072F0.
    let mut material = new_material_property(world);
    material.values.emissive[0] = 1.0;
    material.values.emissive[1] = 1.0;
    material.raw_state_54 += 1;
    material.values.emissive[2] = 1.0;
    let material = spawn_screen_property(world, NiClass::MaterialProperty, material);
    attach_screen_property(&mut properties, material);
    // 004072F7..00407339.
    let zbuffer = ZBufferProperty {
        flags_18: ZBufferProperty::default().flags_18 & 0xFFFC,
    };
    let zbuffer = spawn_screen_property(world, NiClass::ZBufferProperty, zbuffer);
    attach_screen_property(&mut properties, zbuffer);
    // 00407340..00407395: the cold seven-slot array, flags 4; 004057B0 creates the base map (00704100, flags 0x3100)
    // through 0042BB90 AddAtIndex(0) and assigns the texture; 00405900(0): (0 << 8) | (flags & 0xF0FF); 00405870(0):
    // (0 << 12) | (flags & 0xCFFF).
    let mut texturing = TexturingProperty::default();
    let mut base = TexturingMap {
        texture_08: base_texture,
        ..TexturingMap::default()
    };
    base.flags_04 &= 0xF0FF;
    base.flags_04 &= 0xCFFF;
    texturing.maps_1c[0] = Some(base);
    let texturing = spawn_screen_property(world, NiClass::TexturingProperty, texturing);
    attach_screen_property(&mut properties, texturing);

    let entity = spawn_screen_elements(world, quad, properties);
    // 004073A0, then 004073B3.
    refresh_property_state(world, entity, defaults)?;
    update_screen_elements_world(world, entity);
    Ok(entity)
}

#[cfg(test)]
mod tests {
    use super::super::properties::MaterialProperty;
    use super::super::scene_state::ScenePropertyState;
    use super::*;

    /// No C++ unit test covers this directly (phase 05 reaches it through its startup block): the reached quad path.
    #[test]
    fn rectangle_quad_values() {
        let mut quad = ScreenElementsQuad::default();
        construct_screen_elements_data(&mut quad);
        assert_eq!(insert_screen_polygon(&mut quad, 2), -1);
        assert_eq!(quad.polygon_count, 0);
        assert_eq!(insert_screen_polygon(&mut quad, 4), 0);
        assert_eq!(quad.indices, [0, 1, 2, 0, 2, 3]);
        assert_eq!(
            (quad.vertex_count, quad.triangle_count, quad.index_count),
            (4, 2, 6)
        );
        assert_eq!(quad.polygon_index, [0]);

        assert!(!set_screen_rectangle(&mut quad, 1, 0.0, 0.0, 1.0, 1.0));
        assert!(!set_screen_rectangle(&mut quad, -1, 0.0, 0.0, 1.0, 1.0));
        assert!(set_screen_rectangle(&mut quad, 0, 1.0, 2.0, 3.0, 4.0));
        assert_eq!(
            quad.vertices,
            [
                [1.0, 2.0, 0.0],
                [1.0, 6.0, 0.0],
                [4.0, 6.0, 0.0],
                [4.0, 2.0, 0.0]
            ]
        );
        assert_eq!((quad.dirty_flags, quad.bound_dirty), (1, 1));
        update_screen_bound(&mut quad);
        assert_eq!(quad.bound_dirty, 0);
        assert_eq!(quad.bound_center, [2.5, 4.0, 0.0]);
        assert_eq!(quad.bound_radius, 2.5);
        assert!(!set_screen_rectangle_uv(
            &mut quad, 0, 1, 0.0, 0.0, 1.0, 1.0
        ));
        assert!(set_screen_rectangle_uv(
            &mut quad, 0, 0, 0.25, 0.5, 0.75, 1.0
        ));
        assert_eq!(
            quad.texture_coordinates,
            [[0.25, 0.5], [0.25, 1.0], [0.75, 1.0], [0.75, 0.5]]
        );
    }

    #[test]
    fn empty_vertices_bound_to_the_origin() {
        assert_eq!(compute_bound_from_vertices(&[], 0), ([0.0; 3], 0.0));
    }

    /// No C++ unit test covers this directly: 00406950's attach order and flag masks.
    #[test]
    fn fallback_quad_attaches_in_order_and_merges() {
        let mut world = World::new();
        let texture = world.spawn_empty().id();
        let default_alpha = world.spawn_empty().id();
        let default_material = world.spawn_empty().id();
        let defaults = ScenePropertyDefaults {
            alpha: Some(default_alpha),
            material: Some(default_material),
            ..ScenePropertyDefaults::default()
        };
        let quad = build_image_space_fallback_quad(&mut world, &defaults, Some(texture)).unwrap();
        let list: Vec<Entity> = world
            .get::<NiAvProperties>(quad)
            .unwrap()
            .0
            .iter()
            .flatten()
            .copied()
            .collect();
        assert_eq!(list.len(), 4);
        // AddHead: the last attached is first.
        assert_eq!(
            world.get::<NiClass>(list[0]),
            Some(&NiClass::TexturingProperty)
        );
        assert_eq!(
            world.get::<NiClass>(list[3]),
            Some(&NiClass::VertexColorProperty)
        );
        assert_eq!(
            world.get::<VertexColorProperty>(list[3]).unwrap().flags_18,
            0x0000
        );
        assert_eq!(
            world.get::<ZBufferProperty>(list[1]).unwrap().flags_18,
            0x000C
        );
        let material = world.get::<MaterialProperty>(list[2]).unwrap();
        assert_eq!(material.values.emissive, [1.0; 3]);
        assert_eq!(material.raw_state_54, 2);
        assert_eq!(material.material_id_18, 2, "the shared counter's first ID");
        let base = world.get::<TexturingProperty>(list[0]).unwrap().maps_1c[0].unwrap();
        assert_eq!(base.flags_04, 0);
        assert_eq!(base.texture_08, Some(texture));
        let state = world.get::<NiGeometry>(quad).unwrap().property_state_ac;
        let state = world.get::<ScenePropertyState>(state.unwrap()).unwrap();
        assert_eq!(state.slots[0], Some(default_alpha), "no local alpha");
        assert_eq!(
            state.slots[2],
            Some(list[2]),
            "the local list replaces a default"
        );
        assert_eq!(state.slots[7], Some(list[3]));
        assert_eq!(state.slots[5], None);
        let world_state = world.get::<NiAvWorld>(quad).unwrap();
        assert_eq!(world_state.bound, [0.5, 0.5, 0.0, 0.5f32.sqrt()]);
    }
}
