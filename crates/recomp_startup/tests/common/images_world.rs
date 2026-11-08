//! IDA: N/A the bytes of the world forms and references, for the sample rows. These types have no DATA rows, so a pointer compares by null-ness.

use bevy_ecs::prelude::*;
use recomp_core::pointer_list::PointerList;
use recomp_game::tes::actor::TesObjectRefr;
use recomp_game::tes::base_extra_list::ExtraDataList;
use recomp_game::tes::data_handler::TesTexture;
use recomp_game::tes::form_components::TesFullName;
use recomp_game::tes::forms_world::{
    PointerMap, TesObjectCell, TesObjectLand, TesPathGrid, TesRoad, TesWorldSpace,
};

use super::image::Image;

// IDA: an ExtraDataList at `at`: vtable, +0x04 the head, +0x08..+0x13 the presence bits.
fn extra_list(image: &mut Image, at: usize, list: &ExtraDataList) {
    image.u32(at, list.original_vtable);
    image.pointer(at + 4, "extra head", !list.entries.is_empty());
    image.bytes(at + 8, &list.presence_08);
}

// IDA: a NiTPointerMap head at `at`.
fn pointer_map<V>(image: &mut Image, at: usize, map: &PointerMap<V>) {
    image.u32(at, map.original_vtable);
    image.u32(at + 4, map.buckets_04);
    image.pointer(at + 8, "map buckets", true);
    image.u32(at + 12, map.count_0c);
}

// IDA: a BSSimpleList head at `at`: +0 the item, +4 the next node.
fn list_head<T: Copy + PartialEq>(image: &mut Image, at: usize, list: &PointerList<T>) {
    let count = list.items().count();
    image.pointer(at, "list item", count > 0);
    image.pointer(at + 4, "list next", count > 1);
}

// IDA: TESObjectCELL (0x58): +0x27 stays unset.
pub fn cell_image(world: &World, form: Entity) -> Image {
    let cell = world.get::<TesObjectCell>(form).unwrap();
    let mut image = Image::new(0x58);
    image.form_head(world, form);
    image.full_name(0x18, world.get::<TesFullName>(form).unwrap());
    image.u8(0x24, cell.flags_24);
    image.u8(0x25, cell.field_25);
    image.u8(0x26, cell.process_level_26);
    extra_list(&mut image, 0x28, &cell.extra_28);
    image.pointer(0x3C, "cell data", cell.data_3c.is_some());
    image.pointer(0x40, "land", cell.land_40.is_some());
    image.pointer(0x44, "path grid", cell.path_grid_44.is_some());
    list_head(&mut image, 0x48, &cell.objects_48);
    if cell.flags_24 & 1 != 0 {
        image.u32(0x50, cell.toft_50);
    } else {
        image.pointer(0x50, "world", cell.world_50.is_some());
    }
    image.u32(0x54, cell.field_54);
    image
}

// IDA: TESObjectREFR to +0x40, the part REFR, ACHR and ACRE share and the run dumped.
fn reference_head(image: &mut Image, world: &World, form: Entity) {
    let reference = world.get::<TesObjectRefr>(form).unwrap();
    image.form_head(world, form);
    image.u32(0x18, reference.child_cell_vtable_18);
    image.pointer(0x1C, "base", reference.base_form_1c.is_some());
    for (index, value) in reference.rotation_20.iter().enumerate() {
        image.f32(0x20 + 4 * index, *value);
    }
    for (index, value) in reference.position_2c.iter().enumerate() {
        image.f32(0x2C + 4 * index, *value);
    }
    image.f32(0x38, reference.scale_38);
    image.u32(0x3C, reference.loaded_3d_3c);
}

// IDA: TESObjectREFR (0x58).
pub fn reference_image(world: &World, form: Entity) -> Image {
    let reference = world.get::<TesObjectRefr>(form).unwrap();
    let mut image = Image::new(0x58);
    reference_head(&mut image, world, form);
    image.pointer(0x40, "parent cell", reference.parent_cell_40.is_some());
    extra_list(&mut image, 0x44, &reference.extra_data_44);
    image
}

