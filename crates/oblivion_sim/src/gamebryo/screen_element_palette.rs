//! IDA: 007AB1D0 UI_InitializeScreenElementPalette: the 17 NiScreenElements behind 00B42CF8, with their stencil and
//! material overrides. Its only caller is the Phase 05 shader setup (007BA1FE inside 007BA0F0), which is not ported;
//! the C++ keeps this class in `gamebryo/registration.cpp` and checks it in the Phase 03 tests. No drawing: D3D9 stops
//! at this state.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::av_object::{NiAvParent, NiAvProperties};
use super::mesh_geometry::NiGeometry;
use super::object_net::{AlphaProperty, NiObjectNet};
use super::properties::{
    StencilProperty, VertexColorProperty, ZBufferProperty, new_material_property,
};
use super::registration::NiProcessDefaults;
use super::scene_node::NiAvWorld;
use super::scene_state::{
    attach_object_property, refresh_property_state, release_geometry_property_state,
};
use super::screen_elements::{
    ScreenElementsQuad, construct_screen_elements_data, insert_screen_polygon,
    set_screen_rectangle, set_screen_rectangle_uv, spawn_screen_property, update_screen_bound,
    update_screen_elements_world,
};
use super::stream_objects::NiClass;
use super::transform::NiTransform;

pub const SCREEN_ELEMENT_COUNT: usize = 17;

/// IDA: 00B2DE00..00B2DF14, 17 four-float records, read from the executable. The loop takes r, g and b as the material's
/// emissive; nothing reads the fourth float.
pub const SCREEN_ELEMENT_COLORS_B2DE00: [[f32; 4]; SCREEN_ELEMENT_COUNT] = [
    [0.0, 1.0, 0.0, 1.0],
    [0.0, 0.875, 0.125, 1.0],
    [0.0, 0.75, 0.25, 1.0],
    [0.0, 0.625, 0.375, 1.0],
    [0.0, 0.5, 0.5, 1.0],
    [0.0, 0.375, 0.625, 1.0],
    [0.0, 0.25, 0.75, 1.0],
    [0.0, 0.125, 0.875, 1.0],
    [0.0, 0.0, 1.0, 1.0],
    [0.125, 0.0, 0.875, 1.0],
    [0.25, 0.0, 0.75, 1.0],
    [0.375, 0.0, 0.625, 1.0],
    [0.5, 0.0, 0.5, 1.0],
    [0.625, 0.0, 0.375, 1.0],
    [0.75, 0.0, 0.25, 1.0],
    [0.875, 0.0, 0.125, 1.0],
    [1.0, 0.0, 0.0, 1.0],
];

/// IDA: 00708450 -> 00718A50 / 0070FD10: NiAVObject's constructor state, as the C++ port's `ColdGeometry` records it: local
/// and world transforms, a zero world bound and flags 0x000E. Value state, not the original layout. Completion: 100% of
/// the constructor fields.
/// TODO: [ ] Parent transforms and the render-mode-4 stencil drawing at 007AEBA7.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct ScreenElementColdGeometry {
    pub local: NiTransform,
    pub world: NiTransform,
    pub world_bound_center: [f32; 3],
    pub world_bound_radius: f32,
    pub flags: u16,
    pub parent_attached: bool,
    pub collision_attached: bool,
}

impl Default for ScreenElementColdGeometry {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            local: NiTransform::default(),
            world: NiTransform::default(),
            world_bound_center: [0.0; 3],
            world_bound_radius: 0.0,
            flags: 0x000E,
            parent_attached: false,
            collision_attached: false,
        }
    }
}

/// IDA: N/A one 00B42CF8 slot's host value: the NiScreenElements entity and the color record it used.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenElementEntry {
    pub color: [f32; 4],
    pub geometry: Entity,
}

/// IDA: 00B42CF8's 17 refcounted slots. `owned` stands for the references the geometries hold on their properties: every
/// entity initialization made, released together, because property refcounts are not modelled. Completion: 100% of the
/// slots; nothing reads them yet.
/// TODO: [ ] Connect the screen-element render consumer.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct UiScreenElementPalette {
    pub entries_b42cf8: [Option<ScreenElementEntry>; SCREEN_ELEMENT_COUNT],
    owned: Vec<Entity>,
}

/// IDA: N/A a FormHeap request at its original size: the reservation is what can fail. See `scene_state::merge_property_list`.
fn form_heap(bytes: usize) -> Result<Vec<u8>, GameError> {
    let mut storage = Vec::new();
    storage
        .try_reserve_exact(bytes)
        .map_err(|_| GameError::unclassified(0, "bad allocation"))?;
    Ok(storage)
}

