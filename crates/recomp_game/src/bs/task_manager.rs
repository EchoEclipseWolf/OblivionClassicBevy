//! Phase 09 (D1/D3): the task infrastructure TES_Constructor 00441970 builds: the two LockFreeMap task singletons (004BE8A0 -> 00B35B90, 004BDCD0 ->
//! 00B35B8C), the IOManager (00434480 -> 00B33A10, a BSTaskManager<__int64> with one BSTaskManagerThread) and the ModelLoader (0043E950 -> 00B33A1C,
//! five LockFree maps, the distant-3D queue and the BackgroundCloneThread).
//!
//! D3: both worker threads are created as the original does (two semaphores each, a suspended start, then ResumeThread) and block in their run loops
//! (00433BC0 / 0043CC20) on the +0x14 semaphore. P09-RUN-001 record 669 shows both waiting there at the window end. A wake (00431FF0's, 00431F50's) only sends them back to the wait: host substitution, 00433BC0's walk runs on the main thread (`model_loader::ModelDrain`, Phase 13e), because a worker can't hold `World`. No arithmetic runs on these threads.
//!
//! Each is a real `std::thread` holding only an `Arc<TaskThreadState>`, never `World`, on the Phase 06 `MovieThreadHandle` and `HostSemaphore`.
//! Host substitution: the original never stops these threads. Here `BsTaskThread::stop_and_join` sets the stop flag, resumes a thread that is still
//! suspended, releases +0x14 and joins. `Drop` calls it, so dropping `TaskGlobals` (or the `World` holding it) cannot leak or hang a thread.
//!
//! Host substitutions for values: object storage is host memory; thread IDs are `std::thread::ThreadId`; TlsAlloc has no host index (`LockFreeTlsLock` keeps the
//! registered threads instead); 00B33A08 holds 1e9 because no host timer reads it in this window.
//!
//! Phase 13 adds the worker pause 00432860 / 00432890, which 00442890 reaches from the main thread. 13e's drain takes the +0x20 token per pass as 00433C37 does, and gives it back as 00433D20 does. The host run loops themselves never hold it. The clone thread's side of the token is unported: nothing queues a clone.
//! Evidence: evidence/phase09-window-2026-10-06/disasm-09c.txt, P09-RUN-001.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::model_loader::{IoTaskMap, MODEL_MAP_BUCKETS, ModelMap, TaskCounters};
use crate::tes::movie_player::{HostSemaphore, MovieThreadHandle};
use crate::tes::os_globals::lock_shared;

/// IDA: vtables of the task objects (P09-RUN-001 raw bytes).
pub const LOCK_FREE_MAP_EXTERIOR_CELL_TASK_VTABLE: u32 = 0x00A4_5C0C;
pub const LOCK_FREE_MAP_DISTANT_LOD_TASK_VTABLE: u32 = 0x00A4_5B1C;
pub const LOCK_FREE_MAP_BS_TASK_VTABLE: u32 = 0x00A3_6514;
pub const BS_TASK_MANAGER_VTABLE: u32 = 0x00A3_6554;
pub const IO_MANAGER_VTABLE: u32 = 0x00A3_65AC;
pub const LOCK_FREE_QUEUE_IO_TASK_VTABLE: u32 = 0x00A3_6500;
pub const BS_TASK_THREAD_VTABLE: u32 = 0x00A3_63FC;
pub const BS_TASK_MANAGER_THREAD_VTABLE: u32 = 0x00A3_6460;
pub const BACKGROUND_CLONE_THREAD_VTABLE: u32 = 0x00A3_7088;
pub const LOCK_FREE_MAP_MODEL_VTABLE: u32 = 0x00A3_6FCC;
pub const LOCK_FREE_CASE_INSENSITIVE_MAP_MODEL_VTABLE: u32 = 0x00A3_7194;
pub const LOCK_FREE_MAP_KF_MODEL_VTABLE: u32 = 0x00A3_700C;
pub const LOCK_FREE_CASE_INSENSITIVE_MAP_KF_MODEL_VTABLE: u32 = 0x00A3_71DC;
pub const LOCK_FREE_MAP_QUEUED_REFERENCE_VTABLE: u32 = 0x00A3_6EF4;
pub const LOCK_FREE_MAP_QUEUED_ANIM_IDLE_VTABLE: u32 = 0x00A3_6F34;
pub const LOCK_FREE_MAP_QUEUED_HELMET_VTABLE: u32 = 0x00A3_6F74;
pub const LOCK_FREE_QUEUE_ATTACH_DISTANT_3D_VTABLE: u32 = 0x00A3_6FB4;
pub const LOCK_FREE_QUEUE_QUEUED_REFERENCE_VTABLE: u32 = 0x00A3_6EE0;