// IDA: Character and Creature, as far as the run dumped them (0x40).
pub fn actor_reference_image(world: &World, form: Entity) -> Image {
    let mut image = Image::new(0x40);
    reference_head(&mut image, world, form);
    image
}

// IDA: TESPathGrid (0x54): +0x32 stays unset.
pub fn path_grid_image(world: &World, form: Entity) -> Image {
    let grid = world.get::<TesPathGrid>(form).unwrap();
    let mut image = Image::new(0x54);
    image.form_head(world, form);
    image.u32(0x18, grid.child_cell_vtable_18);
    image.u32(0x1C, grid.reference_1c);
    image.pointer(0x20, "cell", grid.cell_20.is_some());
    image.pointer(0x24, "points", grid.points_24.is_some());
    image.pointer(0x28, "inter", !grid.inter_points_28.is_empty());
    image.pointer(0x2C, "inter next", false);
    image.u16(0x30, grid.point_count_30);
    pointer_map(&mut image, 0x34, &grid.references_34);
    pointer_map(&mut image, 0x44, &grid.points_44);
    image
}

// IDA: TESWorldSpace (0xE0): +0x5D..+0x5F and +0xD8 stay unset.
pub fn world_space_image(world: &World, form: Entity) -> Image {
    let space = world.get::<TesWorldSpace>(form).unwrap();
    let mut image = Image::new(0xE0);
    image.form_head(world, form);
    image.full_name(0x18, world.get::<TesFullName>(form).unwrap());
    image.texture(0x24, world.get::<TesTexture>(form).unwrap());
    image.pointer(0x30, "cell map", true);
    image.pointer(0x34, "persistent cell", space.persistent_cell_34.is_some());
    pointer_map(&mut image, 0x38, &space.lod_38);
    image.pointer(0x48, "LOD world", space.world_48.is_some());
    image.u32(0x4C, space.lod_4c[0]);
    image.u32(0x50, space.lod_4c[1]);
    image.pointer(0x54, "road", space.road_54.is_some());
    image.form_ref(0x58, "climate", space.climate_58);
    image.u8(0x5C, space.flags_5c);
    image.pointer(0x60, "subspaces", space.subspaces_60.is_some());
    pointer_map(&mut image, 0x64, &space.references_64);
    list_head(&mut image, 0x74, &space.unsorted_74);
    image.form_ref(0x7C, "parent", space.parent_7c);
    image.form_ref(0x80, "water", space.water_80);
    image.bytes(0x84, &space.map_84);
    image.u32(0x94, space.sound_94);
    for (index, value) in space.bounds_98.iter().enumerate() {
        image.f32(0x98 + 4 * index, *value);
    }
    image.pointer(0xA8, "OFST", space.offsets_a8.is_some());
    for (index, value) in space.master_bounds_ac.iter().enumerate() {
        image.u32(0xAC + 4 * index, *value);
    }
    image.u32(0xBC, space.record_offset_bc);
    image.bs_string(0xC0, "editor ID", &space.editor_c0);
    pointer_map(&mut image, 0xC8, &space.map_c8);
    image.u32(0xDC, space.field_dc);
    image
}

// IDA: TESObjectLAND (0x28).
pub fn land_image(world: &World, form: Entity) -> Image {
    let land = world.get::<TesObjectLand>(form).unwrap();
    let mut image = Image::new(0x28);
    image.form_head(world, form);
    image.u32(0x18, land.child_cell_vtable_18);
    image.u32(0x1C, land.flags_1c);
    image.pointer(0x20, "cell", land.cell_20.is_some());
    image.pointer(0x24, "data", land.data_24.is_some());
    image
}

// IDA: TESRoad (0x30).
pub fn road_image(world: &World, form: Entity) -> Image {
    let road = world.get::<TesRoad>(form).unwrap();
    let mut image = Image::new(0x30);
    image.form_head(world, form);
    image.u32(0x18, road.points_18);
    pointer_map(&mut image, 0x1C, &road.map_1c);
    image.pointer(0x2C, "world", road.world_2c.is_some());
    image
}