// IDA: N/A a helper, with no address of its own.
fn own(world: &mut World, entity: Entity) {
    world
        .resource_mut::<UiScreenElementPalette>()
        .owned
        .push(entity);
}

// IDA: 007AB2A0..007AB5C0, one pass of the seventeen-entry loop. Hex-Rays' local names at this site are crossed: its
// `newProperty` holds the vertex color and its `vertex_color_property` the z-buffer. Completion: 100% of the store order
// through 00707370.
fn build_entry(
    world: &mut World,
    index: usize,
    shared: [Entity; 3],
    defaults: &super::scene_state::ScenePropertyDefaults,
) -> Result<(), GameError> {
    let [vertex_color, zbuffer, alpha] = shared;
    // FormHeap(0xC0) NiScreenElements over FormHeap(0x70) 00703230(0, 0, 1, 1, 1, 4, 1, 2, 1).
    let element_storage = form_heap(0xC0)?;
    let data_storage = form_heap(0x70)?;
    let mut quad = ScreenElementsQuad::default();
    construct_screen_elements_data(&mut quad);
    // 007AB3B7 / 3D9 / 3E4 / 407 -> 00702970 / 2EC0 / 3050 / 2FC0.
    insert_screen_polygon(&mut quad, 4);
    set_screen_rectangle(&mut quad, 0, 0.0, 0.0, 1.0, 1.0);
    update_screen_bound(&mut quad);
    set_screen_rectangle_uv(&mut quad, 0, 0, 0.0, 0.0, 1.0, 1.0);
    let geometry = world
        .spawn((
            NiClass::ScreenElements,
            quad,
            NiAvProperties::default(),
            NiGeometry::default(),
            NiAvWorld::default(),
            NiObjectNet::default(),
            NiAvParent::default(),
            ScreenElementColdGeometry::default(),
        ))
        .id();
    drop((element_storage, data_storage));
    own(world, geometry);

    // FormHeap(0x24) 00718F80: flags 0x4180 | 1, then the top nibble 2 (3 from index 16), then & 0xFC01; +0x1C = index + 1.
    let stencil_storage = form_heap(0x24)?;
    let mut stencil = StencilProperty::default();
    stencil.flags_18 |= 1;
    stencil.flags_18 = stencil.flags_18 & 0x0FFF | if index >= 16 { 0x3000 } else { 0x2000 };
    stencil.flags_18 &= 0xFC01;
    stencil.reference_1c = index as u32 + 1;
    let stencil = spawn_screen_property(world, NiClass::StencilProperty, stencil);
    drop(stencil_storage);
    own(world, stencil);

    // FormHeap(0x5C) 007095A0: the next 00B25AF4 ID (P08-RUN-002: 2..0x12); +0x54 is incremented before the emissive stores.
    let material_storage = form_heap(0x5C)?;
    let mut material = new_material_property(world);
    let color = SCREEN_ELEMENT_COLORS_B2DE00[index];
    material.raw_state_54 += 1;
    material.values.emissive = [color[0], color[1], color[2]];
    let material = spawn_screen_property(world, NiClass::MaterialProperty, material);
    drop(material_storage);
    own(world, material);

    // 00405680 five times, AddHead: the list ends alpha, z-buffer, vertex color, material, stencil.
    for property in [stencil, material, vertex_color, zbuffer, alpha] {
        attach_object_property(world, geometry, Some(property))?;
    }
    // 00707610.
    refresh_property_state(world, geometry, defaults)?;
    // 007AB564 00707370 Update(0.0, 1), before the slot store. The C++ port skips it.
    update_screen_elements_world(world, geometry);
    let bound = world
        .get::<NiAvWorld>(geometry)
        .expect("just spawned")
        .bound;
    let mut cold = world
        .get_mut::<ScreenElementColdGeometry>(geometry)
        .expect("just spawned");
    cold.world_bound_center = [bound[0], bound[1], bound[2]];
    cold.world_bound_radius = bound[3];
    world
        .resource_mut::<UiScreenElementPalette>()
        .entries_b42cf8[index] = Some(ScreenElementEntry { color, geometry });
    Ok(())
}