/// IDA: 00435E70 LockFreeTlsLock(count) (0x10 bytes): +0x00 count, +0x04 TlsAlloc() index, +0x08 count 8-byte Concurrency::_ReaderWriterLock slots
/// (zeroed by their constructor), +0x0C = 0. Completion: 100% of the stores; the index is a host stand-in, since no lock is taken yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockFreeTlsLock {
    pub count_00: u32,
    pub tls_index_04: Option<u32>,
    pub reader_writer_locks_08: Vec<[u32; 2]>,
    pub field_0c: u32,
    /// IDA: N/A what TlsGetValue(+0x04) answers: the threads with a slot, in slot order.
    registered: Vec<ThreadId>,
}

impl LockFreeTlsLock {
    // IDA: 00435E70: 00435EA7 +0x0C = 0; 00435EAE +0x00 = count; 00435EB5 FormHeap(8 * count) with the _ReaderWriterLock constructor per slot -> +0x08;
    // 00435EE4 +0x04 = TlsAlloc(). Completion: 100%.
    #[must_use]
    pub fn new(count: u32) -> Self {
        Self {
            count_00: count,
            tls_index_04: None,
            reader_writer_locks_08: vec![[0; 2]; count as usize],
            field_0c: 0,
            registered: Vec::new(),
        }
    }

    /// IDA: `register_thread` for the calling thread.
    /// # Errors
    /// No free slot.
    pub fn register_current_thread(&mut self) -> Result<(), StreamError> {
        self.register_thread(std::thread::current().id())
    }

    // IDA: TlsGetValue(+0x04) and, on a thread's first use, 00431D60 AddInterface: InterlockedIncrement(+0x0C) - 1 is the slot. Below +0x00, the map's vt+0 makes the interface, 00431DA6 stores it at slot + 4, 00431DB3 the thread ID at slot + 0, and TlsSetValue. At or past it, the original prints "Could not add new interface..." and calls DebugBreak (00431DEA): a host stop. The slot holds host values: {slot + 1, 1}. The drain, on the main thread for the IO thread, passes the IO thread's ID, so the slots count the threads the original has. Completion: 100% of the slot order.
    /// # Errors
    /// No free slot.
    pub fn register_thread(&mut self, current: ThreadId) -> Result<(), StreamError> {
        if self.registered.contains(&current) {
            return Ok(());
        }
        let slot = self.field_0c;
        self.field_0c += 1;
        if slot >= self.count_00 {
            return Err(StreamError(
                "00431DEA: ThreadSpecificInterfaceManager::AddInterface has no free slot",
            ));
        }
        self.reader_writer_locks_08[slot as usize] = [slot + 1, 1];
        self.registered.push(current);
        Ok(())
    }
}

/// IDA: the LockFreeMap<K, V> constructor family 004BE200 / 004BD050 / 00432520 / 00438930 / 00438A30 / 00438B20 / 00438DC0 / 00438F10 / 0055E720
/// (0x1C bytes): +0x00 vtable, +0x18 = 0, +0x08 bucket count, +0x0C buckets (4 * count, each zeroed by 004BDDB0), +0x04 entry storage (12 * entries,
/// uninitialized), +0x10 the third argument (12), +0x14 LockFreeTlsLock(entries). The case-insensitive string maps then store their own vtable.
/// Completion: 100% of the store order and values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockFreeMap {
    pub original_vtable: u32,
    /// IDA: N/A a host view of the 12-byte +0x04 storage size.
    pub entry_count: u32,
    pub bucket_count_08: u32,
    pub buckets_0c: Vec<u32>,
    pub field_10: u32,
    pub lock_14: LockFreeTlsLock,
    pub field_18: u32,
}

impl LockFreeMap {
    // IDA: 004BE237 vtable; 004BE23D +0x18 = 0; 004BE244 +0x08 = buckets; 004BE24C FormHeap(4 * buckets), 004BDDB0 zeroes each slot -> +0x0C; 004BE29B
    // FormHeap(12 * entries) -> +0x04; 004BE2A9 +0x10 = the third argument; 004BE2AC..004BE2D0 FormHeap(0x10) 00435E70(entries) -> +0x14. Completion: 100%.
    #[must_use]
    pub fn new(vtable: u32, entries: u32, buckets: u32, argument_10: u32) -> Self {
        Self {
            original_vtable: vtable,
            entry_count: entries,
            bucket_count_08: buckets,
            buckets_0c: vec![0; buckets as usize],
            field_10: argument_10,
            lock_14: LockFreeTlsLock::new(entries),
            field_18: 0,
        }
    }
}

