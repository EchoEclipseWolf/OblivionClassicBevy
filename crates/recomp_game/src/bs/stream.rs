//! 006F9980 BSStream_LoadFromPath, the NIF load of Meshes\Sky\Atmosphere.nif (0053B3F0) and Meshes\Sky\Clouds.nif (0053BDB0): the
//! 00B27E4C open slot (FileFinder/BSFile or the archives), the path store, 007120F0 NiStream_LoadFromOpenReader and the BSStream
//! +0x3C override 006F9B20, whose null +0x488 remap map delegates to 00714390 (`load_supported_stream_file`).
//!
//! Host substitution: the reader's bytes are read in order into one buffer before 00714390, where the original streams them in 0x2800-byte
//! reads. The bytes and their order are the same.
//! Evidence: evidence/phase08-window-2026-10-06/decompile-b (006F9980, 007120F0, 006F9B20, 00436420), P08-RUN-001 (both loads, reader 0).

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use crate::bs::file::GameFile;
use crate::gamebryo::registration::NiFactoryRegistry;
use crate::gamebryo::stream_callbacks::{StreamRootCallbackControl, StreamRootCallbackRuntime};
use crate::gamebryo::stream_file_pipeline::{
    StreamFileCursors, StreamFilePipelineError, StreamFilePipelineRuntime,
    StreamFilePipelineSuccess, StreamOwnerArray, StreamTerminalServices, StreamTerminalState,
    load_supported_stream_file,
};
use crate::gamebryo::stream_graph::StreamGraphLinkRuntime;
use crate::gamebryo::stream_postload::StreamPostLoadRuntime;
use crate::gamebryo::texture::SourceTextureLoadServices;

/// IDA: 006F9980 passes 0x2800 as the reader buffer size (00748110 slot).
pub const BS_STREAM_READER_BUFFER_SIZE: u32 = 0x2800;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BsStreamLoadStatus {
    /// 006F9980 returned 1 (the 00714390 load succeeded).
    Loaded,
    /// The open slot returned null: 006F99A7 returns 0 before the path store.
    NoReader,
    /// 007120F0: the reader's vtable +0x04 (good) is false.
    ReaderFailed,
    /// 00714390 returned 0 (header, graph, post-load or callback failure).
    LoadFailed,
}

/// IDA: 00B27E4C open slot (thunk 00748100): the file finder and archives, in the host's hands.
pub type BsStreamOpen<'a> = &'a mut (dyn FnMut(&str, i32, u32) -> Option<Box<dyn GameFile>> + 'a);

/// IDA: N/A the services 006F9980 reaches: the open slot and the NiStream load runtime (factory registry, link and post-load services).
/// The root callbacks are the registry's own, as the C++ `current_registry` gives them.
pub struct BsStreamServices<'a> {
    pub open: BsStreamOpen<'a>,
    pub registry: &'a NiFactoryRegistry,
    pub draw_rand: Option<&'a mut (dyn FnMut() -> i32 + 'a)>,
    /// IDA: N/A `draw_rand` stands for the IO thread's draw, under its 027F control word.
    pub draw_on_io_thread: bool,
    pub source_textures: Option<&'a mut (dyn SourceTextureLoadServices + 'a)>,
    pub post_load: StreamPostLoadRuntime<'a>,
}

/// IDA: N/A result of one BSStream load: the status, the +0xE0 path copy and the loaded graph (its roots are the NiStream top-level
/// objects).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BsStreamLoad {
    pub status: BsStreamLoadStatus,
    /// +0xE0 (strcpy_s 0x104).
    pub path: String,
    pub stream: Option<StreamFilePipelineSuccess>,
    pub error: Option<StreamFilePipelineError>,
}

impl BsStreamLoad {
    /// IDA: NiStream top-level array size (+0x210, the 0053B3F0 / 0053BDB0 `!= 1` test). Completion: 100% host view (0 without a
    /// loaded stream).
    #[must_use]
    pub fn root_count(&self) -> usize {
        self.stream
            .as_ref()
            .map_or(0, |stream| stream.graph.roots.owners.len())
    }

    /// IDA: top-level object `index` (+0x208)[index]; null when absent. Completion: 100%.
    #[must_use]
    pub fn root(&self, index: usize) -> Option<Entity> {
        self.stream
            .as_ref()
            .and_then(|stream| stream.graph.roots.owners.get(index).copied().flatten())
    }
}

