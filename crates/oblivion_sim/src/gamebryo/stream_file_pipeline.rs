//! A whole NIF file: 00712C00's header, then 00714390's passes.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::registration::NiFactoryRegistry;
use super::stream_callbacks::{
    StreamRootCallbackControl, StreamRootCallbackResult, StreamRootCallbackRuntime,
    StreamRootCallbackStatus,
};
use super::stream_graph::{
    StreamGraph, StreamGraphError, StreamGraphErrorCode, StreamGraphLinkRuntime,
    StreamGraphProgress, StreamGraphProgressDecision, despawn_stream_graph, load_stream_graph_body,
};
use super::stream_objects::StreamObjectEntry;
use super::stream_postload::{
    StreamFinalizeResult, StreamPostLoadResult, StreamPostLoadRuntime, StreamPostLoadStatus,
    finalize_stream_graph,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamFileHeaderErrorCode {
    Truncated,
    NotNif,
    UnsupportedVersion,
    UnsupportedEndian,
    UnsupportedUserVersion,
    UnsupportedMetadataWidth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamFileHeaderError {
    pub code: StreamFileHeaderErrorCode,
    pub byte_offset: usize,
    pub original_error_code: Option<u32>,
}

/// IDA: 00712C00 stores binary version +0xD8, user version +0xDC, owner-array capacity +0x1F4, auxiliary u32 +0x04 and
/// three byte-counted buffers at +0x08 / +0x48 / +0x88. Completion: 100% of the bounded modern branch data.
/// TODO: [ ] Original fixed arrays, reader callback and error buffer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamFileHeader {
    pub version: u32,
    pub user_version: u32,
    pub object_capacity: u32,
    pub auxiliary_count: u32,
    pub metadata: [Vec<u8>; 3],
    pub bytes_consumed: usize,
}

/// IDA: 00712C00 with 00748330 line, 006BE9D0 u32, 006BDED0 u8 and 006D7C20 raw reads. Completion: 100% of the
/// successful 14.0.0.3..14.0.0.5, native-endian, user 1..11 path.
/// TODO: [ ] Short-read mutations, buffer overflow, endian fallback and earlier version branches.
pub fn read_supported_stream_file_header(
    file: &[u8],
) -> Result<StreamFileHeader, StreamFileHeaderError> {
    use StreamFileHeaderErrorCode as Code;
    let fail = |code, byte_offset, original_error_code| StreamFileHeaderError {
        code,
        byte_offset,
        original_error_code,
    };
    let mut cursor = 0;
    let mut line = Vec::with_capacity(127);
    let mut complete = false;
    // 00748330 reads one byte at a time, drops CR and stops on LF or the 128-byte local buffer. This path needs the LF.
    while cursor < file.len() && line.len() < 127 {
        let value = file[cursor];
        cursor += 1;
        if value == 10 {
            complete = true;
            break;
        }
        if value != 13 {
            line.push(value);
        }
    }
    if !complete {
        return Err(fail(Code::Truncated, cursor, None));
    }
    let until_nul = line.split(|&byte| byte == 0).next().unwrap_or(&line);
    if !until_nul.windows(11).any(|window| window == b"File Format") {
        return Err(fail(Code::NotNif, cursor, Some(2)));
    }
    let read_u32 = |cursor: &mut usize| {
        let bytes: [u8; 4] = file.get(*cursor..*cursor + 4)?.try_into().ok()?;
        *cursor += 4;
        Some(u32::from_le_bytes(bytes))
    };
    let read_u8 = |cursor: &mut usize| {
        let value = *file.get(*cursor)?;
        *cursor += 1;
        Some(value)
    };
    let version = read_u32(&mut cursor).ok_or_else(|| fail(Code::Truncated, cursor, None))?;
    // Bound versions: 14000003..14000005, 0A020000 (three preload models', metalSparks and others), 0A01006A (the
    // prison's cobwebs and SkyDome01) and 0A000100 (the FaceGen ears, EarsHuman.nif).
    if !(0x1400_0003..=0x1400_0005).contains(&version)
        && version != 0x0A02_0000
        && version != 0x0A01_006A
        && version != 0x0A00_0100
    {
        return Err(fail(Code::UnsupportedVersion, cursor, None));
    }
    // 00712CCF: the endian byte is 14000003's, and below that it's taken as 1.
    if version >= 0x1400_0003 {
        let endian = read_u8(&mut cursor).ok_or_else(|| fail(Code::Truncated, cursor, None))?;
        if endian != 1 {
            return Err(fail(Code::UnsupportedEndian, cursor, None));
        }
    }
    // 00712D0A: the user version is read from 0A000108 (0A000105 is then forced to 1); below it the 00712930
    // constructor's +0xDC, 11, stays.
    let user_version = if version >= 0x0A00_0108 {
        read_u32(&mut cursor).ok_or_else(|| fail(Code::Truncated, cursor, None))?
    } else {
        11
    };
    // B26DF8 stores maximum 11; B3FB88 reads zero in IDA but is mutable. Positive values enter the modern branch.
    if user_version == 0 || user_version > 11 {
        return Err(fail(
            Code::UnsupportedUserVersion,
            cursor,
            (user_version > 11).then_some(4),
        ));
    }
    let object_capacity =
        read_u32(&mut cursor).ok_or_else(|| fail(Code::Truncated, cursor, None))?;
    // 00712D6D: the aux u32 and the three metadata fields only above 0A000100 and (to 0A000108 or with a nonzero user
    // version); the aux u32 only above 0A000101 (0A000102 is then rewritten to 0A000101).
    let has_metadata = version > 0x0A00_0100 && (version <= 0x0A00_0108 || user_version != 0);
    let auxiliary_count = if has_metadata && version > 0x0A00_0101 {
        read_u32(&mut cursor).ok_or_else(|| fail(Code::Truncated, cursor, None))?
    } else {
        0
    };
    let mut header = StreamFileHeader {
        version,
        user_version,
        object_capacity,
        auxiliary_count,
        ..Default::default()
    };
    // 00712E85..00712ED4: three u8 lengths, each followed by that many raw bytes, in 64-byte fields.
    for field in &mut header.metadata {
        if !has_metadata {
            break;
        }
        let width =
            usize::from(read_u8(&mut cursor).ok_or_else(|| fail(Code::Truncated, cursor, None))?);
        if width > 64 {
            return Err(fail(Code::UnsupportedMetadataWidth, cursor, None));
        }
        let bytes = file
            .get(cursor..cursor + width)
            .ok_or_else(|| fail(Code::Truncated, cursor, None))?;
        field.extend_from_slice(bytes);
        cursor += width;
    }
    header.bytes_consumed = cursor;
    Ok(header)
}

/// IDA: 007135C0's or 00712930's owner array with the stores of 007135C0's tail. Slots hold host identities.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamOwnerArray {
    pub owner_slots: Vec<u32>,
    pub owner_count: u32,
    pub owner_high_water: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamTerminalState {
    pub owners: StreamOwnerArray,
    pub field_22c: u32,
    pub field_23c: u32,
    pub field_230: u32,
    pub field_240: u32,
}

/// IDA: 007135C0 enters lpCriticalSection__7, publishes B3FC7C / B3FC78, releases owners, then restores the nesting
/// and thread state. Exclusive `&mut` replaces the lock; the order of the published stores stays observable.
#[derive(Default)]
pub struct StreamTerminalServices<'a> {
    pub enter_critical_section: Option<&'a mut dyn FnMut()>,
    pub current_thread_id: Option<&'a mut dyn FnMut() -> u32>,
    pub release_owner: Option<&'a mut dyn FnMut(u32)>,
    pub leave_critical_section: Option<&'a mut dyn FnMut()>,
    pub nesting_count: u32,
    pub owning_thread_id: u32,
}