/// IDA: the LockFreeQueue<T> constructor family 00432400 / 00438CF0 / 00438870 (0x1C bytes): +0x00 vtable, +0x18 = 0, an 8-byte sentinel node {0, 0} at
/// +0x04 (head) and +0x08 (tail), +0x0C the second argument (8), +0x10 storage (8 * count, uninitialized), +0x14 LockFreeTlsLock(count).
/// Completion: 100% of the store order and values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockFreeQueue {
    pub original_vtable: u32,
    /// IDA: the node +0x04/+0x08 point at.
    pub sentinel: [u32; 2],
    pub field_0c: u32,
    /// IDA: N/A a host view of the +0x10 slots.
    pub storage_count: u32,
    pub lock_14: LockFreeTlsLock,
    pub field_18: u32,
}

impl LockFreeQueue {
    // IDA: 00432424 vtable; 0043242C +0x18 = 0; 00432433 the {0, 0} sentinel node -> +0x04 and +0x08; 0043245E +0x0C = the second argument; 00432475
    // FormHeap(8 * count) -> +0x10; 0043247F 00435E70(count) -> +0x14. Completion: 100%.
    #[must_use]
    pub fn new(vtable: u32, count: u32, argument_0c: u32) -> Self {
        Self {
            original_vtable: vtable,
            sentinel: [0, 0],
            field_0c: argument_0c,
            storage_count: count,
            lock_14: LockFreeTlsLock::new(count),
            field_18: 0,
        }
    }
}

/// IDA: N/A one call of the constructors below, in original order, with its site. A worker's own calls are in `TaskThreadState::wait_trace`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskCall {
    /// CreateSemaphoreA(0, initial, maximum, 0).
    CreateSemaphore {
        site: u32,
        initial: i32,
        maximum: i32,
    },
    /// CreateThread(0, 0, 00430DE0, this, CREATE_SUSPENDED, +0x08).
    CreateThread { site: u32, kind: u32 },
    /// 00430DF0 ResumeThread(+0x04).
    ResumeThread { site: u32, kind: u32 },
    /// QueryPerformanceFrequency(00B33A08).
    QueryPerformanceFrequency { site: u32 },
}

/// IDA: what a worker and its creator share: BSTaskThread +0x0C..+0x20, plus the host's stop flag and the worker's own view.
pub struct TaskThreadState {
    pub counter_0c: AtomicI32,
    pub maximum_10: i32,
    pub semaphore_14: HostSemaphore,
    pub counter_18: AtomicI32,
    pub maximum_1c: i32,
    pub semaphore_20: HostSemaphore,
    stop_requested: AtomicBool,
    waiting: AtomicBool,
    /// IDA: N/A the run loop's WaitForSingleObject(+0x14, INFINITE) calls, by site.
    wait_trace: Mutex<Vec<u32>>,
}

impl std::fmt::Debug for TaskThreadState {
    /// IDA: N/A Rust's `Debug`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TaskThreadState")
    }
}

// IDA: the first statement of both run loops: WaitForSingleObject(+0x14, INFINITE) and, unless WAIT_TIMEOUT, InterlockedDecrement(+0x0C). False when the
// host stop flag ends the loop. Completion: 100% of the wait.
fn wait_for_work(state: &TaskThreadState, site: u32) -> bool {
    lock_shared(&state.wait_trace).push(site);
    state.waiting.store(true, Ordering::Release);
    state.semaphore_14.acquire();
    state.counter_0c.fetch_sub(1, Ordering::AcqRel);
    state.waiting.store(false, Ordering::Release);
    !state.stop_requested.load(Ordering::Acquire)
}

// IDA: 00433BC0: waits on +0x14, then walks the manager's task map. Host substitution (the plan's "Worker threads cannot hold World"): the body past the wait is `model_loader::ModelDrain` on the main thread, so a wake only returns here. Completion: 100% of the wait; the body is in `ModelDrain`.
fn bs_task_manager_thread_run(state: &TaskThreadState) {
    while wait_for_work(state, 0x0043_3BC0) {}
}

// IDA: 0043CC20: waits on +0x14, then takes a queued reference (00432400's queue, 0043C030) and clones it (0043AF30). No phase queues one, so a wake finds the queue empty and waits again, as 00442890's pause wakes it (00431F50). Completion: 100% of the wait, 0% of the body; nothing runs on this thread.
// TODO: [ ] Cloning and the token handshake, once a phase queues references (after M5).
fn background_clone_thread_run(state: &TaskThreadState) {
    while wait_for_work(state, 0x0043_CC20) {}
}

/// IDA: 00430E00 BSTaskThread_Constructor(this, kind, name) (0x28 base): the kind and name arguments are unused; +0x00 vtable A363FC, +0x0C = 0, +0x10 =
/// 1, +0x14 CreateSemaphoreA(0, +0x0C, 1, 0), +0x18 = 1, +0x1C = 1, +0x20 CreateSemaphoreA(0, +0x18, 1, 0), +0x04 CreateThread(0, 0, 00430DE0, this,
/// CREATE_SUSPENDED, +0x08). 00430DE0 calls vtable +4 (the run loop). The derived constructors then store their vtable. Completion: 100%.
#[derive(Debug)]
pub struct BsTaskThread {
    pub original_vtable: u32,
    handle_04: Option<MovieThreadHandle>,
    /// IDA: +0x08, a host substitution for the DWORD.
    pub thread_id_08: ThreadId,
    state: Arc<TaskThreadState>,
    /// IDA: N/A the unused constructor arguments, kept for the record.
    pub kind: u32,
    pub debug_name: &'static str,
}

