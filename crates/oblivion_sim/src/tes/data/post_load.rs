//! Phase 12d: 0044F780..0044FA38, the tail of 0044F3D0: vt+0x6C over the ten lists, the name sorts, the time globals, the topics, the TESObject chain, TLS +0x184 cleared, the regions, the worldspaces, the interior cells, the idle roots, the effect settings and the packages.
//!
//! Evidence: objdump of each address named here; P12-RUN-005's PLIST and POST rows.

use bevy_ecs::prelude::*;
use oblivion_core::crt;
use oblivion_core::error::GameError;
use oblivion_core::pointer_list::PointerList;

use super::data_handler::{DataHandler, DataHandlerLists};
use super::data_load::{DataLoadGlobals, DataLoadServices};
use crate::tes::form_components::{TesFullName, TesObjectLinks};
use crate::tes::forms_ai::init_idle_roots;
use crate::tes::forms_dialogue::{sort_topic_entries, sort_topics_by_name};
use crate::tes::forms_init::init_item_of;
use crate::tes::forms_init_world::world_subspace_pass;
use crate::tes::forms_magic::init_effect_settings;
use crate::tes::global::init_time_globals;
use crate::tes::tes::GameThreadLocals;

// IDA: the shape of each list pass: over the list while the item is nonzero, vt+0x6C.
fn run_list(
    world: &mut World,
    items: Vec<Entity>,
    services: &mut DataLoadServices<'_>,
) -> Result<(), GameError> {
    for form in items {
        init_item_of(world, form, services)?;
    }
    Ok(())
}

// IDA: N/A a list's items, as a pass walks them.
pub(crate) fn items(list: &PointerList<Entity>) -> Vec<Entity> {
    list.items().collect()
}

// IDA: `_mbsicmp` 009836C9 of two +0x1C names, with `Default` for a null: over the single-byte code page of the run it is `_stricmp`.
fn compare_names(world: &World, first: Entity, second: Entity) -> i32 {
    let name = |form: Entity| {
        world
            .get::<TesFullName>(form)
            .and_then(|name| name.name_04.data_00.clone())
            .unwrap_or_default()
    };
    crt::stricmp(&name(first), &name(second))
}

// IDA: the bubble sort 0052F6D0 and 0044D610 share. The count of nonzero items less one is the pass count; each pass walks the nodes swapping the items of a node and its next when the first's name compares above the second's.
pub(crate) fn bubble_sort_by_name(world: &World, list: &mut PointerList<Entity>) {
    let mut sorted: Vec<Entity> = list.items().collect();
    if sorted.len() < 2 {
        return;
    }
    for _ in 0..sorted.len() - 1 {
        for at in 0..sorted.len() - 1 {
            if compare_names(world, sorted[at], sorted[at + 1]) > 0 {
                sorted.swap(at, at + 1);
            }
        }
    }
    list.set_items(&sorted);
}

/// IDA: 0044D610 DataHandler_SortLists: the same sort over +0x54 (CLAS), then +0x44 (RACE). Completion: 100%.
pub fn sort_class_and_race_lists(world: &mut World) {
    let mut list = std::mem::take(&mut world.resource_mut::<DataHandlerLists>().classes_54);
    bubble_sort_by_name(world, &mut list);
    world.resource_mut::<DataHandlerLists>().classes_54 = list;
    let mut list = std::mem::take(&mut world.resource_mut::<DataHandlerLists>().races_44);
    bubble_sort_by_name(world, &mut list);
    world.resource_mut::<DataHandlerLists>().races_44 = list;
}

/// IDA: 0044F780..0044FA38. The ten list passes (0044F785 is `LoadStage::NextFile`'s +0xCD7), the topic sort unless exactly one file is Loaded, 00402860, the topics, 0052FAE0 by the same test, the TESObject chain, TLS +0x184 = 0, the regions, 004EF290 per worldspace, the +0xC4 cells, the worldspaces, 00520FA0, 00416900, the +0x04 packages, 0044D610, 00B33A9C = 0. Completion: 100%.
pub fn run_post_load_passes(
    world: &mut World,
    services: &mut DataLoadServices<'_>,
) -> Result<(), GameError> {
    let lists = world.resource::<DataHandlerLists>();
    let order = [
        items(&lists.races_44),
        items(&lists.factions_5c),
        items(&lists.quests_84),
        items(&lists.birth_signs_8c),
        items(&lists.combat_styles_94),
        items(&lists.effect_shaders_ac),
        items(&lists.load_screens_9c),
        items(&lists.object_animations_b4),
        items(&lists.water_forms_a4),
        items(&lists.land_textures_4c),
    ];
    for list in order {
        run_list(world, list, services)?;
    }
    // 0044F8C4..0044F8F6: the walk stops at the first null item.
    let data = world.resource::<DataHandler>();
    let mut loaded = 0;
    for &id in &data.files_8c8 {
        let Some(file) = data.file_store.get(id) else {
            break;
        };
        if file.is_loaded() {
            loaded += 1;
        }
    }
    let one_file = loaded == 1;
    if !one_file {
        sort_topics_by_name(world);
    }
    init_time_globals(world);
    let topics = items(&world.resource::<DataHandlerLists>().topics_7c);
    run_list(world, topics, services)?;
    if !one_file {
        sort_topic_entries(world);
    }
    // 0044F932..0044F954: 004B2460 the first, 004B2360 each next, read after the object's vt+0x6C.
    let mut object = world.resource::<DataHandlerLists>().objects_00.first_04;
    while let Some(form) = object {
        init_item_of(world, form, services)?;
        object = world
            .get::<TesObjectLinks>(form)
            .expect("the list holds TESObjects")
            .next_20;
    }
    // 0044F95A.
    world.resource_mut::<GameThreadLocals>().byte_184 = 0;
    // 0044F961..0044F990 +0xBC.
    let regions = items(&world.resource::<DataHandlerLists>().regions_bc);
    run_list(world, regions, services)?;
    // 0044F992..0044F9B0 004EF290 per worldspace, skipping a null item.
    let worlds = items(&world.resource::<DataHandlerLists>().world_spaces_0c);
    for form in &worlds {
        world_subspace_pass(world, *form)?;
    }
    // 0044F9B2..0044F9D9 the +0xCC size read once (`cells_c0` starts at +0xC0), each nonzero +0xC4 slot.
    let cells: Vec<Option<Entity>> = {
        let array = &world.resource::<DataHandler>().cells_c0;
        array
            .cells
            .iter()
            .take(array.size_0c as usize)
            .copied()
            .collect()
    };
    for cell in cells.into_iter().flatten() {
        init_item_of(world, cell, services)?;
    }
    // 0044F9DB..0044F9F3.
    run_list(world, worlds, services)?;
    // 0044F9F5 00520FA0 and 0044FA00 00416900.
    init_idle_roots(world, services)?;
    init_effect_settings(world, services)?;
    // 0044FA05..0044FA22 +0x04, past a null item in the original: the list holds none.
    let packages = items(&world.resource::<DataHandlerLists>().packages_04);
    run_list(world, packages, services)?;
    // 0044FA26.
    sort_class_and_race_lists(world);
    // 0044FA2E.
    world.resource_mut::<DataLoadGlobals>().current_cell_b33a9c = None;
    Ok(())
}