// IDA: 006F9980 BSStream_LoadFromPath(stream, path, reader = null): the open slot (path, mode 0, 0x2800); null returns 0. Otherwise
// strcpy_s(+0xE0, 0x104, path), nullsub_2, 00747930 SetReferencePath(+0x1E8 search path, +0xE0) (BSSearchPath ignores it), then 007120F0:
// the reader's vtable +0x04 good test, +0x21C = reader, vtable +0x3C 006F9B20 -> 00714390 with the +0x488 remap map null; +0x21C is
// cleared and the reader deleted (vtable +0, 1). A path of 0x104+ characters is an error (the strcpy_s invalid-parameter handler).
// Completion: 100% of the order.
// TODO: [ ] Bind the 007135C0 terminal owner array of the live NiStream.
pub fn load_bs_stream_from_path(
    world: &mut World,
    path: &str,
    services: &mut BsStreamServices<'_>,
) -> Result<BsStreamLoad, StreamError> {
    load_bs_stream_with_reader(world, path, None, services)
}

// IDA: 006F9980 with its reader argument, as 00439524 passes it: 006F998D a nonnull reader skips the open slot, and 006F99E3 deletes it as it deletes one it opened. A null reader takes the slot, as above. Completion: 100%.
pub fn load_bs_stream_with_reader(
    world: &mut World,
    path: &str,
    given_reader: Option<Box<dyn GameFile>>,
    services: &mut BsStreamServices<'_>,
) -> Result<BsStreamLoad, StreamError> {
    let mut load = BsStreamLoad {
        status: BsStreamLoadStatus::NoReader,
        path: String::new(),
        stream: None,
        error: None,
    };
    // 006F998F..006F99A1: no reader -> the open slot (path, 0, 0x2800).
    // 006F99A7: a null reader returns 0.
    let Some(mut reader) =
        given_reader.or_else(|| (services.open)(path, 0, BS_STREAM_READER_BUFFER_SIZE))
    else {
        return Ok(load);
    };
    // 006F99B7 strcpy_s(+0xE0, 0x104, path).
    if path.len() >= 0x104 {
        return Err(StreamError("006F99B7: strcpy_s path of 0x104+ characters"));
    }
    load.path = path.to_owned();
    // 006F99CC 00747930: BSSearchPath never reads the reference path.
    // 007120FF reader vtable +0x04 (good).
    if !reader.good() {
        load.status = BsStreamLoadStatus::ReaderFailed;
        return Ok(load);
    }
    // 00714390 through 006F9B20 (remap map +0x488 null): the host reads the reader's bytes in order into one buffer.
    let size = reader.size() as usize;
    let mut bytes = vec![0; size];
    let read = if size == 0 {
        0
    } else {
        reader.read(&mut bytes)
    };
    bytes.truncate(read);

    let mut preload = StreamOwnerArray::default();
    let mut terminal = StreamTerminalState::default();
    let (mut enter, mut thread, mut release, mut leave) = (|| {}, || 0u32, |_: u32| {}, || {});
    let mut owner_services = StreamTerminalServices {
        enter_critical_section: Some(&mut enter),
        current_thread_id: Some(&mut thread),
        release_owner: Some(&mut release),
        leave_critical_section: Some(&mut leave),
        ..Default::default()
    };
    let mut cursors = StreamFileCursors::default();
    let mut control = StreamRootCallbackControl::default();
    let callbacks = services.registry.stream_callbacks().to_vec();
    let mut current = move |_: &World| callbacks.clone();
    let draw_rand: Option<&mut (dyn FnMut() -> i32 + '_)> = match &mut services.draw_rand {
        Some(service) => Some(&mut **service),
        None => None,
    };
    let source_textures: Option<&mut (dyn SourceTextureLoadServices + '_)> =
        match &mut services.source_textures {
            Some(service) => Some(&mut **service),
            None => None,
        };
    let post_load = StreamPostLoadRuntime {
        rotation_target_data: match &mut services.post_load.rotation_target_data {
            Some(service) => Some(&mut **service),
            None => None,
        },
        precache_source_textures: services.post_load.precache_source_textures,
        release_static_texture_pixel_data: services.post_load.release_static_texture_pixel_data,
        create_source_texture_renderer_data: match &mut services
            .post_load
            .create_source_texture_renderer_data
        {
            Some(service) => Some(&mut **service),
            None => None,
        },
        invoke_unported: match &mut services.post_load.invoke_unported {
            Some(service) => Some(&mut **service),
            None => None,
        },
    };
    let loaded = load_supported_stream_file(
        world,
        services.registry,
        &bytes,
        StreamFilePipelineRuntime {
            preload: &mut preload,
            terminal: &mut terminal,
            owner_services: &mut owner_services,
            cursors: &mut cursors,
            control: &mut control,
            link_runtime: StreamGraphLinkRuntime {
                draw_rand,
                draw_on_io_thread: services.draw_on_io_thread,
                source_textures,
                stream_auxiliary_04: None,
            },
            post_load_runtime: post_load,
            callback_runtime: StreamRootCallbackRuntime {
                current_callbacks: Some(&mut current),
                invoke: None,
            },
            publish_loaded_object: None,
        },
    );
    // 006F99E3 the reader is deleted (vtable +0, 1) when `reader` leaves scope.
    match loaded {
        Ok(success) => {
            load.status = BsStreamLoadStatus::Loaded;
            load.stream = Some(success);
        }
        Err(error) => {
            load.status = BsStreamLoadStatus::LoadFailed;
            load.error = Some(error);
        }
    }
    Ok(load)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bs::file::NiFile;
    use crate::bs::file::NiFileMode;

    fn services<'a>(
        open: BsStreamOpen<'a>,
        registry: &'a NiFactoryRegistry,
    ) -> BsStreamServices<'a> {
        BsStreamServices {
            open,
            registry,
            draw_rand: None,
            draw_on_io_thread: false,
            source_textures: None,
            post_load: StreamPostLoadRuntime::default(),
        }
    }

    #[test]
    fn a_null_reader_returns_before_the_path_store() {
        let mut world = World::new();
        let registry = NiFactoryRegistry::default();
        let mut requests = Vec::new();
        let mut open = |path: &str, mode: i32, size: u32| -> Option<Box<dyn GameFile>> {
            requests.push((path.to_owned(), mode, size));
            None
        };
        let load = load_bs_stream_from_path(
            &mut world,
            "Meshes\\Sky\\Clouds.nif",
            &mut services(&mut open, &registry),
        )
        .unwrap();
        assert_eq!(load.status, BsStreamLoadStatus::NoReader);
        assert_eq!(load.path, "");
        assert_eq!(load.root_count(), 0);
        assert_eq!(load.root(0), None);
        assert_eq!(
            requests,
            [("Meshes\\Sky\\Clouds.nif".to_owned(), 0, 0x2800)]
        );
    }

    // 006F998D: a given reader never reaches the open slot.
    #[test]
    fn a_given_reader_skips_the_open_slot() {
        let mut world = World::new();
        let registry = NiFactoryRegistry::default();
        let root = std::env::temp_dir();
        let mut opens = 0;
        let mut open = |_: &str, _: i32, _: u32| -> Option<Box<dyn GameFile>> {
            opens += 1;
            None
        };
        let reader = NiFile::new(
            &root,
            "no-such-recomp-file.nif",
            NiFileMode::ReadOnly,
            0x2800,
        );
        let load = load_bs_stream_with_reader(
            &mut world,
            "x.nif",
            Some(Box::new(reader)),
            &mut services(&mut open, &registry),
        )
        .unwrap();
        assert_eq!(load.status, BsStreamLoadStatus::ReaderFailed);
        assert_eq!(load.path, "x.nif");
        assert_eq!(opens, 0);
    }

    #[test]
    fn a_bad_reader_stops_at_its_good_test() {
        let mut world = World::new();
        let registry = NiFactoryRegistry::default();
        let root = std::env::temp_dir();
        let mut open = |path: &str, _: i32, size: u32| -> Option<Box<dyn GameFile>> {
            Some(Box::new(NiFile::new(
                &root,
                path,
                NiFileMode::ReadOnly,
                size,
            )))
        };
        let load = load_bs_stream_from_path(
            &mut world,
            "no-such-recomp-file.nif",
            &mut services(&mut open, &registry),
        )
        .unwrap();
        assert_eq!(load.status, BsStreamLoadStatus::ReaderFailed);
        assert_eq!(load.path, "no-such-recomp-file.nif");
    }

    #[test]
    fn the_strcpy_s_limit_is_an_error() {
        let mut world = World::new();
        let registry = NiFactoryRegistry::default();
        let root = std::env::temp_dir();
        let mut open = |path: &str, _: i32, size: u32| -> Option<Box<dyn GameFile>> {
            Some(Box::new(NiFile::new(
                &root,
                path,
                NiFileMode::ReadOnly,
                size,
            )))
        };
        let long = "x".repeat(0x104);
        assert_eq!(
            load_bs_stream_from_path(&mut world, &long, &mut services(&mut open, &registry))
                .unwrap_err(),
            StreamError("006F99B7: strcpy_s path of 0x104+ characters")
        );
    }

    #[test]
    fn garbage_bytes_fail_inside_the_stream_load() {
        let mut world = World::new();
        let registry = NiFactoryRegistry::default();
        let folder = std::env::temp_dir().join(format!("recomp-stream-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("bad.nif"), b"not a nif\n").unwrap();
        let mut open = |path: &str, _: i32, size: u32| -> Option<Box<dyn GameFile>> {
            Some(Box::new(NiFile::new(
                &folder,
                path,
                NiFileMode::ReadOnly,
                size,
            )))
        };
        let load =
            load_bs_stream_from_path(&mut world, "bad.nif", &mut services(&mut open, &registry))
                .unwrap();
        assert_eq!(load.status, BsStreamLoadStatus::LoadFailed);
        assert!(load.error.is_some());
        let _ = std::fs::remove_dir_all(&folder);
    }
}