impl BsTaskThread {
    // IDA: 00430E00, with `run` as the vtable +4 the derived constructor installs. A CreateThread failure stores a null handle in the original and
    // 00430DF0 passes it on; the host stops. Completion: 100%.
    fn new(
        kind: u32,
        name: &'static str,
        vtable: u32,
        run: fn(&TaskThreadState),
        trace: &mut Vec<TaskCall>,
    ) -> Result<Self, StreamError> {
        // 00430E12 +0x0C = 0; 00430E21 +0x10 = 1; 00430E28 the +0x14 semaphore (initial +0x0C, maximum 1).
        trace.push(TaskCall::CreateSemaphore {
            site: 0x0043_0E28,
            initial: 0,
            maximum: 1,
        });
        // 00430E2F +0x18 = 1; 00430E3E +0x1C = 1; 00430E45 the +0x20 semaphore.
        trace.push(TaskCall::CreateSemaphore {
            site: 0x0043_0E45,
            initial: 1,
            maximum: 1,
        });
        let state = Arc::new(TaskThreadState {
            counter_0c: AtomicI32::new(0),
            maximum_10: 1,
            semaphore_14: HostSemaphore::new(0),
            counter_18: AtomicI32::new(1),
            maximum_1c: 1,
            semaphore_20: HostSemaphore::new(1),
            stop_requested: AtomicBool::new(false),
            waiting: AtomicBool::new(false),
            wait_trace: Mutex::new(Vec::new()),
        });
        // 00430E5A CreateThread(0, 0, 00430DE0, this, CREATE_SUSPENDED, +0x08).
        trace.push(TaskCall::CreateThread {
            site: 0x0043_0E5A,
            kind,
        });
        let thread_state = Arc::clone(&state);
        let handle = MovieThreadHandle::spawn_suspended(name, move || run(&thread_state))
            .map_err(|_| StreamError("00430E5A: CreateThread failed"))?;
        let thread_id_08 = handle
            .thread_id()
            .ok_or(StreamError("00430E5A: no thread ID"))?;
        Ok(Self {
            original_vtable: vtable,
            handle_04: Some(handle),
            thread_id_08,
            state,
            kind,
            debug_name: name,
        })
    }

    // IDA: 00430DF0 ResumeThread(+0x04). Completion: 100%.
    pub fn resume(&self, trace: &mut Vec<TaskCall>) {
        trace.push(TaskCall::ResumeThread {
            site: 0x0043_0DF0,
            kind: self.kind,
        });
        if let Some(handle) = &self.handle_04 {
            handle.resume();
        }
    }

    // IDA: N/A host shutdown (host substitution: the original never stops these threads): set the stop flag, resume a thread still suspended so it can
    // see it, release the +0x14 semaphore, join. Idempotent.
    pub fn stop_and_join(&mut self) {
        let Some(handle) = self.handle_04.take() else {
            return;
        };
        self.state.stop_requested.store(true, Ordering::Release);
        handle.resume();
        self.state.semaphore_14.release();
        handle.wait();
    }

    // IDA: the shape 00432056, 004322EA and 00431F54 share: when +0x0C reads 0, InterlockedIncrement(+0x0C) and ReleaseSemaphore(+0x14, 1). Completion: 100%.
    pub(crate) fn wake(&self) {
        if self.state.counter_0c.load(Ordering::Acquire) == 0 {
            self.state.counter_0c.fetch_add(1, Ordering::AcqRel);
            self.state.semaphore_14.release();
        }
    }

    // IDA: 00431F50 (and 004322E4..0043232C per thread): `wake`, then WaitForSingleObject(+0x20, INFINITE); unless WAIT_TIMEOUT, InterlockedDecrement(+0x18). Completion: 100%.
    pub(crate) fn pause(&self) {
        self.wake();
        self.state.semaphore_20.acquire();
        self.state.counter_18.fetch_sub(1, Ordering::AcqRel);
    }

    // IDA: 00431FA0's body past its thread test (and 00432394..004323D3 per thread): `wake`, InterlockedIncrement(+0x18), ReleaseSemaphore(+0x20, 1). Completion: 100%.
    pub(crate) fn resume_after_pause(&self) {
        self.wake();
        self.state.counter_18.fetch_add(1, Ordering::AcqRel);
        self.state.semaphore_20.release();
    }

    /// IDA: N/A the run loop has reached its first wait.
    #[must_use]
    pub fn waiting(&self) -> bool {
        self.state.waiting.load(Ordering::Acquire)
    }