// IDA: N/A no address of its own; its comments cite 007AB204.
fn build(world: &mut World) -> Result<(), GameError> {
    let defaults = world
        .get_resource_or_insert_with(NiProcessDefaults::default)
        .scene_property_defaults();
    // 007AB204..007AB301: three owners made once, outside the loop, each FormHeap(0x1C). Every geometry retains them.
    let storage = form_heap(0x1C)?;
    let vertex_color = VertexColorProperty {
        flags_18: VertexColorProperty::default().flags_18 & 0xFFC7,
    };
    let vertex_color = spawn_screen_property(world, NiClass::VertexColorProperty, vertex_color);
    drop(storage);
    own(world, vertex_color);

    let storage = form_heap(0x1C)?;
    let mut zbuffer = ZBufferProperty::default();
    zbuffer.flags_18 &= !1;
    zbuffer.flags_18 &= !2;
    let zbuffer = spawn_screen_property(world, NiClass::ZBufferProperty, zbuffer);
    drop(storage);
    own(world, zbuffer);

    let storage = form_heap(0x1C)?;
    let mut alpha = AlphaProperty::default();
    alpha.flags_18 &= !1;
    let alpha = spawn_screen_property(world, NiClass::AlphaProperty, alpha);
    drop(storage);
    own(world, alpha);

    for index in 0..SCREEN_ELEMENT_COUNT {
        build_entry(world, index, [vertex_color, zbuffer, alpha], &defaults)?;
    }
    Ok(())
}

// IDA: 007AB1D0 with a nonzero argument. Host policy, as in the C++ port (IDA: N/A): the prior palette is released first and
// a failed allocation releases everything built so far, then reports it. The original replaces each slot in place and
// dereferences null on a failed allocation. Completion: 100% of the color, property and quad state; drawing is open.
// TODO: [ ] Compare the original's partial-allocation failure under a safe runtime.
pub fn initialize_screen_element_palette(world: &mut World) -> Result<(), GameError> {
    shutdown_screen_element_palette(world);
    let result = build(world);
    if result.is_err() {
        shutdown_screen_element_palette(world);
    }
    result
}

// IDA: 007AB1D0 with a zero argument clears all 17 slots, each dropping a geometry reference, and a geometry's last
// release frees its properties. Completion: 100% of the host release; a second call finds nothing.
// TODO: [ ] Verify repeated original initialize and cleanup ordering.
pub fn shutdown_screen_element_palette(world: &mut World) {
    let owned = {
        let mut palette = world.get_resource_or_insert_with(UiScreenElementPalette::default);
        palette.entries_b42cf8 = [None; SCREEN_ELEMENT_COUNT];
        std::mem::take(&mut palette.owned)
    };
    for &entity in &owned {
        if world.get::<NiGeometry>(entity).is_some() {
            release_geometry_property_state(world, entity);
        }
    }
    for entity in owned {
        world.despawn(entity);
    }
}

#[cfg(test)]
mod tests {
    use super::super::properties::{MaterialIdCounter, MaterialProperty};
    use super::super::registration::{StartupCallbackScheduler, register_known_crt_callbacks};
    use super::super::scene_state::ScenePropertyState;
    use super::*;