// IDA: N/A no address of its own; its comments cite 008BCC50.
fn clear_owners(
    owners: &mut StreamOwnerArray,
    services: &mut StreamTerminalServices<'_>,
    message: &'static str,
    tail: impl FnOnce(),
) -> Result<(), GameError> {
    if services.enter_critical_section.is_none()
        || services.current_thread_id.is_none()
        || services.leave_critical_section.is_none()
        || owners.owner_count as usize > owners.owner_slots.len()
    {
        return Err(GameError::from_legacy_message(message));
    }
    // The original calls InterlockedDecrement only for a nonnull slot. This preflight comes before the lock so an
    // unresolved release service cannot leave it held.
    if services.release_owner.is_none()
        && owners.owner_slots[..owners.owner_count as usize]
            .iter()
            .any(|&slot| slot != 0)
    {
        return Err(GameError::unclassified(
            0,
            "missing stream owner release service",
        ));
    }
    (services
        .enter_critical_section
        .as_mut()
        .expect("checked above"))();
    let thread_id = (services.current_thread_id.as_mut().expect("checked above"))();
    services.nesting_count += 1;
    services.owning_thread_id = thread_id;
    // 008BCC50 keeps capacity; each nonnull slot is released and cleared before the next.
    for slot in &mut owners.owner_slots[..owners.owner_count as usize] {
        if *slot != 0 {
            (services.release_owner.as_mut().expect("checked above"))(*slot);
            *slot = 0;
        }
    }
    owners.owner_high_water = 0;
    owners.owner_count = 0;
    tail();
    services.nesting_count -= 1;
    if services.nesting_count == 0 {
        services.owning_thread_id = 0;
    }
    (services
        .leave_critical_section
        .as_mut()
        .expect("checked above"))();
    Ok(())
}