    // IDA: 00433C37 WaitForSingleObject(+0x20, 0), the token a pass of the walk holds. False is WAIT_TIMEOUT: a pause holds it. Completion: 100%. A taken token does not touch +0x18 (00433C43 `jne 00433C82`).
    pub(crate) fn try_take_walk_token(&self) -> bool {
        self.state.semaphore_20.try_acquire()
    }

    // IDA: 00433C4B..00433C55 the timeout arm once the wait returns: InterlockedDecrement(+0x18). The wait itself is the caller's. Completion: 100%.
    pub(crate) fn count_waited_walk_token(&self) {
        self.state.counter_18.fetch_sub(1, Ordering::AcqRel);
    }

    // IDA: 00433D1F InterlockedIncrement(+0x18) and 00433D2D ReleaseSemaphore(+0x20, 1, 0), at the end of every pass. With 00433C43's skip, +0x18 rises by one on each pass that took the token at once. Completion: 100%.
    pub(crate) fn give_back_walk_token(&self) {
        self.state.counter_18.fetch_add(1, Ordering::AcqRel);
        self.state.semaphore_20.release();
    }

    /// IDA: N/A the sites of the run loop's waits, in order.
    #[must_use]
    pub fn wait_trace(&self) -> Vec<u32> {
        lock_shared(&self.state.wait_trace).clone()
    }

    /// IDA: N/A the state the worker shares.
    #[must_use]
    pub fn state(&self) -> &TaskThreadState {
        &self.state
    }

    /// IDA: N/A whether the host joined it.
    #[must_use]
    pub const fn joined(&self) -> bool {
        self.handle_04.is_none()
    }
}

impl Drop for BsTaskThread {
    /// IDA: N/A Rust's `Drop`.
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

/// IDA: 00439C10 BackgroundCloneThread_Constructor(queueCapacity) (0x2C): the BSTaskThread base (kind 3, "BackgroundCloneThread"), vtable A37088, +0x24 =
/// 0, +0x28 LockFreeQueue<NiPointer<QueuedReference>>(capacity, 8). Run loop 0043CC20. Completion: 100% of the stores.
#[derive(Debug)]
pub struct BackgroundCloneThread {
    pub thread: BsTaskThread,
    pub field_24: u32,
    pub reference_queue_28: LockFreeQueue,
}

impl BackgroundCloneThread {
    // IDA: 00439C10. 00439C46 vtable A37088; 00439C56 +0x24 = 0; 00439C5D..00439C84 the queue -> +0x28. Completion: 100%.
    fn new(queue_capacity: u32, trace: &mut Vec<TaskCall>) -> Result<Self, StreamError> {
        let thread = BsTaskThread::new(
            3,
            "BackgroundCloneThread",
            BACKGROUND_CLONE_THREAD_VTABLE,
            background_clone_thread_run,
            trace,
        )?;
        Ok(Self {
            thread,
            field_24: 0,
            reference_queue_28: LockFreeQueue::new(
                LOCK_FREE_QUEUE_QUEUED_REFERENCE_VTABLE,
                queue_capacity,
                8,
            ),
        })
    }
}

/// IDA: 00434480 IOManager_Constructor (0x3C) over 00434230 BSTaskManager<__int64>(entries, threads, buckets).
#[derive(Debug)]
pub struct IoManager {
    pub task_map: LockFreeMap,
    pub original_vtable: u32,
    pub field_1c: u32,
    pub field_20: u32,
    pub thread_count_24: u32,
    pub threads_28: Vec<BsTaskThread>,
    pub array_2c: Vec<u32>,
    /// IDA: +0x30 = the first thread's +0x08.
    pub main_thread_id_30: ThreadId,
    pub io_queue_34: LockFreeQueue,
    pub field_38: u32,
    /// IDA: N/A the task map's entries (Phase 13).
    pub tasks: IoTaskMap,
}

impl IoManager {
    // IDA: 00434480: 00434230(2, 1, 18): 0043426D 00432520(threads + entries, buckets, 12); 00434282 vtable A36554; 00434288 +0x1C = 0; 0043428F +0x20 = 0;
    // 00434292 +0x24 = thread count; 0043429A..004342AC FormHeap(4 * buckets) zeroed -> +0x2C; 004342C5 FormHeap(4 * threads) -> +0x28; 004342D7..00434328 per
    // thread: FormHeap(0x28) 00434300 BSTaskManagerThread(index + 2), stored, then 00430DF0. Then 004344BF vtable A365AC; 004344C5 +0x38 = 6; 004344CC..004344F5
    // LockFreeQueue<NiPointer<IOTask>>(3, 8) -> +0x34; 004344F8 QueryPerformanceFrequency(00B33A08); 004344FE..00434506 +0x30 = threads[0] +0x08. Completion: 100%.
    fn new(frequency: &mut i64, trace: &mut Vec<TaskCall>) -> Result<Self, StreamError> {
        let (entries, thread_count, buckets) = (2_u32, 1_u32, 18_u32);
        let task_map = LockFreeMap::new(
            LOCK_FREE_MAP_BS_TASK_VTABLE,
            thread_count + entries,
            buckets,
            12,
        );
        let mut threads_28 = Vec::with_capacity(thread_count as usize);
        for index in 0..thread_count {
            // 00434300 vtable A36460; 00434306 +0x24 = the manager, which `IoManager` owning the threads stands for.
            let thread = BsTaskThread::new(
                index + 2,
                "BSTaskManagerThread",
                BS_TASK_MANAGER_THREAD_VTABLE,
                bs_task_manager_thread_run,
                trace,
            )?;
            thread.resume(trace);
            threads_28.push(thread);
        }
        let io_queue_34 = LockFreeQueue::new(LOCK_FREE_QUEUE_IO_TASK_VTABLE, 3, 8);
        trace.push(TaskCall::QueryPerformanceFrequency { site: 0x0043_44F8 });
        *frequency = 1_000_000_000;
        // 00434501: no null check in the original.
        let main_thread_id_30 = threads_28
            .first()
            .ok_or(StreamError(
                "00434501: the IOManager has no first task thread",
            ))?
            .thread_id_08;
        Ok(Self {
            task_map,
            original_vtable: IO_MANAGER_VTABLE,
            field_1c: 0,
            field_20: 0,
            thread_count_24: thread_count,
            threads_28,
            array_2c: vec![0; buckets as usize],
            main_thread_id_30,
            io_queue_34,
            field_38: 6,
            tasks: IoTaskMap {
                store: Vec::new(),
                buckets: vec![Vec::new(); buckets as usize],
            },
        })
    }