    /// `verify_startup_registration`'s palette checks.
    #[test]
    fn seventeen_elements_with_their_owners() {
        let mut world = World::new();
        let mut known = StartupCallbackScheduler::default();
        register_known_crt_callbacks(&mut known).unwrap();
        known.run(&mut world).unwrap();
        let retained = *world.resource::<NiProcessDefaults>();
        let ids_before = world.resource::<MaterialIdCounter>().0;
        // P08-RUN-002: 00B25AF4 is 1 entering 7098F4's constructor and 2 entering the palette's first, so the default
        // material holds ID 2 and the palette takes 3..0x13.
        assert_eq!(ids_before, 2);

        initialize_screen_element_palette(&mut world).unwrap();
        let entries = world.resource::<UiScreenElementPalette>().entries_b42cf8;
        assert!(entries.iter().all(Option::is_some));
        assert_eq!(
            world.resource::<MaterialIdCounter>().0,
            ids_before + 17,
            "P08-RUN-002: one ID per entry"
        );
        let state_of = |world: &World, entry: &ScreenElementEntry| {
            let state = world
                .get::<NiGeometry>(entry.geometry)
                .unwrap()
                .property_state_ac
                .unwrap();
            world.get::<ScenePropertyState>(state).unwrap().clone()
        };
        let first = state_of(&world, &entries[0].unwrap());
        for (index, entry) in entries.iter().enumerate() {
            let entry = entry.unwrap();
            let mesh = world.get::<ScreenElementsQuad>(entry.geometry).unwrap();
            assert_eq!(
                mesh.vertices,
                [
                    [0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [1.0, 1.0, 0.0],
                    [1.0, 0.0, 0.0]
                ]
            );
            assert_eq!(
                mesh.texture_coordinates,
                [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]
            );
            assert_eq!(mesh.indices, [0, 1, 2, 0, 2, 3]);
            assert_eq!(mesh.bound_center, [0.5, 0.5, 0.0]);
            assert!((mesh.bound_radius * mesh.bound_radius - 0.5).abs() < 1e-6);

            let cold = world
                .get::<ScreenElementColdGeometry>(entry.geometry)
                .unwrap();
            let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
            assert_eq!(cold.local.rotation, identity);
            assert_eq!(cold.world.rotation, identity);
            assert_eq!(cold.local.translation, [0.0; 3]);
            assert_eq!(cold.world.translation, [0.0; 3]);
            assert_eq!((cold.local.scale, cold.world.scale), (1.0, 1.0));
            // The C++ test expects the constructor's zero here because it skips 00707370; 007AB564 runs it, so
            // 00722AA0 leaves the model bound under the identity world transform.
            assert_eq!(cold.world_bound_center, mesh.bound_center);
            assert_eq!(cold.world_bound_radius, mesh.bound_radius);
            assert_eq!(cold.flags, 0x000E);
            assert!(!cold.parent_attached && !cold.collision_attached);

            let state = state_of(&world, &entry);
            // Three shared owners and two per-entry ones.
            assert_eq!(state.slots[0], first.slots[0]);
            assert_eq!(state.slots[7], first.slots[7]);
            assert_eq!(state.slots[9], first.slots[9]);
            if index != 0 {
                assert_ne!(state.slots[2], first.slots[2]);
                assert_ne!(state.slots[5], first.slots[5]);
            }

            let alpha = state.slots[0].unwrap();
            assert_ne!(Some(alpha), retained.alpha_b3fce4);
            assert_eq!(world.get::<AlphaProperty>(alpha).unwrap().flags_18, 0x00EC);
            let material = state.slots[2].unwrap();
            assert_ne!(Some(material), retained.material_b3faa4);
            let material = world.get::<MaterialProperty>(material).unwrap();
            assert_eq!(
                material.values.emissive,
                [entry.color[0], entry.color[1], entry.color[2]]
            );
            assert_eq!(material.raw_state_54, 2);
            assert_eq!(material.material_id_18, ids_before + index as u32 + 1);
            let stencil = world
                .get::<StencilProperty>(state.slots[5].unwrap())
                .unwrap();
            assert_eq!(stencil.flags_18, if index < 16 { 0x2001 } else { 0x3001 });
            assert_eq!(stencil.reference_1c, index as u32 + 1);
            assert_eq!(stencil.mask_20, 0xFFFF_FFFF);
            assert_eq!(
                world
                    .get::<VertexColorProperty>(state.slots[7].unwrap())
                    .unwrap()
                    .flags_18,
                0
            );
            assert_eq!(
                world
                    .get::<ZBufferProperty>(state.slots[9].unwrap())
                    .unwrap()
                    .flags_18,
                0x000C
            );
            assert_eq!(state.slots[3], retained.renderer_specific);
            assert_eq!(state.slots[4], retained.shade_b401ac);
            assert_eq!(state.slots[6], retained.texturing_b3f974);
            assert_eq!(state.slots[8], retained.wireframe_b3f984);
            assert_eq!(state.slots[1], None);

            // 00405680's AddHead over the attach order.
            let list = &world.get::<NiAvProperties>(entry.geometry).unwrap().0;
            assert_eq!(
                *list,
                [
                    state.slots[0],
                    state.slots[9],
                    state.slots[7],
                    state.slots[2],
                    state.slots[5]
                ]
            );
        }
        assert_eq!(entries[0].unwrap().color, [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(entries[8].unwrap().color, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(entries[16].unwrap().color, [1.0, 0.0, 0.0, 1.0]);

        shutdown_screen_element_palette(&mut world);
        let palette = world.resource::<UiScreenElementPalette>();
        assert!(palette.entries_b42cf8.iter().all(Option::is_none));
        assert!(world.get_entity(entries[0].unwrap().geometry).is_err());
        assert_eq!(
            world.query::<&ScenePropertyState>().iter(&world).count(),
            0,
            "no state outlives its geometry"
        );
        assert_eq!(world.query::<&ScreenElementsQuad>().iter(&world).count(), 0);
        // The process defaults are not the palette's to release.
        assert!(
            world
                .get::<AlphaProperty>(retained.alpha_b3fce4.unwrap())
                .is_some()
        );
        // A second cleanup finds nothing.
        shutdown_screen_element_palette(&mut world);
        known.shutdown(&mut world).unwrap();
    }
}