/// IDA: 00712930 NiStream_ClearPreloadOwners, called by 00714390 after the cursor resets and before the >= 05000001
/// type-table gate. Completion: 100% of the normal order.
/// TODO: [ ] Interlocked owner release and failure behaviour.
pub fn clear_stream_preload_owners(
    state: &mut StreamOwnerArray,
    services: &mut StreamTerminalServices<'_>,
) -> Result<(), GameError> {
    clear_owners(
        state,
        services,
        "incomplete stream pre-load services or owner array",
        || {},
    )
}

/// IDA: 007135C0 NiStream_ClearTerminalOwners through 008BCC50: lock, thread publication, owner releases, four stores,
/// nesting withdrawal, conditional thread clear, unlock. Completion: 100% of the normal order.
/// TODO: [ ] InterlockedDecrement, the deleting destructor, and exceptional cleanup.
pub fn clear_stream_terminal_owners(
    state: &mut StreamTerminalState,
    services: &mut StreamTerminalServices<'_>,
) -> Result<(), GameError> {
    let StreamTerminalState {
        owners,
        field_22c,
        field_23c,
        field_230,
        field_240,
    } = state;
    clear_owners(
        owners,
        services,
        "incomplete stream terminal services or owner array",
        || {
            *field_22c = 0;
            *field_23c = 0;
            *field_230 = 0;
            *field_240 = 0;
        },
    )
}

/// IDA: 00714442's host observer of a constructed owner.
pub type PublishObserver<'a> = dyn FnMut(usize, &StreamObjectEntry) + 'a;
pub type PublishLoadedObject<'a> = &'a mut PublishObserver<'a>;