    // IDA: 004322B0(this): InterlockedIncrement(+0x20); only the result 1 goes on. Sleep(1) until +0x1C reads 0; then per thread, by +0x24: 00431F50's shape, and InterlockedIncrement(+0x1C). The host's +0x1C moves only on this thread, so the wait can't spin. Completion: 100%.
    fn pause_threads(&mut self) -> bool {
        self.field_20 += 1;
        if self.field_20 != 1 {
            return false;
        }
        for thread in &self.threads_28 {
            thread.pause();
            self.field_1c += 1;
        }
        true
    }

    // IDA: 00432350(this): +0x20 reading 0 returns 0; InterlockedDecrement(+0x20) not reaching 0 returns 0. Sleep(1) until +0x1C reads +0x24; then per thread 00431FA0's body, and InterlockedDecrement(+0x1C). Completion: 100%.
    fn resume_threads(&mut self) -> bool {
        if self.field_20 == 0 {
            return false;
        }
        self.field_20 -= 1;
        if self.field_20 != 0 {
            return false;
        }
        for thread in &self.threads_28 {
            thread.resume_after_pause();
            self.field_1c -= 1;
        }
        true
    }
}

/// IDA: 0043E950 ModelLoader_Constructor (0x1C).
#[derive(Debug)]
pub struct ModelLoader {
    pub model_map_00: LockFreeMap,
    pub kf_map_04: LockFreeMap,
    pub reference_map_08: LockFreeMap,
    pub idle_map_0c: LockFreeMap,
    pub helmet_map_10: LockFreeMap,
    pub distant_3d_queue_14: LockFreeQueue,
    pub clone_thread_18: BackgroundCloneThread,
    /// IDA: N/A the model map's entries (Phase 13).
    pub models: ModelMap,
    /// IDA: N/A +0x04's entries, the KF files (`kf_model`).
    pub kfs: super::kf_model::KfMap,
    /// IDA: N/A +0x08's reference tasks and the clone thread's and finish queues.
    pub references: super::reference_tasks::ReferenceTasks,
}

impl ModelLoader {
    // IDA: 0043E950: 0043E976..0043E9B5 the model map 00438DC0(4, 1009, 12), then the case-insensitive vtable A37194; 0043E9B7..0043E9F1 the KF map, vtable
    // A371DC; 0043E9F4..0043E9B5 the reference, idle and helmet maps (3, 37, 12); 0043EA8A..0043EAB7 the distant-3D queue (3, 8); 0043EABA..0043EAE5
    // BackgroundCloneThread(3) -> +0x18; 0043EAE8 resume, through the pointer without a null check. Completion: 100% of the order.
    fn new(trace: &mut Vec<TaskCall>) -> Result<Self, StreamError> {
        let mut model_map_00 = LockFreeMap::new(LOCK_FREE_MAP_MODEL_VTABLE, 4, 1009, 12);
        model_map_00.original_vtable = LOCK_FREE_CASE_INSENSITIVE_MAP_MODEL_VTABLE;
        let mut kf_map_04 = LockFreeMap::new(LOCK_FREE_MAP_KF_MODEL_VTABLE, 4, 1009, 12);
        kf_map_04.original_vtable = LOCK_FREE_CASE_INSENSITIVE_MAP_KF_MODEL_VTABLE;
        let reference_map_08 = LockFreeMap::new(LOCK_FREE_MAP_QUEUED_REFERENCE_VTABLE, 3, 37, 12);
        let idle_map_0c = LockFreeMap::new(LOCK_FREE_MAP_QUEUED_ANIM_IDLE_VTABLE, 3, 37, 12);
        let helmet_map_10 = LockFreeMap::new(LOCK_FREE_MAP_QUEUED_HELMET_VTABLE, 3, 37, 12);
        let distant_3d_queue_14 =
            LockFreeQueue::new(LOCK_FREE_QUEUE_ATTACH_DISTANT_3D_VTABLE, 3, 8);
        let clone_thread_18 = BackgroundCloneThread::new(3, trace)?;
        clone_thread_18.thread.resume(trace);
        Ok(Self {
            model_map_00,
            kf_map_04,
            reference_map_08,
            idle_map_0c,
            helmet_map_10,
            distant_3d_queue_14,
            clone_thread_18,
            models: ModelMap {
                store: Vec::new(),
                buckets: vec![Vec::new(); MODEL_MAP_BUCKETS as usize],
            },
            kfs: super::kf_model::KfMap::default(),
            references: super::reference_tasks::ReferenceTasks::default(),
        })
    }
}

/// IDA: the task globals Phase 09 publishes. Dropping this joins both workers.
#[derive(Resource, Debug, Default)]
pub struct TaskGlobals {
    /// 00B35B90 (004BE8A0).
    pub exterior_cell_tasks_b35b90: Option<LockFreeMap>,
    /// 00B35B8C (004BDCD0).
    pub distant_lod_tasks_b35b8c: Option<LockFreeMap>,
    /// 00B33A10.
    pub io_manager_b33a10: Option<IoManager>,
    /// 00B33A1C.
    pub model_loader_b33a1c: Option<ModelLoader>,
    /// 00B33A08 (a host value: nothing reads it in this window).
    pub performance_frequency_b33a08: i64,
    /// IDA: N/A the constructors' thread and semaphore calls, in order.
    pub trace: Vec<TaskCall>,
    /// 00B33A14, 00B33A18, 00B33A20.
    pub counters: TaskCounters,
}

impl TaskGlobals {
    // IDA: 00441B01..00441B2A FormHeap(0x3C) 00434480 -> 00B33A10. Completion: 100%.
    pub fn construct_io_manager(&mut self) -> Result<(), StreamError> {
        self.io_manager_b33a10 = Some(IoManager::new(
            &mut self.performance_frequency_b33a08,
            &mut self.trace,
        )?);
        Ok(())
    }

