//! Phase 19: 00404C60, what the first 005ADB40 runs before the bar's last stretch. On the default path it frees nothing (`run-k-phase19`, across `snapshot-purge_start` to `snapshot-phase19_end`).

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::facegen::FaceGenSingleton;
use crate::bs::model_loader::ModelMap;
use crate::bs::task_manager::TaskGlobals;
use crate::bs::tree_manager::TreeManagerGlobals;

/// IDA: 0043BEB0 ModelLoader_PurgeUnusedModels: a model with a reference count of 0 leaves the map. Its tree stays with the World, where the original frees it. The second loop, over the KF map, has nothing to do: the port holds no KF model, and the map is empty in the run. Returns how many went. Completion: 100% of the model map.
#[must_use]
pub fn purge_unused_models(models: &mut ModelMap) -> usize {
    let store = &models.store;
    let mut removed = 0;
    for bucket in &mut models.buckets {
        let before = bucket.len();
        bucket.retain(|entry| store[entry.model.0].references_04 & 0xFFFF != 0);
        removed += before - bucket.len();
    }
    removed
}

/// IDA: 00404C60(g_osGlobals, 1). 0054FE70 unloads the EGM and EGT data of FaceGen's +0xDAC map, null here. 0043BEB0. 0043FC20(0): 0055E390(1), null here, twice; 007B84E0 the shaders' pass textures and 004A25F0's palette purge, which remove nothing here (185 entries, each with a reference besides the palette's), and the port holds neither pass textures nor texture reference counts; 00402740's pool pages are the host's. Returns the models that went. Completion: 100% of the state the port holds.
/// # Errors
/// A stop on FaceGen's map or the tree map.
pub fn purge_after_loading(world: &mut World) -> Result<usize, StreamError> {
    if world
        .get_resource::<FaceGenSingleton>()
        .and_then(|singleton| singleton.0.as_ref())
        .is_some_and(|state| state.model_map_dac != 0)
    {
        return Err(StreamError(
            "0054FE82: FaceGen's EGM and EGT map isn't ported",
        ));
    }
    let removed = world
        .get_resource_mut::<TaskGlobals>()
        .and_then(|tasks| tasks.into_inner().model_loader_b33a1c.as_mut())
        .map_or(0, |loader| purge_unused_models(&mut loader.models));
    if world
        .get_resource::<TreeManagerGlobals>()
        .and_then(|globals| globals.tree_manager_b39e04.as_ref())
        .is_some_and(|manager| manager.cached_properties_00 != 0)
    {
        return Err(StreamError(
            "0043FC62: purging a cached-property map is not ported",
        ));
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bs::model_loader::{Model, ModelId, ModelMapEntry};
    use crate::bs::tree_manager::BsTreeManager;
    use crate::tes::facegen::FaceGenState;

    fn model(references: u32) -> Model {
        Model {
            references_04: references,
            ..Model::default()
        }
    }

    // 0043BEB0 tests the low word of +0x04.
    #[test]
    fn only_a_count_of_zero_leaves_the_map() {
        let entry = |key: &[u8], model: usize| ModelMapEntry {
            key: key.to_vec(),
            model: ModelId(model),
        };
        let mut models = ModelMap {
            store: vec![model(1), model(0), model(0x1_0000)],
            buckets: vec![vec![entry(b"a", 0), entry(b"b", 1)], vec![entry(b"c", 2)]],
        };
        assert_eq!(purge_unused_models(&mut models), 2);
        assert_eq!(models.buckets[0].len(), 1);
        assert_eq!(models.buckets[0][0].key, b"a");
        assert!(models.buckets[1].is_empty());
        assert_eq!(models.store.len(), 3, "the slots stay");
    }

    // The run's path: both maps null. The other two are stops.
    #[test]
    fn the_null_maps_return_and_the_others_stop() {
        let mut world = World::new();
        assert_eq!(purge_after_loading(&mut world).unwrap(), 0);
        world.insert_resource(FaceGenSingleton(Some(Box::new(FaceGenState {
            model_map_dac: 1,
            ..FaceGenState::default()
        }))));
        assert!(purge_after_loading(&mut world).is_err());
        world.resource_mut::<FaceGenSingleton>().0 = None;
        world.insert_resource(TreeManagerGlobals {
            tree_manager_b39e04: Some(BsTreeManager {
                cached_properties_00: 1,
                ..BsTreeManager::default()
            }),
            ..TreeManagerGlobals::default()
        });
        assert!(purge_after_loading(&mut world).is_err());
    }
}