/// IDA: 007143A6 / AC / B2 reset +0x270 / +0x26C / +0x268 in that order; the passes increment them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamFileCursors {
    pub next_post_load: u32,
    pub next_link: u32,
    pub next_load: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamFilePipelineErrorStage {
    Header,
    UnavailablePauseService,
    Graph,
    PostLoad,
    Callback,
    Cancelled,
    /// A C++ `invalid_argument` from the post-load, callback or owner passes.
    Stream,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamFilePipelineError {
    pub stage: StreamFilePipelineErrorStage,
    pub byte_offset: usize,
    pub header: Option<Box<StreamFileHeaderError>>,
    pub graph: Option<Box<StreamGraphError>>,
    pub post_load: Option<Box<StreamPostLoadResult>>,
    pub callback: Option<Box<StreamRootCallbackResult>>,
    pub stream: Option<GameError>,
}

impl StreamFilePipelineError {
    // IDA: N/A a helper, with no address of its own.
    const fn at(stage: StreamFilePipelineErrorStage, byte_offset: usize) -> Self {
        Self {
            stage,
            byte_offset,
            header: None,
            graph: None,
            post_load: None,
            callback: None,
            stream: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamFilePipelineSuccess {
    pub header: StreamFileHeader,
    pub graph: StreamGraph,
    pub bytes_consumed: usize,
}

pub struct StreamFilePipelineRuntime<'a> {
    pub preload: &'a mut StreamOwnerArray,
    pub terminal: &'a mut StreamTerminalState,
    pub owner_services: &'a mut StreamTerminalServices<'a>,
    pub cursors: &'a mut StreamFileCursors,
    pub control: &'a mut StreamRootCallbackControl<'a>,
    pub link_runtime: StreamGraphLinkRuntime<'a>,
    pub post_load_runtime: StreamPostLoadRuntime<'a>,
    pub callback_runtime: StreamRootCallbackRuntime<'a>,
    /// IDA: 00714442 publishes each constructed owner before 00714453 increments +0x268. Host observer.
    pub publish_loaded_object: Option<PublishLoadedObject<'a>>,
}

struct FileProgress<'r, 'a> {
    control: &'r mut StreamRootCallbackControl<'a>,
    cursors: &'r mut StreamFileCursors,
    publish: Option<&'r mut PublishObserver<'a>>,
}

impl FileProgress<'_, '_> {
    // IDA: N/A a helper, with no address of its own.
    fn pause(&mut self) -> StreamGraphProgressDecision {
        if self.control.load_state == 3 {
            let Some(wait) = self.control.wait_for_resume.as_mut() else {
                return StreamGraphProgressDecision::UnavailablePauseService;
            };
            self.control.load_state = 4;
            wait(&mut self.control.load_state);
        }
        StreamGraphProgressDecision::Proceed
    }
}

impl StreamGraphProgress for FileProgress<'_, '_> {
    // IDA: see the words here, which name the addresses this relates to.
    // 00714400..00714429 checks state 2 before each LoadBinary; state 3 stores 4, then 00748810 waits.
    fn before_load(&mut self, _index: usize) -> StreamGraphProgressDecision {
        if self.control.load_state == 2 {
            return StreamGraphProgressDecision::Cancelled;
        }
        self.pause()
    }
    // IDA: see the words here, which name the addresses this relates to.
    // 00714442 returns before +0x268 increments at 00714453, so this owner publishes first.
    fn after_load(&mut self, index: usize, entry: &StreamObjectEntry) {
        if let Some(publish) = self.publish.as_mut() {
            publish(index, entry);
        }
        self.cursors.next_load += 1;
    }
    // IDA: see the words here, which name the addresses this relates to.
    // 00714478..00714495 waits on state 3 before LinkObject, with no state 2 exit in the link loop.
    fn before_link(&mut self, _index: usize) -> StreamGraphProgressDecision {
        self.pause()
    }
    // IDA: see the words here, which name the addresses this relates to.
    // 007144AA virtual +0x20 completes before +0x26C increments, no-op links included.
    fn after_link(&mut self, _index: usize) {
        self.cursors.next_link += 1;
    }
}

/// IDA: 00714390 NiStream_LoadObjectsAndInvokeCallbacks: +0x34 header, three cursor resets, 00712930, the type table,
/// group, payload, root and link passes, PostLoad, root callbacks and 007135C0. Completion: 100% of the uninterrupted
/// modern stage order.
/// TODO: [ ] Live owner-array publication, wait and event returns, failure cleanup, and the reader and allocator.
pub fn load_supported_stream_file(
    world: &mut World,
    registry: &NiFactoryRegistry,
    file: &[u8],
    runtime: StreamFilePipelineRuntime<'_>,
) -> Result<StreamFilePipelineSuccess, StreamFilePipelineError> {
    use StreamFilePipelineErrorStage as Stage;
    let StreamFilePipelineRuntime {
        preload,
        terminal,
        owner_services,
        cursors,
        control,
        mut link_runtime,
        mut post_load_runtime,
        mut callback_runtime,
        publish_loaded_object,
    } = runtime;
    let stream_failure = |offset, error| StreamFilePipelineError {
        stream: Some(error),
        ..StreamFilePipelineError::at(Stage::Stream, offset)
    };
    let header =
        read_supported_stream_file_header(file).map_err(|error| StreamFilePipelineError {
            header: Some(Box::new(error)),
            ..StreamFilePipelineError::at(Stage::Header, error.byte_offset)
        })?;

    // 007143A6 / AC / B2 writes +0x270, +0x26C, then +0x268.
    cursors.next_post_load = 0;
    cursors.next_link = 0;
    cursors.next_load = 0;
    clear_stream_preload_owners(preload, owner_services)
        .map_err(|error| stream_failure(header.bytes_consumed, error))?;

    // 006C8820 reads the sequence priority byte only when NiStream +0x04 is nonzero.
    link_runtime.stream_auxiliary_04 = Some(header.auxiliary_count);
    let built = {
        let mut progress = FileProgress {
            control,
            cursors,
            publish: publish_loaded_object,
        };
        load_stream_graph_body(
            world,
            registry,
            &file[header.bytes_consumed..],
            header.object_capacity,
            header.version,
            &mut link_runtime,
            Some(&mut progress),
        )
    };
    let graph = match built {
        Ok(graph) => graph,
        Err(error) => {
            let offset = header.bytes_consumed
                + error.body_byte_offset.unwrap_or_else(|| {
                    error
                        .table_error
                        .as_ref()
                        .map_or(0, |table| table.byte_offset)
                });
            let stage = match error.code {
                StreamGraphErrorCode::Cancelled => {
                    clear_stream_terminal_owners(terminal, owner_services)
                        .map_err(|inner| stream_failure(offset, inner))?;
                    Stage::Cancelled
                }
                StreamGraphErrorCode::UnavailablePauseService => Stage::UnavailablePauseService,
                _ => Stage::Graph,
            };
            return Err(StreamFilePipelineError {
                graph: Some(Box::new(error)),
                ..StreamFilePipelineError::at(stage, offset)
            });
        }
    };
    let consumed = header.bytes_consumed + graph.body_bytes_consumed;

    let mut next_post_load = cursors.next_post_load as usize;
    let finalized = finalize_stream_graph(
        world,
        &graph,
        header.version,
        &mut next_post_load,
        control,
        &mut post_load_runtime,
        &mut callback_runtime,
    );
    cursors.next_post_load = next_post_load as u32;
    // A failure past this point drops `graph`, whose C++ `shared_ptr` owners go with it.
    let finalized = match finalized {
        Ok(finalized) => finalized,
        Err(error) => {
            despawn_stream_graph(world, &graph);
            return Err(stream_failure(consumed, error));
        }
    };
    match finalized {
        StreamFinalizeResult::PostLoad(post) => {
            if post.status != StreamPostLoadStatus::Complete {
                despawn_stream_graph(world, &graph);
                return Err(StreamFilePipelineError {
                    post_load: Some(Box::new(post)),
                    ..StreamFilePipelineError::at(Stage::PostLoad, consumed)
                });
            }
        }
        StreamFinalizeResult::Callbacks(callbacks) => match callbacks.status {
            StreamRootCallbackStatus::UnsupportedCallback => {
                despawn_stream_graph(world, &graph);
                return Err(StreamFilePipelineError {
                    callback: Some(Box::new(callbacks)),
                    ..StreamFilePipelineError::at(Stage::Callback, consumed)
                });
            }
            StreamRootCallbackStatus::Cancelled => {
                despawn_stream_graph(world, &graph);
                clear_stream_terminal_owners(terminal, owner_services)
                    .map_err(|error| stream_failure(consumed, error))?;
                return Err(StreamFilePipelineError {
                    callback: Some(Box::new(callbacks)),
                    ..StreamFilePipelineError::at(Stage::Cancelled, consumed)
                });
            }
            StreamRootCallbackStatus::Complete => {}
        },
    }
    // 007126A0 is gated below 0401000C, outside this branch. 007135C0 clears the +0x1EC owner array.
    if let Err(error) = clear_stream_terminal_owners(terminal, owner_services) {
        despawn_stream_graph(world, &graph);
        return Err(stream_failure(consumed, error));
    }
    Ok(StreamFilePipelineSuccess {
        header,
        graph,
        bytes_consumed: consumed,
    })
}