    // IDA: 00441B23..00441B59 FormHeap(0x1C) 0043E950 -> 00B33A1C. Completion: 100%.
    pub fn construct_model_loader(&mut self) -> Result<(), StreamError> {
        self.model_loader_b33a1c = Some(ModelLoader::new(&mut self.trace)?);
        Ok(())
    }

    // IDA: 00432860(g_IOManager): 004322B0; true, then g_modelLoader +0x18's thread unless it's the caller (00431F50). Completion: 100%.
    // Errors: a null g_IOManager or g_modelLoader, which the original dereferences.
    pub fn pause_workers(&mut self) -> Result<(), StreamError> {
        let io = self
            .io_manager_b33a10
            .as_mut()
            .ok_or(StreamError("00432860: a null g_IOManager"))?;
        if io.pause_threads() {
            let clone = &self
                .model_loader_b33a1c
                .as_ref()
                .ok_or(StreamError("00432869: a null g_modelLoader"))?
                .clone_thread_18
                .thread;
            if clone.thread_id_08 != std::thread::current().id() {
                clone.pause();
            }
        }
        Ok(())
    }

    // IDA: 00432890(g_IOManager): 00432350; true, then 00431FA0 on g_modelLoader +0x18's thread, whose own test skips the caller. Completion: 100%.
    pub fn resume_workers(&mut self) -> Result<(), StreamError> {
        let io = self
            .io_manager_b33a10
            .as_mut()
            .ok_or(StreamError("00432890: a null g_IOManager"))?;
        if io.resume_threads() {
            let clone = &self
                .model_loader_b33a1c
                .as_ref()
                .ok_or(StreamError("00432899: a null g_modelLoader"))?
                .clone_thread_18
                .thread;
            if clone.thread_id_08 != std::thread::current().id() {
                clone.resume_after_pause();
            }
        }
        Ok(())
    }

    // IDA: N/A host shutdown: stop and join both worker threads (D3). `Drop` does this too; call it for an explicit order.
    pub fn stop_worker_threads(&mut self) {
        if let Some(loader) = &mut self.model_loader_b33a1c {
            loader.clone_thread_18.thread.stop_and_join();
        }
        if let Some(io) = &mut self.io_manager_b33a10 {
            for thread in &mut io.threads_28 {
                thread.stop_and_join();
            }
        }
    }

    // IDA: 004BE8A0: when 00B35B90 is null, FormHeap(0x1C) 004BE200(2, 37, 12). Completion: 100%.
    pub fn create_exterior_cell_task_map(&mut self) {
        if self.exterior_cell_tasks_b35b90.is_none() {
            self.exterior_cell_tasks_b35b90 = Some(LockFreeMap::new(
                LOCK_FREE_MAP_EXTERIOR_CELL_TASK_VTABLE,
                2,
                37,
                12,
            ));
        }
    }

    // IDA: 004BDCD0: when 00B35B8C is null, FormHeap(0x1C) 004BD050(2, 37, 12). Completion: 100%.
    pub fn create_distant_lod_task_map(&mut self) {
        if self.distant_lod_tasks_b35b8c.is_none() {
            self.distant_lod_tasks_b35b8c = Some(LockFreeMap::new(
                LOCK_FREE_MAP_DISTANT_LOD_TASK_VTABLE,
                2,
                37,
                12,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 004322B0 and 00432350: a nested pause only counts; the outer pair moves the tokens. No worker holds one on the host.
    #[test]
    fn a_pause_takes_each_token_and_the_resume_gives_it_back() {
        let mut globals = TaskGlobals::default();
        globals.construct_io_manager().unwrap();
        globals.construct_model_loader().unwrap();
        globals.pause_workers().unwrap();
        globals.pause_workers().unwrap();
        let tokens = |globals: &TaskGlobals| {
            (
                globals.io_manager_b33a10.as_ref().unwrap().threads_28[0]
                    .state()
                    .counter_18
                    .load(Ordering::Acquire),
                globals
                    .model_loader_b33a1c
                    .as_ref()
                    .unwrap()
                    .clone_thread_18
                    .thread
                    .state()
                    .counter_18
                    .load(Ordering::Acquire),
            )
        };
        let io_state = |globals: &TaskGlobals| {
            let io = globals.io_manager_b33a10.as_ref().unwrap();
            (io.field_1c, io.field_20)
        };
        assert_eq!(tokens(&globals), (0, 0));
        assert_eq!(io_state(&globals), (1, 2));
        globals.resume_workers().unwrap();
        assert_eq!(tokens(&globals), (0, 0), "still paused");
        assert_eq!(io_state(&globals), (1, 1));
        globals.resume_workers().unwrap();
        assert_eq!(tokens(&globals), (1, 1));
        assert_eq!(io_state(&globals), (0, 0));
        globals.resume_workers().unwrap();
        assert_eq!(io_state(&globals), (0, 0), "00432356: +0x20 reads 0");
        globals.stop_worker_threads();
    }

    // 00431D60: slots go in order, and past the count is a stop.
    #[test]
    fn a_lock_gives_each_thread_one_slot() {
        let mut lock = LockFreeTlsLock::new(1);
        lock.register_current_thread().unwrap();
        lock.register_current_thread().unwrap();
        assert_eq!(lock.field_0c, 1);
        assert_eq!(lock.reader_writer_locks_08[0], [1, 1]);
        let mut full = LockFreeTlsLock::new(0);
        assert!(full.register_current_thread().is_err());
    }

    #[test]
    fn shutdown_joins_a_suspended_and_a_waiting_thread() {
        let mut trace = Vec::new();
        // Never resumed: the host resumes it to see the stop.
        let mut suspended = BackgroundCloneThread::new(3, &mut trace).unwrap();
        suspended.thread.stop_and_join();
        assert!(suspended.thread.joined());
        let mut globals = TaskGlobals::default();
        globals.construct_io_manager().unwrap();
        globals.construct_model_loader().unwrap();
        globals.stop_worker_threads();
        globals.stop_worker_threads();
        let io = globals.io_manager_b33a10.as_ref().unwrap();
        assert!(io.threads_28[0].joined());
        assert_eq!(io.threads_28[0].wait_trace(), [0x0043_3BC0]);
        assert_eq!(
            globals.trace[..4],
            [
                TaskCall::CreateSemaphore {
                    site: 0x0043_0E28,
                    initial: 0,
                    maximum: 1
                },
                TaskCall::CreateSemaphore {
                    site: 0x0043_0E45,
                    initial: 1,
                    maximum: 1
                },
                TaskCall::CreateThread {
                    site: 0x0043_0E5A,
                    kind: 2
                },
                TaskCall::ResumeThread {
                    site: 0x0043_0DF0,
                    kind: 2
                },
            ]
        );
    }
}
