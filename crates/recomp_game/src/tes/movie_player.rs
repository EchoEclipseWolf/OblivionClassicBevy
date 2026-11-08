//! Phase 06 rows 06-C-1..06-C-6: the MoviePlayer of WinMain 0040EADB..0040EAEA (00410310 Bink sound, 00410E40 the
//! asynchronous intro sequence) and the MoviePlayer thread 00410E20 with its playback and tile helpers. Runtime evidence:
//! P06-RUN-001, P06-RUN-002.
//!
//! The thread is a real `std::thread`. It never holds `World`: everything it reaches is in `MoviePlayer`, a clone of two
//! `Arc`s. The 00B33424.. globals are atomics; movie objects, the input and TimeInfo are behind the small locks in `Shared`;
//! device calls leave through `MovieDevice`, and `QueuedMovieDevice` puts them on a queue for the main thread.
//!
//! x87: the thread's control word stays 027F (P06-RUN-002 `T 9`, `MOVIE_THREAD_CONTROL_WORD`), so its float operations are 53-bit: `f64` intermediates and `stored_float` at each `fstp m32`. The main thread runs at 007F (`S 3`), so 0040F0BE's play and 0057F3BC's present are 24-bit: each operation rounds to float. `movie_x87` says which, and `movie_prepare`, `draw_movie_quad` and `draw_movie_tiles` follow it. 00410840 is reached only through 00410E25's thread (00410DE6, the one xref), so it stays 53-bit.
//!
//! Recorded host substitutions: atomics and locks for the cross-thread globals, which the original reads and writes
//! unsynchronized; `Arc` for the two FormHeap objects' lifetimes; `video_directory` for the relative "Data\Video\" root;
//! the seam for D3DXCreateTextureFromFileA; `MoviePlatform` for the two log sinks; a suspended thread is a `std::thread`
//! blocked on `HostSemaphore`; `MovieThreadHandle` keeps the 00B33438 priority's value but no scheduler sees it; a
//! `std::thread` name for 00747830 SetThreadNameForDebugger; a tile's pixels are bytes the seam receives at UnlockRect.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{JoinHandle, ThreadId};

use bevy_ecs::prelude::*;
use recomp_core::crt::stricmp;
use recomp_core::host_path;
use recomp_core::logging;
use recomp_core::x87::{ftol64_low_f64, stored_float};

use super::bink_library::{BinkHandle, BinkLibrary};
use super::interface_manager::{InterfaceMovieReads, MENU_ID_MESSAGE};
use super::os_globals::{
    CreatedInterface, GameSoundPause, OsGlobalsData, OsGlobalsPlatform, OsInputGlobals,
    SharedOsInput, lock_shared,
};
use super::settings::SettingCollection;
use super::window::SharedTimeInfo;
use crate::gamebryo::dx9_system::{
    D3D_OK, D3DERR_NOTAVAILABLE, D3DFMT_UNKNOWN, D3DFMT_X8R8G8B8, D3DPOOL_DEFAULT, D3DPOOL_MANAGED,
};

/// IDA: P06-RUN-002's `ctrl=027F` on the thread. Evidence, read by the row check; nothing sets a control word here.
pub const MOVIE_THREAD_CONTROL_WORD: u16 = 0x027F;
/// IDA: 00410216 BINKPRELOADALL.
pub const BINK_PRELOAD_ALL: u32 = 0x2000;
/// IDA: 0041024C, off the main thread.
pub const BINK_NO_THREADED_IO: u32 = 0x0800_0000;
/// IDA: 0040FCEA BINKSURFACE32.
pub const BINK_SURFACE32: u32 = 3;
/// IDA: 0040FCE4 BINKCOPYALL.
pub const BINK_COPY_ALL: u32 = 0x8000_0000;
/// IDA: 0040F853 D3DFVF_XYZRHW | D3DFVF_TEX1.
pub const MOVIE_QUAD_FVF: u32 = 0x104;
/// IDA: 00410749.
pub const MOVIE_CLEAR_COLOR: u32 = 0xFF00_0000;
/// IDA: 0041074E D3DCLEAR_TARGET.
pub const D3DCLEAR_TARGET: u32 = 1;
/// IDA: 0040F6C6 D3DUSAGE_DYNAMIC.
pub const D3DUSAGE_DYNAMIC: u32 = 0x200;
/// IDA: 0040F86C D3DPT_TRIANGLESTRIP.
pub const D3DPT_TRIANGLESTRIP: u32 = 5;
/// IDA: `+0x20 == 2`.
pub const MOVIE_STATE_MENU_OWNED: u32 = 2;
/// IDA: 0041045E, 0041046D.
const CONTROL_ACTIVATE: i32 = 5;
const CONTROL_ESCAPE: i32 = 29;
/// IDA: 0041059C, 004105A9.
const TILE_SIZE: u32 = 256;
const TILE_BYTES_PER_PIXEL: u32 = 4;
/// `WaitForSingleObject`'s result.
pub const WAIT_OBJECT_0: u32 = 0;

/// IDA: N/A a counting semaphore: the suspended start of a `std::thread`, and what a test waits on instead of sleeping.
/// Completion: 100% of its host role.
pub struct HostSemaphore {
    count: Mutex<u32>,
    ready: Condvar,
}

impl HostSemaphore {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub const fn new(initial: u32) -> Self {
        Self {
            count: Mutex::new(initial),
            ready: Condvar::new(),
        }
    }

    /// IDA: N/A a helper, with no address of its own.
    pub fn release(&self) {
        *lock_shared(&self.count) += 1;
        self.ready.notify_one();
    }

    /// IDA: N/A WaitForSingleObject(handle, 0): true on WAIT_OBJECT_0, false where the original gets WAIT_TIMEOUT.
    pub fn try_acquire(&self) -> bool {
        let mut count = lock_shared(&self.count);
        if *count == 0 {
            return false;
        }
        *count -= 1;
        true
    }

    /// IDA: N/A a helper, with no address of its own.
    pub fn acquire(&self) {
        let mut count = lock_shared(&self.count);
        while *count == 0 {
            count = self
                .ready
                .wait(count)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *count -= 1;
    }
}

struct MovieThreadState {
    start: HostSemaphore,
    suspended: AtomicBool,
    finished: Mutex<bool>,
    finished_signal: Condvar,
    priority: AtomicI32,
    join: Mutex<Option<JoinHandle<()>>>,
}

/// IDA: N/A marks the thread finished however its body ends. The handle is signaled from then on, as a Win32 thread's.
struct FinishGuard(Arc<MovieThreadState>);

impl Drop for FinishGuard {
    /// IDA: N/A Rust's `Drop`.
    fn drop(&mut self) {
        *lock_shared(&self.0.finished) = true;
        self.0.finished_signal.notify_all();
    }
}

/// IDA: N/A a Win32 thread handle over a `std::thread`: created suspended, then CreateThread's flag 4 is the semaphore. A
/// clone names the same thread. A thread never resumed stays blocked until the process exits, as a suspended one would.
#[derive(Clone)]
pub struct MovieThreadHandle(Arc<MovieThreadState>);

impl std::fmt::Debug for MovieThreadHandle {
    /// IDA: N/A Rust's `Debug`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MovieThreadHandle")
    }
}

impl MovieThreadHandle {
    /// IDA: N/A `CreateThread(NULL, 0, body, arg, CREATE_SUSPENDED, &id)`, named `name`.
    /// # Errors
    /// The host could not start a thread, where CreateThread returns null.
    pub fn spawn_suspended(
        name: &str,
        body: impl FnOnce() + Send + 'static,
    ) -> std::io::Result<Self> {
        let state = Arc::new(MovieThreadState {
            start: HostSemaphore::new(0),
            suspended: AtomicBool::new(true),
            finished: Mutex::new(false),
            finished_signal: Condvar::new(),
            priority: AtomicI32::new(0),
            join: Mutex::new(None),
        });
        let thread_state = Arc::clone(&state);
        let join = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let guard = FinishGuard(thread_state);
                guard.0.start.acquire();
                body();
            })?;
        *lock_shared(&state.join) = Some(join);
        Ok(Self(state))
    }

    /// IDA: N/A CreateThread's thread ID out parameter, while the thread is not yet joined. A host value.
    #[must_use]
    pub fn thread_id(&self) -> Option<std::thread::ThreadId> {
        lock_shared(&self.0.join)
            .as_ref()
            .map(|join| join.thread().id())
    }

    /// IDA: N/A ResumeThread: the previous suspend count.
    pub fn resume(&self) -> u32 {
        if self.0.suspended.swap(false, Ordering::AcqRel) {
            self.0.start.release();
            1
        } else {
            0
        }
    }

    /// IDA: N/A GetExitCodeThread(...) == STILL_ACTIVE; a suspended thread is still active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        !*lock_shared(&self.0.finished)
    }

    /// IDA: N/A WaitForSingleObject(handle, INFINITE).
    pub fn wait(&self) -> u32 {
        let mut finished = lock_shared(&self.0.finished);
        while !*finished {
            finished = self
                .0
                .finished_signal
                .wait(finished)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        drop(finished);
        if let Some(join) = lock_shared(&self.0.join).take() {
            // A body's panic has already reported itself and set the finished flag.
            let _ = join.join();
        }
        WAIT_OBJECT_0
    }

    /// IDA: N/A SetThreadPriority: the value is kept, and no scheduler sees it (recorded).
    pub fn set_priority(&self, value: i32) {
        self.0.priority.store(value, Ordering::Release);
    }

    /// IDA: N/A GetThreadPriority.
    #[must_use]
    pub fn priority(&self) -> i32 {
        self.0.priority.load(Ordering::Acquire)
    }

    /// IDA: N/A handle identity.
    #[must_use]
    pub fn same_thread(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// IDA: N/A a host IDirect3DTexture9. Never compared with evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MovieTexture(pub u32);

/// IDA: 0040F760's 24-byte XYZRHW | TEX1 vertex.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovieVertex {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rhw: f32,
    pub u: f32,
    pub v: f32,
}

/// IDA: N/A one device call, with what the code passed. Arguments not listed are the original's constants.
#[derive(Clone, Debug, PartialEq)]
pub enum MovieDeviceCall {
    TestCooperativeLevel,
    CreateTexture {
        width: u32,
        height: u32,
        levels: u32,
        usage: u32,
        format: u32,
        pool: u32,
    },
    /// D3DXCreateTextureFromFileA.
    CreateTextureFromFile {
        path: PathBuf,
    },
    GetLevelDesc {
        texture: MovieTexture,
    },
    LockRect {
        texture: MovieTexture,
    },
    UnlockRect {
        texture: MovieTexture,
    },
    ReleaseTexture {
        texture: MovieTexture,
    },
    Clear {
        count: u32,
        flags: u32,
        color: u32,
        z_bits: u32,
        stencil: u32,
    },
    BeginScene,
    EndScene,
    Present,
    SetRenderState {
        state: u32,
        value: u32,
    },
    SetTextureStageState {
        stage: u32,
        state: u32,
        value: u32,
    },
    SetSamplerState {
        sampler: u32,
        state: u32,
        value: u32,
    },
    SetVertexShaderNull,
    SetPixelShaderNull,
    SetFvf {
        fvf: u32,
    },
    SetTexture {
        stage: u32,
        texture: Option<MovieTexture>,
    },
    DrawPrimitiveUp {
        primitive: u32,
        count: u32,
        stride: u32,
        vertices: [MovieVertex; 4],
    },
}

/// IDA: N/A one trace row: the original call site and the call.
#[derive(Clone, Debug, PartialEq)]
pub struct MovieDeviceEntry {
    pub site: u32,
    pub call: MovieDeviceCall,
}

/// IDA: N/A D3DXCreateTextureFromFileA's texture and the GetLevelDesc size 00410933 reads back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplashTexture {
    pub texture: MovieTexture,
    pub width: u32,
    pub height: u32,
}

/// IDA: N/A the seam for g_renderer+0x280, the device the thread presents on. `site` is the original call site. Nothing here
/// touches wgpu; the host answers it. Completion: 100% of the calls the thread makes.
pub trait MovieDevice: Send {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// A call whose only result is its HRESULT.
    fn call(&mut self, site: u32, call: MovieDeviceCall) -> u32;
    /// IDA: see the words here, which name the addresses this relates to.
    #[allow(clippy::too_many_arguments)] // 0040F6D8 CreateTexture's own arguments.
    fn create_texture(
        &mut self,
        site: u32,
        width: u32,
        height: u32,
        levels: u32,
        usage: u32,
        format: u32,
        pool: u32,
    ) -> CreatedInterface<MovieTexture>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// UnlockRect, with what the code wrote between LockRect and here.
    fn unlock_rect(&mut self, site: u32, texture: MovieTexture, bytes: &[u8]) -> u32;
    /// IDA: see the words here, which name the addresses this relates to.
    /// D3DXCreateTextureFromFileA and 00410933's GetLevelDesc.
    fn create_texture_from_file(
        &mut self,
        site: u32,
        path: &Path,
    ) -> CreatedInterface<SplashTexture>;
}

/// IDA: N/A the seam's script (the C++ tests' hidden D3D9 device): every result, and the ordered trace.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptedMovieDevice {
    pub trace: Vec<MovieDeviceEntry>,
    /// CreateTexture fails for these, as a real device fails D3DFMT_UNKNOWN.
    pub failed_formats: Vec<u32>,
    pub cooperative_level: u32,
    pub lock_result: u32,
    /// What D3DXCreateTextureFromFileA loads; `None` fails as a missing file does.
    pub splash_size: Option<(u32, u32)>,
    next_texture: u32,
}

impl Default for ScriptedMovieDevice {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            trace: Vec::new(),
            failed_formats: vec![D3DFMT_UNKNOWN],
            cooperative_level: D3D_OK,
            lock_result: D3D_OK,
            splash_size: None,
            next_texture: 1,
        }
    }
}

impl MovieDevice for ScriptedMovieDevice {
    /// IDA: N/A `ScriptedMovieDevice`'s side of `MovieDevice`.
    fn call(&mut self, site: u32, call: MovieDeviceCall) -> u32 {
        let result = match call {
            MovieDeviceCall::TestCooperativeLevel => self.cooperative_level,
            MovieDeviceCall::LockRect { .. } => self.lock_result,
            _ => D3D_OK,
        };
        self.trace.push(MovieDeviceEntry { site, call });
        result
    }
    /// IDA: N/A `ScriptedMovieDevice`'s side of `MovieDevice`.
    fn create_texture(
        &mut self,
        site: u32,
        width: u32,
        height: u32,
        levels: u32,
        usage: u32,
        format: u32,
        pool: u32,
    ) -> CreatedInterface<MovieTexture> {
        self.trace.push(MovieDeviceEntry {
            site,
            call: MovieDeviceCall::CreateTexture {
                width,
                height,
                levels,
                usage,
                format,
                pool,
            },
        });
        if self.failed_formats.contains(&format) {
            return CreatedInterface {
                result: D3DERR_INVALIDCALL,
                object: None,
            };
        }
        let texture = MovieTexture(self.next_texture);
        self.next_texture += 1;
        CreatedInterface {
            result: D3D_OK,
            object: Some(texture),
        }
    }
    /// IDA: N/A `ScriptedMovieDevice`'s side of `MovieDevice`.
    fn unlock_rect(&mut self, site: u32, texture: MovieTexture, _bytes: &[u8]) -> u32 {
        self.trace.push(MovieDeviceEntry {
            site,
            call: MovieDeviceCall::UnlockRect { texture },
        });
        D3D_OK
    }
    /// IDA: N/A `ScriptedMovieDevice`'s side of `MovieDevice`.
    fn create_texture_from_file(
        &mut self,
        site: u32,
        path: &Path,
    ) -> CreatedInterface<SplashTexture> {
        self.trace.push(MovieDeviceEntry {
            site,
            call: MovieDeviceCall::CreateTextureFromFile {
                path: path.to_path_buf(),
            },
        });
        match self.splash_size {
            Some((width, height)) => {
                let texture = MovieTexture(self.next_texture);
                self.next_texture += 1;
                CreatedInterface {
                    result: D3D_OK,
                    object: Some(SplashTexture {
                        texture,
                        width,
                        height,
                    }),
                }
            }
            None => CreatedInterface {
                result: D3DERR_NOTAVAILABLE,
                object: None,
            },
        }
    }
}

/// IDA: N/A one message from the thread to whoever owns the real device. `tile_bytes` is a tile's pixels, at UnlockRect, or the decoded image at CreateTextureFromFile.
#[derive(Clone, Debug, PartialEq)]
pub struct MovieQueueEntry {
    pub entry: MovieDeviceEntry,
    pub tile_bytes: Option<Vec<u8>>,
    /// The texture and size CreateTextureFromFile answered.
    pub splash: Option<SplashTexture>,
}

/// IDA: N/A a decoded image: the size and rows of BGRA8 that D3DXCreateTextureFromFileA would have put in a texture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedSplash {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// IDA: N/A D3DXCreateTextureFromFileA's file decode, which this crate can't do (host substitution): `recomp_host` supplies it. `None` fails the call as a missing file does.
pub type SplashLoader = std::sync::Arc<dyn Fn(&Path) -> Option<DecodedSplash> + Send + Sync>;

/// IDA: N/A the device as a queue: the thread never touches the host's device or `World`. Every call succeeds (P06-RUN-001
/// saw no failure), so a device failure branch needs a seam that answers. A closed queue drops the message: the host's
/// shutdown stops the thread first. Completion: 100% of the host role.
pub struct QueuedMovieDevice {
    sender: Sender<MovieQueueEntry>,
    next_texture: u32,
    splash_loader: Option<SplashLoader>,
}

impl QueuedMovieDevice {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub const fn new(sender: Sender<MovieQueueEntry>) -> Self {
        Self {
            sender,
            next_texture: 1,
            splash_loader: None,
        }
    }

    /// IDA: N/A the same device with a file decoder for `create_texture_from_file`.
    #[must_use]
    pub fn with_splash_loader(mut self, loader: Option<SplashLoader>) -> Self {
        self.splash_loader = loader;
        self
    }

    // IDA: N/A a helper, with no address of its own.
    fn send(&self, site: u32, call: MovieDeviceCall, tile_bytes: Option<Vec<u8>>) {
        let _ = self.sender.send(MovieQueueEntry {
            entry: MovieDeviceEntry { site, call },
            tile_bytes,
            splash: None,
        });
    }

    // IDA: N/A a helper, with no address of its own.
    fn texture(&mut self) -> MovieTexture {
        let texture = MovieTexture(self.next_texture);
        self.next_texture += 1;
        texture
    }
}

impl MovieDevice for QueuedMovieDevice {
    /// IDA: N/A `QueuedMovieDevice`'s side of `MovieDevice`.
    fn call(&mut self, site: u32, call: MovieDeviceCall) -> u32 {
        self.send(site, call, None);
        D3D_OK
    }
    /// IDA: N/A `QueuedMovieDevice`'s side of `MovieDevice`.
    fn create_texture(
        &mut self,
        site: u32,
        width: u32,
        height: u32,
        levels: u32,
        usage: u32,
        format: u32,
        pool: u32,
    ) -> CreatedInterface<MovieTexture> {
        self.send(
            site,
            MovieDeviceCall::CreateTexture {
                width,
                height,
                levels,
                usage,
                format,
                pool,
            },
            None,
        );
        CreatedInterface {
            result: D3D_OK,
            object: Some(self.texture()),
        }
    }
    /// IDA: N/A `QueuedMovieDevice`'s side of `MovieDevice`.
    fn unlock_rect(&mut self, site: u32, texture: MovieTexture, bytes: &[u8]) -> u32 {
        self.send(
            site,
            MovieDeviceCall::UnlockRect { texture },
            Some(bytes.to_vec()),
        );
        D3D_OK
    }
    /// IDA: N/A `QueuedMovieDevice`'s side of `MovieDevice`.
    fn create_texture_from_file(
        &mut self,
        site: u32,
        path: &Path,
    ) -> CreatedInterface<SplashTexture> {
        let decoded = self.splash_loader.as_ref().and_then(|load| load(path));
        let call = MovieDeviceCall::CreateTextureFromFile {
            path: path.to_path_buf(),
        };
        let Some(decoded) = decoded else {
            self.send(site, call, None);
            return CreatedInterface {
                result: D3DERR_NOTAVAILABLE,
                object: None,
            };
        };
        let splash = SplashTexture {
            texture: self.texture(),
            width: decoded.width,
            height: decoded.height,
        };
        let _ = self.sender.send(MovieQueueEntry {
            entry: MovieDeviceEntry { site, call },
            tile_bytes: Some(decoded.bgra),
            splash: Some(splash),
        });
        CreatedInterface {
            result: D3D_OK,
            object: Some(splash),
        }
    }
}

/// IDA: N/A the seam for the Win32 calls and engine sinks the thread makes outside the device. `site` is the original call
/// site.
pub trait MoviePlatform: Send {
    /// IDA: see the words here, which name the addresses this relates to.
    /// 00401E20 / 00401F00 FormHeap: false is a null result.
    fn allocate(&mut self, site: u32, bytes: u32) -> bool;
    /// IDA: see the words here, which name the addresses this relates to.
    /// 00410394..004103DF the PeekMessageA loop of this thread's queue.
    fn pump_messages(&mut self, site: u32);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn get_current_thread_id(&mut self, site: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn suspend_thread(&mut self, site: u32, handle: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn resume_thread(&mut self, site: u32, handle: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn sleep(&mut self, site: u32, milliseconds: u32);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn get_tick_count(&mut self, site: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// `_stat64i32`'s st_size at +0x14: `None` when it returns -1. The file is the caller's resolved path.
    fn file_size(&mut self, site: u32, path: &Path) -> Option<i32>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// g_dx9Renderer +0x200 == 1, a separate read at each site.
    fn renderer_in_scene(&mut self, site: u32) -> bool;
    /// IDA: see the words here, which name the addresses this relates to.
    /// 00768960 NiDX9RenderState_SetFVF per default-shader pass: P06-RUN-001 read 0 passes at startup.
    fn set_default_shader_pass_fvf(&mut self, site: u32, fvf: u32);
    /// IDA: see the words here, which name the addresses this relates to.
    /// 004A7A60 DebugMessage_Warning.
    fn warning(&mut self, text: &str);
    /// IDA: see the words here, which name the addresses this relates to.
    /// 0040FEC0 DebugLog_Level2Printf.
    fn debug_log(&mut self, text: &str);
}

/// IDA: N/A `_stat64i32(path)`'s st_size as the original compares it, a signed 32-bit value (00410206 `mov ecx, [esp+2Ch]`).
/// Completion: 100% below 2 GiB.
/// TODO: [ ] The CRT's result for a file of 2 GiB or more is unverified; the longest movie is 136 MB.
#[must_use]
pub fn stat_file_size(path: &Path) -> Option<i32> {
    std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.len() as i32)
}

/// IDA: N/A the running host's platform: the real clock, sleep and file size. The two thread calls fail like the startup
/// path's null main-thread handle (P06-RUN-001 `mainHandle=0`), and a real suspend has no portable form. Nothing pumps a
/// queue: the thread has no window. Completion: 100% of the host role.
pub struct HostMoviePlatform {
    start: std::time::Instant,
    /// g_dx9Renderer +0x200, set by whoever owns the renderer. The movie thread's Begin/End run on its own at startup.
    pub in_scene: Arc<AtomicBool>,
}

impl Default for HostMoviePlatform {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            start: std::time::Instant::now(),
            in_scene: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// IDA: N/A GetCurrentThreadId for this host: a small stable number per thread. The OS seam's implementation must answer
/// this too, so that 0041022E compares like with like.
#[must_use]
pub fn host_thread_id() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    thread_local! {
        static ID: u32 = NEXT.fetch_add(1, Ordering::Relaxed);
    }
    ID.with(|id| *id)
}

impl MoviePlatform for HostMoviePlatform {
    /// IDA: N/A `HostMoviePlatform`'s side of `MoviePlatform`.
    fn allocate(&mut self, _site: u32, _bytes: u32) -> bool {
        true
    }
    /// IDA: N/A `HostMoviePlatform`'s side of `MoviePlatform`.
    fn pump_messages(&mut self, _site: u32) {}
    /// IDA: N/A `HostMoviePlatform`'s side of `MoviePlatform`.
    fn get_current_thread_id(&mut self, _site: u32) -> u32 {
        host_thread_id()
    }
    /// IDA: N/A `HostMoviePlatform`'s side of `MoviePlatform`.
    fn suspend_thread(&mut self, _site: u32, _handle: u32) -> u32 {
        u32::MAX
    }
    /// IDA: N/A `HostMoviePlatform`'s side of `MoviePlatform`.
    fn resume_thread(&mut self, _site: u32, _handle: u32) -> u32 {
        u32::MAX
    }
    /// IDA: N/A `HostMoviePlatform`'s side of `MoviePlatform`.
    fn sleep(&mut self, _site: u32, milliseconds: u32) {
        std::thread::sleep(std::time::Duration::from_millis(u64::from(milliseconds)));
    }
    /// IDA: N/A `HostMoviePlatform`'s side of `MoviePlatform`.
    fn get_tick_count(&mut self, _site: u32) -> u32 {
        self.start.elapsed().as_millis() as u32
    }
    /// IDA: N/A `HostMoviePlatform`'s side of `MoviePlatform`.
    fn file_size(&mut self, _site: u32, path: &Path) -> Option<i32> {
        stat_file_size(path)
    }
    /// IDA: N/A `HostMoviePlatform`'s side of `MoviePlatform`.
    fn renderer_in_scene(&mut self, _site: u32) -> bool {
        self.in_scene.load(Ordering::Acquire)
    }
    /// IDA: N/A `HostMoviePlatform`'s side of `MoviePlatform`.
    fn set_default_shader_pass_fvf(&mut self, _site: u32, _fvf: u32) {}
    /// IDA: N/A `HostMoviePlatform`'s side of `MoviePlatform`.
    fn warning(&mut self, text: &str) {
        logging::emit(logging::Level::Warning, text);
    }
    /// IDA: N/A `HostMoviePlatform`'s side of `MoviePlatform`.
    fn debug_log(&mut self, text: &str) {
        logging::emit(logging::Level::Debug, text);
    }
}

/// IDA: N/A the platform's script and its record, for tests.
pub struct ScriptedMoviePlatform {
    pub failed_allocations: Vec<u32>,
    pub allocations: Vec<u32>,
    pub pumps: u32,
    pub suspends: Vec<(u32, u32)>,
    pub resumes: Vec<(u32, u32)>,
    pub sleeps: Vec<u32>,
    /// Every GetTickCount advances the clock by `tick_step`.
    pub tick: u32,
    pub tick_step: u32,
    pub in_scene: bool,
    pub scene_reads: Vec<u32>,
    pub fvf_calls: Vec<u32>,
    pub warnings: Vec<String>,
    pub debug_logs: Vec<String>,
    /// GetCurrentThreadId is this on the thread that made the script, and `MAIN_THREAD_ID + 1` elsewhere.
    main_thread: ThreadId,
}

impl ScriptedMoviePlatform {
    pub const MAIN_THREAD_ID: u32 = 0x18F84;
}

impl Default for ScriptedMoviePlatform {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            failed_allocations: Vec::new(),
            allocations: Vec::new(),
            pumps: 0,
            suspends: Vec::new(),
            resumes: Vec::new(),
            sleeps: Vec::new(),
            tick: 0,
            tick_step: 1000,
            in_scene: false,
            scene_reads: Vec::new(),
            fvf_calls: Vec::new(),
            warnings: Vec::new(),
            debug_logs: Vec::new(),
            main_thread: std::thread::current().id(),
        }
    }
}

impl MoviePlatform for ScriptedMoviePlatform {
    /// IDA: N/A `ScriptedMoviePlatform`'s side of `MoviePlatform`.
    fn allocate(&mut self, _site: u32, bytes: u32) -> bool {
        self.allocations.push(bytes);
        !self.failed_allocations.contains(&bytes)
    }
    /// IDA: N/A `ScriptedMoviePlatform`'s side of `MoviePlatform`.
    fn pump_messages(&mut self, _site: u32) {
        self.pumps += 1;
    }
    /// IDA: N/A `ScriptedMoviePlatform`'s side of `MoviePlatform`.
    fn get_current_thread_id(&mut self, _site: u32) -> u32 {
        if std::thread::current().id() == self.main_thread {
            Self::MAIN_THREAD_ID
        } else {
            Self::MAIN_THREAD_ID + 1
        }
    }
    /// IDA: N/A `ScriptedMoviePlatform`'s side of `MoviePlatform`.
    fn suspend_thread(&mut self, site: u32, handle: u32) -> u32 {
        self.suspends.push((site, handle));
        u32::MAX
    }
    /// IDA: N/A `ScriptedMoviePlatform`'s side of `MoviePlatform`.
    fn resume_thread(&mut self, site: u32, handle: u32) -> u32 {
        self.resumes.push((site, handle));
        u32::MAX
    }
    /// IDA: N/A `ScriptedMoviePlatform`'s side of `MoviePlatform`.
    fn sleep(&mut self, _site: u32, milliseconds: u32) {
        self.sleeps.push(milliseconds);
    }
    /// IDA: N/A `ScriptedMoviePlatform`'s side of `MoviePlatform`.
    fn get_tick_count(&mut self, _site: u32) -> u32 {
        self.tick = self.tick.wrapping_add(self.tick_step);
        self.tick
    }
    /// IDA: N/A `ScriptedMoviePlatform`'s side of `MoviePlatform`.
    fn file_size(&mut self, _site: u32, path: &Path) -> Option<i32> {
        stat_file_size(path)
    }
    /// IDA: N/A `ScriptedMoviePlatform`'s side of `MoviePlatform`.
    fn renderer_in_scene(&mut self, site: u32) -> bool {
        self.scene_reads.push(site);
        self.in_scene
    }
    /// IDA: N/A `ScriptedMoviePlatform`'s side of `MoviePlatform`.
    fn set_default_shader_pass_fvf(&mut self, _site: u32, fvf: u32) {
        self.fvf_calls.push(fvf);
    }
    /// IDA: N/A `ScriptedMoviePlatform`'s side of `MoviePlatform`.
    fn warning(&mut self, text: &str) {
        self.warnings.push(text.to_owned());
    }
    /// IDA: N/A `ScriptedMoviePlatform`'s side of `MoviePlatform`.
    fn debug_log(&mut self, text: &str) {
        self.debug_logs.push(text.to_owned());
    }
}

/// IDA: 0040FEE0 MovieTileSet (FormHeap 0x48 + 4 * tileCount). 00410110 releases the textures in index order. The +0 device
/// has no field: every caller passes the seam.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MovieTileSet {
    pub format_04: u32,
    pub bytes_per_pixel_08: u32,
    pub width_0c: u32,
    pub height_10: u32,
    pub lock_index_14: u32,
    /// Zeroed twice, never read.
    pub unk_18: u32,
    /// (width * bpp + 15) & ~15.
    pub pitch_1c: u32,
    pub tile_count_20: u32,
    pub full_columns_24: u32,
    pub full_rows_28: u32,
    pub partial_width_2c: u32,
    pub partial_height_30: u32,
    pub width_remainder_34: u32,
    pub height_remainder_38: u32,
    pub tile_size_3c: u32,
    /// +0x40 -> +0x48, in tile order.
    pub tiles_40: Vec<MovieTile>,
}

/// IDA: N/A one tile: the texture's handle and size, and its pixels. The original's are device memory behind LockRect; here
/// the pitch is width * bpp, and `MovieDevice::unlock_rect` receives them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovieTile {
    pub texture: MovieTexture,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// IDA: N/A the out parameters of 0040F880.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockedMovieTile {
    pub index: usize,
    pub pitch: i32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// IDA: MoviePlayerMovie (FormHeap 0x28, no vtable) apart from `+0x20`.
#[derive(Clone, Debug, PartialEq)]
pub struct MovieFields {
    /// +0x00 HBINK.
    pub bink_00: Option<BinkHandle>,
    /// +0x04 g_dx9Renderer+0x280, by presence: the seam is the device.
    pub device_04: bool,
    pub tiles_08: Option<Box<MovieTileSet>>,
    pub frames_shown_0c: u32,
    pub skipped_frames_10: u32,
    pub scale_14: f32,
    pub x_18: f32,
    pub y_1c: f32,
    pub preparing_24: u8,
}

impl Default for MovieFields {
    /// IDA: 00410BDB..00410BF9: all 0 and 0.0, with +0x14 1.0.
    fn default() -> Self {
        Self {
            bink_00: None,
            device_04: false,
            tiles_08: None,
            frames_shown_0c: 0,
            skipped_frames_10: 0,
            scale_14: 1.0,
            x_18: 0.0,
            y_1c: 0.0,
            preparing_24: 0,
        }
    }
}

/// IDA: MoviePlayerMovie. `+0x20` (1 playing, 2 menu-owned) is an atomic because 00410425 reads it from whichever thread
/// asks; the rest is behind a lock held for one step at a time, never across 00410390.
#[derive(Debug, Default)]
pub struct MoviePlayerMovie {
    pub state_20: AtomicU32,
    fields: Mutex<MovieFields>,
}

impl MoviePlayerMovie {
    /// IDA: N/A an accessor.
    pub fn fields(&self) -> MutexGuard<'_, MovieFields> {
        lock_shared(&self.fields)
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn state(&self) -> u32 {
        self.state_20.load(Ordering::Acquire)
    }

    /// IDA: N/A an accessor.
    pub fn set_state(&self, state: u32) {
        self.state_20.store(state, Ordering::Release);
    }
}

/// IDA: 00B33424..00B33440 (named in IDA batch 1). Identity of the three movie slots is `Arc::ptr_eq`.
#[derive(Debug, Default)]
pub struct MoviePlayerGlobals {
    pub hold_frame_b33424: AtomicU8,
    pub waiter_active_b33425: AtomicU8,
    pub skip_requested_b33426: AtomicU8,
    pub scale_to_width_b33427: AtomicU8,
    pub current_b33428: Mutex<Option<Arc<MoviePlayerMovie>>>,
    pub paused_b3342c: Mutex<Option<Arc<MoviePlayerMovie>>>,
    pub cached_valid_b33431: AtomicU8,
    pub thread_b33434: Mutex<Option<MovieThreadHandle>>,
    pub thread_priority_b33438: AtomicI32,
    pub cached_b33440: Mutex<Option<Arc<MoviePlayerMovie>>>,
    /// IDA: N/A the handle the host closes at shutdown; the thread clears 00B33434 itself.
    pub host_thread_owner: Mutex<Option<MovieThreadHandle>>,
}

/// IDA: the INI settings the MoviePlayer reads.
#[derive(Clone, Debug, PartialEq)]
pub struct MovieSettings {
    /// 00B16178 bSoundEnabled:Audio, its low byte.
    pub sound_enabled_b16178: bool,
    /// 00B030B4 bPreloadIntroSequence:General.
    pub preload_intro_sequence_b030b4: bool,
    /// 00B030BC iPreloadSizeLimit:General.
    pub preload_size_limit_b030bc: i32,
    /// 00B030C4 iIntroSequencePriority:General.
    pub intro_sequence_priority_b030c4: i32,
    /// 00B030AC fStaticScreenWaitTime:General.
    pub static_screen_wait_time_b030ac: f32,
    /// 00B03084 sIntroSequence:General, the original's bytes.
    pub intro_sequence_b03084: Vec<u8>,
}

/// IDA: N/A reads the six settings of the 03-E collection by address. Completion: 100%.
#[must_use]
pub fn movie_settings_from(ini: &SettingCollection) -> MovieSettings {
    let raw = |address| ini.find_address(address).map_or(0, |setting| setting.raw);
    MovieSettings {
        sound_enabled_b16178: raw(0x00B1_6178) as u8 != 0,
        preload_intro_sequence_b030b4: raw(0x00B0_30B4) as u8 != 0,
        preload_size_limit_b030bc: raw(0x00B0_30BC) as i32,
        intro_sequence_priority_b030c4: raw(0x00B0_30C4) as i32,
        static_screen_wait_time_b030ac: f32::from_bits(raw(0x00B0_30AC)),
        intro_sequence_b03084: ini
            .find_address(0x00B0_3084)
            .and_then(|setting| setting.text.clone())
            .unwrap_or_default(),
    }
}

/// IDA: N/A what the thread reads of 00B33398 g_osGlobals: clones of the shared parts.
#[derive(Clone, Debug, Default)]
pub struct MovieOsGlobals {
    pub input_20: Option<SharedOsInput>,
    pub sound_24: Option<GameSoundPause>,
    pub main_thread_id_10: u32,
    pub main_thread_handle_14: u32,
}

impl From<&OsGlobalsData> for MovieOsGlobals {
    /// IDA: N/A Rust's `From`.
    fn from(globals: &OsGlobalsData) -> Self {
        Self {
            input_20: globals.input_20.clone(),
            sound_24: globals
                .sound_24
                .as_ref()
                .map(|sound| sound.game_sounds_paused_a4.clone()),
            main_thread_id_10: globals.main_thread_id_10,
            main_thread_handle_14: globals.main_thread_handle_14,
        }
    }
}

/// IDA: N/A the other subsystems' state the MoviePlayer reads, as the thread can reach it. Set before the thread starts; a
/// later change takes effect only for a new `MoviePlayer` clone.
#[derive(Clone)]
pub struct MoviePlayerServices {
    /// binkw32 imports 00A28330..00A28350.
    pub bink: Arc<Mutex<dyn BinkLibrary>>,
    /// g_dx9Renderer/g_renderer +0x280.
    pub device: Option<Arc<Mutex<dyn MovieDevice>>>,
    pub platform: Arc<Mutex<dyn MoviePlatform>>,
    /// The seam 00410390's polls go through.
    pub os_platform: Arc<Mutex<dyn OsGlobalsPlatform + Send>>,
    /// 00B06C4C (fild, signed).
    pub renderer_width: i32,
    /// 00B06C50.
    pub renderer_height: i32,
    /// 00B33398.
    pub os_globals: Option<MovieOsGlobals>,
    /// 00B33E90 (+0x10 for the poll).
    pub time: SharedTimeInfo,
    /// 00B3A6E0 and the TileMenu table.
    pub interface: InterfaceMovieReads,
    /// The "Data\Video\" root.
    pub video_directory: PathBuf,
    pub settings: MovieSettings,
}

/// IDA: N/A the MoviePlayer for both threads. Clone it to hand it to a thread.
#[derive(Resource, Clone)]
pub struct MoviePlayer {
    pub globals: Arc<MoviePlayerGlobals>,
    pub services: Arc<MoviePlayerServices>,
}

impl MoviePlayer {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn new(services: MoviePlayerServices) -> Self {
        Self {
            globals: Arc::new(MoviePlayerGlobals::default()),
            services: Arc::new(services),
        }
    }

    // IDA: N/A an accessor.
    fn warn(&self, text: &str) {
        lock_shared(&self.services.platform).warning(text);
    }
}

/// IDA: N/A the x87 precision a movie call's operations run at, from the thread's control word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovieX87 {
    /// 007F, the main thread after CreateDevice.
    Bits24,
    /// 027F, the movie thread.
    Bits53,
}

/// IDA: N/A which control word this thread has: 0041022E's own test of the main thread's ID. With no g_osGlobals, the movie thread's.
fn movie_x87(player: &MoviePlayer) -> MovieX87 {
    let main = player.services.os_globals.as_ref().is_some_and(|globals| {
        lock_shared(&player.services.platform).get_current_thread_id(0x0041_022E)
            == globals.main_thread_id_10
    });
    if main {
        MovieX87::Bits24
    } else {
        MovieX87::Bits53
    }
}

/// IDA: 00410310 MoviePlayer_ConfigureBinkSound: when OSGlobals+0x24 (sound) and bSoundEnabled, BinkSetSoundSystem
/// (BinkOpenDirectSound, 0); returns its result, left in eax. Completion: 100%; the original reads g_osGlobals without a null
/// check (00410315), and the host returns 0.
#[must_use]
pub fn configure_bink_sound(player: &MoviePlayer) -> i32 {
    let services = &player.services;
    let has_sound = services
        .os_globals
        .as_ref()
        .is_some_and(|globals| globals.sound_24.is_some());
    if !has_sound || !services.settings.sound_enabled_b16178 {
        return 0;
    }
    lock_shared(&services.bink).set_direct_sound_system()
}

/// IDA: 0040FE00 MoviePlayer_SetThreadPriority: 00B33438 = priority, then SetThreadPriority(hThread, {0:-2, 1:-1, 3:1, 4:2,
/// 5:15, else 0}); 0040FE04 `cmp ecx, 5; ja` is unsigned. Each case reads hThread, then stores, then calls. Returns the BOOL.
/// Completion: 100%, with the recorded substitution on `MovieThreadHandle::set_priority`.
pub fn set_movie_thread_priority(player: &MoviePlayer, priority: i32) -> bool {
    let value = match priority as u32 {
        0 => -2,
        1 => -1,
        3 => 1,
        4 => 2,
        5 => 15,
        _ => 0,
    };
    let thread = lock_shared(&player.globals.thread_b33434).clone();
    player
        .globals
        .thread_priority_b33438
        .store(priority, Ordering::Release);
    thread.is_some_and(|thread| {
        thread.set_priority(value);
        true
    })
}

/// IDA: 00410E40 MoviePlayer_StartAsyncSequence(sequence, flag, priority): CreateThread(NULL, 0, 00410E20, sequence,
/// CREATE_SUSPENDED, &id), 00410E64 hThread = the handle (before the name), 00410E69 00747830(id, "MoviePlayer"), 00410E72
/// 00B33427 = flag, 00410E7E..00410E8D -1 -> iIntroSequencePriority, 00410E9C ResumeThread. Returns ResumeThread's result.
/// The original passes the INI string's pointer; the thread owns a copy of its bytes here. Completion: 100%.
pub fn start_async_movie_sequence(
    player: &MoviePlayer,
    sequence: &[u8],
    scale_to_width: bool,
    priority: i32,
) -> u32 {
    let globals = &player.globals;
    let thread_player = player.clone();
    let sequence = sequence.to_vec();
    let thread = MovieThreadHandle::spawn_suspended("MoviePlayer", move || {
        movie_player_thread_proc(&thread_player, &sequence);
    })
    .ok();
    *lock_shared(&globals.thread_b33434) = thread.clone();
    *lock_shared(&globals.host_thread_owner) = thread;
    globals
        .scale_to_width_b33427
        .store(u8::from(scale_to_width), Ordering::Release);
    set_movie_thread_priority(
        player,
        if priority == -1 {
            player.services.settings.intro_sequence_priority_b030c4
        } else {
            priority
        },
    );
    let thread = lock_shared(&globals.thread_b33434).clone();
    thread.map_or(u32::MAX, |thread| thread.resume())
}

/// IDA: 00410E20 MoviePlayer_ThreadProc: 00410E25 the sequence, then 00410E2F..00410E37 hThread = 0 unless a waiter holds it.
/// Completion: 100%.
pub fn movie_player_thread_proc(player: &MoviePlayer, sequence: &[u8]) -> u32 {
    play_movie_sequence(player, sequence);
    if player.globals.waiter_active_b33425.load(Ordering::Acquire) == 0 {
        *lock_shared(&player.globals.thread_b33434) = None;
    }
    0
}

/// IDA: 00410D10 MoviePlayer_PlaySequence: a FormHeap copy (00410D36) split with `_mbstok(",")`; for each token, unless the
/// skip flag is set and while 00410390(1) allows, `.bik` -> 00410BA0(token, 1, 1, 0, bPreloadIntroSequence, 00B33427, 0) and
/// `.dds` -> 00410840(token); then 00410E08 free and 00410E14 skip flag = 0. Completion: 100%, with the recorded host
/// differences: three zero bytes before the copy keep the unchecked `token + len - 3` (00410D9B) inside the buffer for a token
/// under three bytes, where the original reads heap bytes; `_mbstok` and `_mbsicmp` are single-byte, as they are with the
/// CRT's default multibyte code page 0.
pub fn play_movie_sequence(player: &MoviePlayer, sequence: &[u8]) {
    let globals = &player.globals;
    let length = sequence
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(sequence.len());
    let mut copy = vec![0; 3];
    copy.extend_from_slice(&sequence[..length]);
    copy.push(0);
    let mut cursor = 3;
    while let Some((start, end)) = next_token(&mut copy, &mut cursor) {
        // 00410D70..00410D85.
        if globals.skip_requested_b33426.load(Ordering::Acquire) != 0
            || !movie_should_continue(player, true)
        {
            continue;
        }
        let name: String = copy[start..end]
            .iter()
            .map(|&byte| char::from(byte))
            .collect();
        // 00410D9B lea edi, [eax+esi-3]: a C string from there.
        let suffix = &copy[end - 3..];
        let suffix = &suffix[..suffix
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(suffix.len())];
        if stricmp(suffix, b"bik") == 0 {
            let settings = &player.services.settings;
            play_movie(
                player,
                &name,
                true,
                true,
                false,
                settings.preload_intro_sequence_b030b4,
                globals.scale_to_width_b33427.load(Ordering::Acquire) != 0,
                false,
            );
        } else if stricmp(suffix, b"dds") == 0 {
            show_movie_splash(player, &name);
        }
    }
    globals.skip_requested_b33426.store(0, Ordering::Release);
}

/// IDA: 0098349C `_mbstok(text, ",")` over one buffer: skip the delimiters, NUL the next one, and return the token's range.
/// Completion: 100% single-byte.
fn next_token(buffer: &mut [u8], cursor: &mut usize) -> Option<(usize, usize)> {
    while buffer[*cursor] == b',' {
        *cursor += 1;
    }
    if buffer[*cursor] == 0 {
        return None;
    }
    let start = *cursor;
    while buffer[*cursor] != 0 && buffer[*cursor] != b',' {
        *cursor += 1;
    }
    let end = *cursor;
    if buffer[end] == b',' {
        buffer[end] = 0;
        *cursor += 1;
    }
    Some((start, end))
}

/// IDA: 00410390 MoviePlayer_ShouldContinue. 00410394..004103DF pump this thread's queue; 004103E3..004103F9
/// g_renderer+0x280 TestCooperativeLevel() < 0 stops; 004103FF the skip flag; 0041040C..0041043A InterfaceManager_GetSingleton
/// (0, 1) (never creates), the current movie, its state and 00589B70(1001); 0041043C..0041044A; 0041044C..004104AE poll,
/// controls 5 and 29 (just pressed), then the skip flag while the thread runs and a second poll. Completion: 100%, with two
/// recorded host answers where the original faults: no device stops, and no input continues.
#[must_use]
pub fn movie_should_continue(player: &MoviePlayer, poll_skip_keys: bool) -> bool {
    let globals = &player.globals;
    let services = &player.services;
    lock_shared(&services.platform).pump_messages(0x0041_03A7);
    let Some(device) = &services.device else {
        return false;
    };
    let cooperative = lock_shared(device).call(0x0041_03F5, MovieDeviceCall::TestCooperativeLevel);
    if (cooperative as i32) < 0 {
        return false;
    }
    if globals.skip_requested_b33426.load(Ordering::Acquire) != 0 {
        return false;
    }
    if services.interface.interface_manager_present() {
        let current = lock_shared(&globals.current_b33428).clone();
        if let Some(current) = current
            && current.state() != MOVIE_STATE_MENU_OWNED
            && services.interface.tile_menu_present(MENU_ID_MESSAGE)
        {
            return false;
        }
    }
    if !poll_skip_keys && globals.waiter_active_b33425.load(Ordering::Acquire) == 0 {
        return true;
    }
    let Some(input) = services
        .os_globals
        .as_ref()
        .and_then(|os_globals| os_globals.input_20.as_ref())
    else {
        return true;
    };
    // The C++ `os_input_mutex` scope: the input stays locked across both polls and the queries.
    let mut input = input.lock();
    let mut os_platform = lock_shared(&services.os_platform);
    let time_info_10 = services.time.lock().paused_ticks_10;
    input.poll_devices(time_info_10, &mut *os_platform);
    if input.query_control_state(CONTROL_ACTIVATE, OsInputGlobals::QUERY_PRESSED) == 0
        && input.query_control_state(CONTROL_ESCAPE, OsInputGlobals::QUERY_PRESSED) == 0
    {
        return true;
    }
    let thread = lock_shared(&globals.thread_b33434).clone();
    if thread.is_some_and(|thread| thread.is_active()) {
        globals.skip_requested_b33426.store(1, Ordering::Release);
    }
    let time_info_10 = services.time.lock().paused_ticks_10;
    input.poll_devices(time_info_10, &mut *os_platform);
    false
}

/// IDA: 00410BA0 MoviePlayer_PlayMovie(name, pollSkipKeys, quiet, loop, preload, scaleToWidth, keepCurrent). 00410BA3 `cmp
/// [esp+arg_18], bl; jnz`; 00410BA9 pause; a valid cached object (00410BBF..00410BC5), else FormHeap(0x28) and the 00410BDB..
/// stores; 00410C00 current; 00410C29 play; 00410C30 finish. Completion: 100%; the original calls 00410A70 on a null object
/// when the allocation fails (it faults), and the host, as the C++ does, finishes and returns false.
#[allow(clippy::too_many_arguments)] // 00410BA0's own seven arguments and the MoviePlayer that stands for its globals.
pub fn play_movie(
    player: &MoviePlayer,
    name: &str,
    poll_skip_keys: bool,
    quiet: bool,
    looping: bool,
    preload: bool,
    scale_to_width: bool,
    keep_current: bool,
) -> bool {
    let globals = &player.globals;
    if !keep_current {
        pause_current_movie(player);
        let cached = lock_shared(&globals.cached_b33440).clone();
        let movie = match cached {
            Some(cached) if globals.cached_valid_b33431.load(Ordering::Acquire) != 0 => {
                *lock_shared(&globals.cached_b33440) = None;
                globals.cached_valid_b33431.store(0, Ordering::Release);
                Some(cached)
            }
            _ => lock_shared(&player.services.platform)
                .allocate(0x0041_0BCF, 0x28)
                .then(|| Arc::new(MoviePlayerMovie::default())),
        };
        *lock_shared(&globals.current_b33428) = movie;
    }
    let current = lock_shared(&globals.current_b33428).clone();
    let Some(current) = current else {
        finish_current_movie(player);
        return false;
    };
    let result = movie_play(
        player,
        &current,
        name,
        poll_skip_keys,
        quiet,
        looping,
        preload,
        scale_to_width,
    );
    finish_current_movie(player);
    result
}

/// IDA: 00410A70 MoviePlayerMovie_Play. 00410A79 state = 1; 00410A82 edi = g_osGlobals->sound, 00410A8B pause; 00410AA6 +0x24 =
/// 1, 00410AA9 prepare, 00410AB0 +0x24 = 0; 00410AB4 a failed prepare returns with the game sounds still paused; 00410AC5..
/// 00410AD0 present until it returns 0; 00410AD2..00410ADC framesShown >= HBINK Frames (unsigned); 00410ADF close; 00410AE4..
/// 00410AF2 resume twice. Completion: 100%; 00410A82's g_osGlobals is not null-tested in the original, and is here.
#[allow(clippy::too_many_arguments)] // 00410A70's own six arguments, `this` and the MoviePlayer.
pub fn movie_play(
    player: &MoviePlayer,
    movie: &MoviePlayerMovie,
    name: &str,
    poll_skip_keys: bool,
    quiet: bool,
    looping: bool,
    preload: bool,
    scale_to_width: bool,
) -> bool {
    let services = &player.services;
    let sound_enabled = services.settings.sound_enabled_b16178;
    movie.set_state(1);
    let sound = services
        .os_globals
        .as_ref()
        .and_then(|globals| globals.sound_24.as_ref());
    if let Some(sound) = sound {
        sound.pause(sound_enabled);
    }
    movie.fields().preparing_24 = 1;
    let prepared = movie_prepare(player, movie, name, quiet, preload, scale_to_width);
    movie.fields().preparing_24 = 0;
    if !prepared {
        return false;
    }
    while movie_present_frame(player, movie, poll_skip_keys, looping) {}
    let bink = movie.fields().bink_00;
    let frames = bink.map_or(0, |bink| lock_shared(&services.bink).info(bink).frames_08);
    let finished = movie.fields().frames_shown_0c >= frames;
    movie_close(player, movie);
    if let Some(sound) = sound {
        sound.resume(sound_enabled);
        sound.resume(sound_enabled);
    }
    finished
}

/// IDA: 004104C0 MoviePlayerMovie_Prepare. 004104C8..004104E2 open when there is no handle; 004104E8..004104FC +4 =
/// g_dx9Renderer+0x280; 004104FF..0041052F the renderer dimension (fild, signed) over the movie's (fild, +2^32 when negative:
/// unsigned), stored as a float; 00410532..00410566 x = 0 scaling to the width, else (W - w*s) * 0.5; 00410569..004105A1 y =
/// (H - s*h) * 0.5 scaling to the width, else 0; 0041059A..004105B8 0040FEE0(device, w, h, 4, 256, 22); 004105BD..004105CF the
/// failure. Each operation by `movie_x87`: 53-bit on the movie thread, 24-bit on the main thread's 0040F0BE. Completion: 100%.
pub fn movie_prepare(
    player: &MoviePlayer,
    movie: &MoviePlayerMovie,
    name: &str,
    quiet: bool,
    preload: bool,
    scale_to_width: bool,
) -> bool {
    let services = &player.services;
    if movie.fields().bink_00.is_none() && !movie_open(player, movie, name, quiet, preload, false) {
        return false;
    }
    let x87 = movie_x87(player);
    let mut fields = movie.fields();
    let Some(bink) = fields.bink_00 else {
        return false;
    };
    let info = lock_shared(&services.bink).info(bink);
    fields.device_04 = services.device.is_some();
    let (renderer_width, renderer_height) = (
        f64::from(services.renderer_width),
        f64::from(services.renderer_height),
    );
    let screen = if scale_to_width {
        renderer_width
    } else {
        renderer_height
    };
    let frame = f64::from(if scale_to_width {
        info.width_00
    } else {
        info.height_04
    });
    let scale = stored_float(screen / frame);
    fields.scale_14 = scale;
    // 0041053C..00410566 and 00410571..00410592: (screen - size * scale) * 0.5.
    let centered = |screen: f64, size: u32| match x87 {
        MovieX87::Bits53 => stored_float((screen - f64::from(size) * f64::from(scale)) * 0.5),
        MovieX87::Bits24 => (screen as f32 - size as f32 * scale) * 0.5f32,
    };
    fields.x_18 = if scale_to_width {
        0.0
    } else {
        centered(renderer_width, info.width_00)
    };
    fields.y_1c = if scale_to_width {
        centered(renderer_height, info.height_04)
    } else {
        0.0
    };
    fields.tiles_08 = services.device.as_ref().and_then(|device| {
        create_movie_tile_set(
            &mut *lock_shared(device),
            &mut *lock_shared(&services.platform),
            info.width_00,
            info.height_04,
            TILE_BYTES_PER_PIXEL,
            TILE_SIZE,
            D3DFMT_X8R8G8B8,
        )
    });
    if fields.tiles_08.is_none() {
        drop(fields);
        if !quiet {
            player.warn(&format!("Could not allocate textures for {name} playback."));
        }
        movie.set_state(0);
        return false;
    }
    true
}

/// IDA: 00410160 MoviePlayerMovie_Open(name, quiet, preload, forcePreload). 0041017B..004101DD "Data\Video\" + name;
/// 004101E1..00410216 preload when the arguments allow and the file's st_size <= iPreloadSizeLimit (signed) or forced;
/// 0041021B..0041024C off the main thread, SuspendThread(OSGlobals+0x14) (a null handle at startup: the call fails,
/// P06-RUN-001) and 0x8000000; 00410252..00410260 BinkOpen; 00410265..00410287 the failure; 00410289..00410299 ResumeThread.
/// Completion: 100%.
pub fn movie_open(
    player: &MoviePlayer,
    movie: &MoviePlayerMovie,
    name: &str,
    quiet: bool,
    preload: bool,
    force_preload: bool,
) -> bool {
    let services = &player.services;
    let path = host_path::resolve(&services.video_directory, name);
    let mut flags = 0;
    let mut platform = lock_shared(&services.platform);
    if (preload || force_preload)
        && let Some(size) = platform.file_size(0x0041_01F9, &path)
        && (size <= services.settings.preload_size_limit_b030bc || force_preload)
    {
        flags = BINK_PRELOAD_ALL;
    }
    let mut suspended = None;
    if let Some(os_globals) = &services.os_globals
        && platform.get_current_thread_id(0x0041_022E) != os_globals.main_thread_id_10
    {
        suspended = Some(os_globals.main_thread_handle_14);
        platform.suspend_thread(0x0041_0246, os_globals.main_thread_handle_14);
        flags |= BINK_NO_THREADED_IO;
    }
    let bink = lock_shared(&services.bink).open(&path, flags);
    movie.fields().bink_00 = bink;
    let mut result = true;
    if bink.is_none() {
        if !quiet {
            platform.warning(&format!("Could not open {name} for playback."));
        }
        movie.set_state(0);
        result = false;
    }
    if let Some(handle) = suspended {
        platform.resume_thread(0x0041_0299, handle);
    }
    result
}

/// IDA: 004106C0 MoviePlayerMovie_PresentFrame(pollSkipKeys, loop). 004106C8 continue test; 004106D8 00B33424 holds the frame;
/// 004106E6 BinkWait; 004107ED..004107FF early: Sleep(1) and continue, unless the menu owns the movie (state 2), which draws
/// anyway; 004106F4..0041072C copy (copyAll 0), skipped-frame log, next; 00410730..00410755 Clear(0, NULL, TARGET, 0xFF000000,
/// 1.0, 0); 00410757..004107AD Begin/EndScene unless the renderer is in a scene (+0x200 == 1, read twice); 00410790 0040FA20;
/// 004107AF..004107C6 Present unless state 2; 004107C8..004107CF 00768960(g_dx9Renderer, 0x104); 004107D4..004107E4 framesShown
/// < Frames (unsigned) || loop. Completion: 100%; the device and HBINK are unchecked in the original (host stop).
pub fn movie_present_frame(
    player: &MoviePlayer,
    movie: &MoviePlayerMovie,
    poll_skip_keys: bool,
    looping: bool,
) -> bool {
    let services = &player.services;
    if !movie_should_continue(player, poll_skip_keys) {
        return false;
    }
    let mut guard = movie.fields();
    let fields = &mut *guard;
    let bink = fields
        .bink_00
        .expect("004106E2: the original passes a null HBINK to BinkWait");
    let device = services
        .device
        .as_ref()
        .expect("00410733: the original calls through a null device");
    let menu_owned = movie.state() == MOVIE_STATE_MENU_OWNED;
    let tiles = fields
        .tiles_08
        .as_deref_mut()
        .expect("004106F4: the original copies into a null tile set");
    if player.globals.hold_frame_b33424.load(Ordering::Acquire) == 0 {
        if lock_shared(&services.bink).wait(bink) != 0 {
            if !menu_owned {
                lock_shared(&services.platform).sleep(0x0041_07F9, 1);
                return true;
            }
        } else {
            let skipped = copy_movie_frame_to_tiles(
                &mut *lock_shared(&services.bink),
                &mut *lock_shared(device),
                bink,
                tiles,
                false,
            );
            if skipped {
                fields.skipped_frames_10 = fields.skipped_frames_10.wrapping_add(1);
                lock_shared(&services.platform).debug_log(&format!(
                    "Bink playback: Skipped frame #{}.  Total skipped: {}",
                    fields.frames_shown_0c as i32, fields.skipped_frames_10 as i32
                ));
            }
            lock_shared(&services.bink).next_frame(bink);
            fields.frames_shown_0c = fields.frames_shown_0c.wrapping_add(1);
        }
    }
    let in_scene = |site| lock_shared(&services.platform).renderer_in_scene(site);
    let x87 = movie_x87(player);
    lock_shared(device).call(
        0x0041_0755,
        MovieDeviceCall::Clear {
            count: 0,
            flags: D3DCLEAR_TARGET,
            color: MOVIE_CLEAR_COLOR,
            z_bits: 1.0f32.to_bits(),
            stencil: 0,
        },
    );
    if !in_scene(0x0041_0757) {
        lock_shared(device).call(0x0041_076C, MovieDeviceCall::BeginScene);
    }
    draw_movie_tiles(
        &mut *lock_shared(device),
        tiles,
        fields.x_18,
        fields.y_1c,
        fields.scale_14,
        fields.scale_14,
        x87,
    );
    if !in_scene(0x0041_0798) {
        lock_shared(device).call(0x0041_07AD, MovieDeviceCall::EndScene);
    }
    if !menu_owned {
        lock_shared(device).call(0x0041_07C6, MovieDeviceCall::Present);
    }
    lock_shared(&services.platform).set_default_shader_pass_fvf(0x0041_07CF, MOVIE_QUAD_FVF);
    let frames = lock_shared(&services.bink).info(bink).frames_08;
    fields.frames_shown_0c < frames || looping
}

/// IDA: 004102C0 MoviePlayerMovie_Close. 004102C4..004102DB close and free; 004102E5..00410301 in store order. Completion:
/// 100%.
pub fn movie_close(player: &MoviePlayer, movie: &MoviePlayerMovie) {
    close_movie_fields(player, movie);
    movie.set_state(0);
}

/// IDA: the shared body of 004102C0 and 00410B02..00410B4A: close, free, then the stores (`+0x20` is the caller's).
fn close_movie_fields(player: &MoviePlayer, movie: &MoviePlayerMovie) {
    let services = &player.services;
    let mut fields = movie.fields();
    if let Some(bink) = fields.bink_00 {
        lock_shared(&services.bink).close(bink);
    }
    if fields.tiles_08.is_some() {
        match &services.device {
            Some(device) => destroy_movie_tile_set(&mut *lock_shared(device), &mut fields.tiles_08),
            None => fields.tiles_08 = None,
        }
    }
    *fields = MovieFields::default();
}

/// IDA: 00410B00 MoviePlayer_FinishCurrentMovie. 00410B02..00410B55 close, reset and free the current movie; 00410B5B..00410B73
/// a paused movie becomes current and resumes. Completion: 100%; the free is the slot's `Arc`, so a late reader stays valid.
pub fn finish_current_movie(player: &MoviePlayer) {
    let globals = &player.globals;
    let current = lock_shared(&globals.current_b33428).clone();
    if let Some(current) = current {
        close_movie_fields(player, &current);
        current.set_state(0);
        *lock_shared(&globals.current_b33428) = None;
    }
    let paused = lock_shared(&globals.paused_b3342c).clone();
    if let Some(paused) = paused {
        *lock_shared(&globals.current_b33428) = Some(Arc::clone(&paused));
        *lock_shared(&globals.paused_b3342c) = None;
        let bink = paused.fields().bink_00;
        if let Some(bink) = bink {
            lock_shared(&player.services.bink).pause(bink, 0);
        }
    }
}

/// IDA: 00410340 MoviePlayer_PauseCurrentMovie. 00410340..00410347 no current movie returns; 00410349..00410357 warns when
/// another is paused; 0041035F..0041037B BinkPause(current->bink, 1), paused = current, current = 0. Completion: 100%.
pub fn pause_current_movie(player: &MoviePlayer) {
    let globals = &player.globals;
    let current = lock_shared(&globals.current_b33428).clone();
    let Some(current) = current else {
        return;
    };
    if lock_shared(&globals.paused_b3342c).is_some() {
        player.warn(
            "Trying to pause a movie while another movie is already paused.  This is not currently supported.",
        );
    }
    let bink = current.fields().bink_00;
    if let Some(bink) = bink {
        lock_shared(&player.services.bink).pause(bink, 1);
    }
    *lock_shared(&globals.paused_b3342c) = Some(current);
    *lock_shared(&globals.current_b33428) = None;
}

/// IDA: 00410840 MoviePlayer_ShowSplash. 00410854..004108D4 "Data\Video\" + name and D3DXCreateTextureFromFileA (a failure
/// returns at once); 004108DA..004108FE Clear, BeginScene; 00410900..00410933 GetLevelDesc(0); 00410935..00410979 scale = H /
/// height (unsigned), x = (W - s * width) * 0.5; 0041097D..00410992 states and SetTexture; 00410994..004109C8 quad (x, 0, 1, 1,
/// (int)(W - (x + x)), H), __ftol2_sse truncating; 004109CD..004109E9 EndScene, Present; 004109EB..00410A31 deadline =
/// GetTickCount (unsigned) + wait * 1000.0, a 64-bit truncating fistp of which the low dword stays; 00410A35..00410A47 the
/// wait; 00410A49..00410A53 Release. Returns the HRESULT. Completion: 95%: 53-bit arithmetic and call order, as data.
pub fn show_movie_splash(player: &MoviePlayer, name: &str) -> i32 {
    let services = &player.services;
    let Some(device) = &services.device else {
        return 0;
    };
    let path = host_path::resolve(&services.video_directory, name);
    let created = lock_shared(device).create_texture_from_file(0x0041_08CD, &path);
    let Some(splash) = created.object else {
        return created.result as i32;
    };
    lock_shared(device).call(
        0x0041_08F3,
        MovieDeviceCall::Clear {
            count: 0,
            flags: D3DCLEAR_TARGET,
            color: MOVIE_CLEAR_COLOR,
            z_bits: 1.0f32.to_bits(),
            stencil: 0,
        },
    );
    lock_shared(device).call(0x0041_08FE, MovieDeviceCall::BeginScene);
    lock_shared(device).call(
        0x0041_0933,
        MovieDeviceCall::GetLevelDesc {
            texture: splash.texture,
        },
    );
    let renderer_width = f64::from(services.renderer_width);
    let scale = stored_float(f64::from(services.renderer_height) / f64::from(splash.height));
    let x = stored_float((renderer_width - f64::from(scale) * f64::from(splash.width)) * 0.5);
    {
        let mut device = lock_shared(device);
        set_movie_quad_states(&mut *device);
        device.call(
            0x0041_0992,
            MovieDeviceCall::SetTexture {
                stage: 0,
                texture: Some(splash.texture),
            },
        );
        let width = ftol64_low_f64(renderer_width - (f64::from(x) + f64::from(x))) as i32;
        draw_movie_quad(
            &mut *device,
            x,
            0.0,
            1.0,
            1.0,
            width,
            services.renderer_height,
            MovieX87::Bits53,
        );
        device.call(0x0041_09D9, MovieDeviceCall::EndScene);
        device.call(0x0041_09E9, MovieDeviceCall::Present);
    }
    let now = lock_shared(&services.platform).get_tick_count(0x0041_09F1);
    let deadline = ftol64_low_f64(
        f64::from(now) + f64::from(services.settings.static_screen_wait_time_b030ac) * 1000.0,
    );
    while deadline > lock_shared(&services.platform).get_tick_count(0x0041_0A35)
        && movie_should_continue(player, true)
    {}
    lock_shared(device).call(
        0x0041_0A53,
        MovieDeviceCall::ReleaseTexture {
            texture: splash.texture,
        },
    );
    0
}

/// IDA: 00410C40 MoviePlayer_PlayMenuMovie(name, preload), named here, for 005B5D07, 005B5FA0 and 0040557E on the main thread. 00410C44..00410C7D 00410340's body inline; 00410C83..00410CD2 a valid cached object or FormHeap(0x28) and the stores; 00410CE2 current; 00410CE8 004105F0; a false result finishes the current movie. Returns its result. Completion: 100%; a null FormHeap result faults in the original at 004105F3, and the host finishes and returns false.
pub fn play_menu_movie(player: &MoviePlayer, name: &str, preload: bool) -> bool {
    let globals = &player.globals;
    pause_current_movie(player);
    let cached = lock_shared(&globals.cached_b33440).clone();
    let movie = match cached {
        Some(cached) if globals.cached_valid_b33431.load(Ordering::Acquire) != 0 => {
            *lock_shared(&globals.cached_b33440) = None;
            globals.cached_valid_b33431.store(0, Ordering::Release);
            Some(cached)
        }
        _ => lock_shared(&player.services.platform)
            .allocate(0x0041_0CA5, 0x28)
            .then(|| Arc::new(MoviePlayerMovie::default())),
    };
    lock_shared(&globals.current_b33428).clone_from(&movie);
    let Some(movie) = movie else {
        finish_current_movie(player);
        return false;
    };
    let prepared = movie_prepare_menu(player, &movie, name, preload);
    if !prepared {
        finish_current_movie(player);
    }
    prepared
}

/// IDA: 004105F0 MoviePlayerMovie_PrepareMenu(name, preload), named here. 004105F6 state = 2; 004105FD +0x24 = 1; 00410603..00410613 open (quiet, not forced) when there is no handle; 00410620..0041062C +4; 0041062F..00410651 scale = H / height; 00410654..0041067B x = (W - scale * width) * 0.5; 0041067E y = 0; 0041068E the tile set (w, h, 4, 256, 22); a null set clears +0x20. +0x24 = 0 on every way out. These callers run at 24 bits, so each `fstp` and each operation is `f32`. Completion: 100%.
pub fn movie_prepare_menu(
    player: &MoviePlayer,
    movie: &MoviePlayerMovie,
    name: &str,
    preload: bool,
) -> bool {
    let services = &player.services;
    movie.set_state(MOVIE_STATE_MENU_OWNED);
    movie.fields().preparing_24 = 1;
    if movie.fields().bink_00.is_none() && !movie_open(player, movie, name, true, preload, false) {
        movie.fields().preparing_24 = 0;
        return false;
    }
    let mut fields = movie.fields();
    let Some(bink) = fields.bink_00 else {
        fields.preparing_24 = 0;
        return false;
    };
    let info = lock_shared(&services.bink).info(bink);
    fields.device_04 = services.device.is_some();
    let scale = services.renderer_height as f32 / info.height_04 as f32;
    fields.scale_14 = scale;
    fields.x_18 = (services.renderer_width as f32 - info.width_00 as f32 * scale) * 0.5f32;
    fields.y_1c = 0.0;
    fields.tiles_08 = services.device.as_ref().and_then(|device| {
        create_movie_tile_set(
            &mut *lock_shared(device),
            &mut *lock_shared(&services.platform),
            info.width_00,
            info.height_04,
            TILE_BYTES_PER_PIXEL,
            TILE_SIZE,
            D3DFMT_X8R8G8B8,
        )
    });
    fields.preparing_24 = 0;
    if fields.tiles_08.is_none() {
        drop(fields);
        movie.set_state(0);
        return false;
    }
    true
}

/// IDA: 00410810 MoviePlayer_PresentCurrent(pollSkipKeys, loop), named here: a current movie with a nonzero state presents one frame. 0057F3BC calls it as (0, 1) for a menu-owned movie. Completion: 100%.
pub fn present_current_movie(player: &MoviePlayer, poll_skip_keys: bool, looping: bool) -> bool {
    let current = lock_shared(&player.globals.current_b33428).clone();
    current.is_some_and(|movie| {
        movie.state() != 0 && movie_present_frame(player, &movie, poll_skip_keys, looping)
    })
}

/// IDA: 0040FDA0 MoviePlayer_IsRunning: one read of hThread; GetExitCodeThread == STILL_ACTIVE. Completion: 100%.
#[must_use]
pub fn movie_player_is_running(player: &MoviePlayer) -> bool {
    lock_shared(&player.globals.thread_b33434)
        .as_ref()
        .is_some_and(MovieThreadHandle::is_active)
}

/// IDA: 0040FDD0 MoviePlayer_WaitForThread: the loaded handle is waited on, and the waiter flag is set after the load. eax is
/// 0 without a thread, else the wait's result. Completion: 100%.
pub fn movie_player_wait_for_thread(player: &MoviePlayer) -> u32 {
    let globals = &player.globals;
    let thread = lock_shared(&globals.thread_b33434).clone();
    let Some(thread) = thread else {
        return 0;
    };
    globals.waiter_active_b33425.store(1, Ordering::Release);
    let waited = thread.wait();
    *lock_shared(&globals.thread_b33434) = None;
    globals.waiter_active_b33425.store(0, Ordering::Release);
    waited
}

/// IDA: N/A 0040FDD0's wait, for a host whose main thread must keep drawing: the movie thread's device calls are drained on it, so blocking would hold them back (recorded substitution). The first call does what 0040FDD0 does before its wait; `true` means the wait has ended and its stores ran, in 0040FDD0's order. Completion: 100% of the stores.
pub fn movie_player_poll_wait_for_thread(player: &MoviePlayer) -> bool {
    let globals = &player.globals;
    let thread = lock_shared(&globals.thread_b33434).clone();
    let Some(thread) = thread else {
        globals.waiter_active_b33425.store(0, Ordering::Release);
        return true;
    };
    globals.waiter_active_b33425.store(1, Ordering::Release);
    if thread.is_active() {
        return false;
    }
    thread.wait();
    *lock_shared(&globals.thread_b33434) = None;
    globals.waiter_active_b33425.store(0, Ordering::Release);
    true
}

/// IDA: 0040FD90 MoviePlayer_RequestSkip. Completion: 100%.
pub fn movie_player_request_skip(player: &MoviePlayer) {
    player
        .globals
        .skip_requested_b33426
        .store(1, Ordering::Release);
}

/// IDA: N/A host shutdown (06-E-2): ask the thread to stop, wait like 0040FDD0 on the owned handle (the published one may
/// already be cleared by the thread), and drop it. The original's own waiters are 0040F092/0040F0DB (Phase 21). Completion:
/// 100% of the host role; the C++ `MoviePlayerGuard` that calls it at exit belongs to `recomp_host`.
pub fn movie_player_shutdown(player: &MoviePlayer) {
    let globals = &player.globals;
    let owner = lock_shared(&globals.host_thread_owner).clone();
    let Some(owner) = owner else {
        return;
    };
    globals.skip_requested_b33426.store(1, Ordering::Release);
    globals.waiter_active_b33425.store(1, Ordering::Release);
    owner.wait();
    *lock_shared(&globals.thread_b33434) = None;
    globals.waiter_active_b33425.store(0, Ordering::Release);
    *lock_shared(&globals.host_thread_owner) = None;
    globals.skip_requested_b33426.store(0, Ordering::Release);
}

/// IDA: 0040F640: <= 2 unchanged; then the next power of two up to 512; 1024. Completion: 100%.
#[must_use]
pub const fn round_partial_tile_size(remainder: u32) -> u32 {
    if remainder <= 0x10 {
        if remainder <= 4 {
            return if remainder > 2 { 4 } else { remainder };
        }
        return if remainder > 8 { 16 } else { 8 };
    }
    if remainder <= 0x40 {
        return if remainder > 0x20 { 64 } else { 32 };
    }
    if remainder <= 0x80 {
        return 128;
    }
    if remainder <= 0x100 {
        return 256;
    }
    if remainder > 0x200 { 1024 } else { 512 }
}

/// IDA: 0040F6A0 (device, format, pitch, bpp, width, height): 0040F6D8 DYNAMIC in DEFAULT, else 0040F6F4 MANAGED (`pitch`
/// unused); 0040F6F8 `jl 0040F747` the null out value; 0040F70F LockRect, 0040F724 zero width * bpp bytes of each row, 0040F745
/// UnlockRect; a failed lock still returns the texture. Completion: 100%.
fn create_movie_tile_texture(
    device: &mut dyn MovieDevice,
    format: u32,
    bytes_per_pixel: u32,
    width: u32,
    height: u32,
) -> Option<MovieTile> {
    let mut created = device.create_texture(
        0x0040_F6D8,
        width,
        height,
        1,
        D3DUSAGE_DYNAMIC,
        format,
        D3DPOOL_DEFAULT,
    );
    if (created.result as i32) < 0 {
        created = device.create_texture(0x0040_F6F4, width, height, 1, 0, format, D3DPOOL_MANAGED);
        if (created.result as i32) < 0 {
            return None;
        }
    }
    let texture = created.object?;
    let pixels = vec![0; (width * height * bytes_per_pixel) as usize];
    let locked = device.call(0x0040_F70F, MovieDeviceCall::LockRect { texture });
    if (locked as i32) >= 0 {
        device.unlock_rect(0x0040_F745, texture, &pixels);
    }
    Some(MovieTile {
        texture,
        width,
        height,
        pixels,
    })
}

/// IDA: 0040FEE0 MovieTileSet_Create. 0040FEE4..0040FF4B remainders, partial sizes, pitch and the tile count; 0040FF4E..
/// 0040FF5F FormHeap(0x48 + 4n), null -> 0; 0040FF65..0040FFCE the stores; 0040FFD5..004100A3 full rows (full columns, then the
/// partial column), then the partial row and its corner; 004100A5..004100FE any null tile releases every created one and
/// frees; 004100C0 +0x18 = 0. Completion: 100%.
#[must_use]
pub fn create_movie_tile_set(
    device: &mut dyn MovieDevice,
    platform: &mut dyn MoviePlatform,
    width: u32,
    height: u32,
    bytes_per_pixel: u32,
    tile_size: u32,
    format: u32,
) -> Option<Box<MovieTileSet>> {
    let width_remainder = width % tile_size;
    let full_columns = width / tile_size;
    let partial_width = round_partial_tile_size(width_remainder);
    let full_rows = height / tile_size;
    let height_remainder = height % tile_size;
    let partial_height = round_partial_tile_size(height_remainder);
    let tile_count = (width.wrapping_add(tile_size).wrapping_sub(1) / tile_size)
        .wrapping_mul(height.wrapping_add(tile_size).wrapping_sub(1) / tile_size);
    if !platform.allocate(
        0x0040_FF56,
        0x48u32.wrapping_add(tile_count.wrapping_mul(4)),
    ) {
        return None;
    }
    let mut set = Box::new(MovieTileSet {
        format_04: format,
        bytes_per_pixel_08: bytes_per_pixel,
        width_0c: width,
        height_10: height,
        lock_index_14: 0,
        unk_18: 0,
        pitch_1c: width.wrapping_mul(bytes_per_pixel).wrapping_add(15) & 0xFFFF_FFF0,
        tile_count_20: tile_count,
        full_columns_24: full_columns,
        full_rows_28: full_rows,
        partial_width_2c: partial_width,
        partial_height_30: partial_height,
        width_remainder_34: width_remainder,
        height_remainder_38: height_remainder,
        tile_size_3c: tile_size,
        tiles_40: Vec::new(),
    });
    let mut created = Vec::with_capacity(tile_count as usize);
    let mut make = |device: &mut dyn MovieDevice, tile_width, tile_height| {
        created.push(create_movie_tile_texture(
            device,
            format,
            bytes_per_pixel,
            tile_width,
            tile_height,
        ));
    };
    for _ in 0..full_rows {
        for _ in 0..full_columns {
            make(device, tile_size, tile_size);
        }
        if partial_width != 0 {
            make(device, partial_width, tile_size);
        }
    }
    if partial_height != 0 {
        for _ in 0..full_columns {
            make(device, tile_size, partial_height);
        }
        if partial_width != 0 {
            make(device, partial_width, partial_height);
        }
    }
    if created.iter().any(Option::is_none) {
        for tile in created.iter().flatten() {
            device.call(
                0x0041_00E4,
                MovieDeviceCall::ReleaseTexture {
                    texture: tile.texture,
                },
            );
        }
        return None;
    }
    set.tiles_40 = created.into_iter().flatten().collect();
    set.unk_18 = 0;
    Some(set)
}

/// IDA: 00410110 MovieTileSet_Destroy: 00410115..0041014E release each tile in order, then free. Completion: 100%.
pub fn destroy_movie_tile_set(device: &mut dyn MovieDevice, set: &mut Option<Box<MovieTileSet>>) {
    let Some(tiles) = set.as_ref().map(|set| &set.tiles_40) else {
        return;
    };
    for tile in tiles {
        device.call(
            0x0041_0137,
            MovieDeviceCall::ReleaseTexture {
                texture: tile.texture,
            },
        );
    }
    *set = None;
}

/// IDA: 0040F880 MovieTileSet_LockNext. 0040F888..0040F8AF past the end or a failed LockRect resets the index and returns 0;
/// 0040F8D1..0040F8ED columns = full + (partial width != 0); the rectangle; 0040F950 ++index. Completion: 100%.
pub fn lock_next_movie_tile(
    device: &mut dyn MovieDevice,
    set: &mut MovieTileSet,
) -> Option<LockedMovieTile> {
    if set.lock_index_14 >= set.tile_count_20 {
        set.lock_index_14 = 0;
        return None;
    }
    let index = set.lock_index_14 as usize;
    let texture = set.tiles_40[index].texture;
    let locked = device.call(0x0040_F8AB, MovieDeviceCall::LockRect { texture });
    if (locked as i32) < 0 {
        set.lock_index_14 = 0;
        return None;
    }
    let columns = set.full_columns_24 + u32::from(set.partial_width_2c != 0);
    let row = set.lock_index_14 / columns;
    let column = set.lock_index_14 % columns;
    let tile = LockedMovieTile {
        index,
        pitch: (set.tiles_40[index].width * set.bytes_per_pixel_08) as i32,
        x: column * set.tile_size_3c,
        y: row * set.tile_size_3c,
        width: if column < set.full_columns_24 {
            set.tile_size_3c
        } else {
            set.partial_width_2c
        },
        height: if row < set.full_rows_28 {
            set.tile_size_3c
        } else {
            set.partial_height_30
        },
    };
    set.lock_index_14 += 1;
    Some(tile)
}

/// IDA: 0040FCA0 MovieTileSet_CopyFrame. 0040FCAA..0040FCAF result 1, BinkDoFrame; 0040FCF0..0040FD18 per tile (bink, bits,
/// pitch, Height, 0, 0, x, y, w, h, flags); 0040FD1E..0040FD32 UnlockRect on the tile just locked. Returns result != 0, the last
/// copy's. Completion: 100%.
pub fn copy_movie_frame_to_tiles(
    library: &mut dyn BinkLibrary,
    device: &mut dyn MovieDevice,
    bink: BinkHandle,
    set: &mut MovieTileSet,
    copy_all: bool,
) -> bool {
    let mut result = 1;
    library.do_frame(bink);
    let flags = if copy_all { BINK_COPY_ALL } else { 0 } | BINK_SURFACE32;
    while let Some(locked) = lock_next_movie_tile(device, set) {
        let height = library.info(bink).height_04;
        let tile = &mut set.tiles_40[locked.index];
        result = library.copy_to_buffer_rect(
            bink,
            &mut tile.pixels,
            locked.pitch,
            height,
            0,
            0,
            locked.x,
            locked.y,
            locked.width,
            locked.height,
            flags,
        );
        device.unlock_rect(0x0040_FD32, tile.texture, &tile.pixels);
    }
    result != 0
}

/// IDA: 0040F970 in call order. Completion: 100%.
pub fn set_movie_quad_states(device: &mut dyn MovieDevice) {
    let render = |device: &mut dyn MovieDevice, site, state, value| {
        device.call(site, MovieDeviceCall::SetRenderState { state, value });
    };
    let stage = |device: &mut dyn MovieDevice, site, state, value| {
        device.call(
            site,
            MovieDeviceCall::SetTextureStageState {
                stage: 0,
                state,
                value,
            },
        );
    };
    let sampler = |device: &mut dyn MovieDevice, site, state, value| {
        device.call(
            site,
            MovieDeviceCall::SetSamplerState {
                sampler: 0,
                state,
                value,
            },
        );
    };
    render(device, 0x0040_F97D, D3DRS_ALPHABLENDENABLE, 0);
    stage(device, 0x0040_F98E, 4, 2); // ALPHAOP, SELECTARG1
    stage(device, 0x0040_F99F, 2, 2); // COLORARG1, TEXTURE
    stage(device, 0x0040_F9B0, 5, 2); // ALPHAARG1, TEXTURE
    stage(device, 0x0040_F9C1, 1, 4); // COLOROP, MODULATE
    sampler(device, 0x0040_F9D2, 6, 2); // MINFILTER, LINEAR
    sampler(device, 0x0040_F9E3, 5, 2); // MAGFILTER, LINEAR
    sampler(device, 0x0040_F9F4, 7, 2); // MIPFILTER, LINEAR
    sampler(device, 0x0040_FA05, 1, 3); // ADDRESSU, CLAMP
    sampler(device, 0x0040_FA16, 2, 3); // ADDRESSV, CLAMP
}

const D3DRS_ALPHABLENDENABLE: u32 = 0x1B;
const D3DRS_ZENABLE: u32 = 7;
const D3DRS_ZWRITEENABLE: u32 = 0xE;
const D3DRS_COLORWRITEENABLE: u32 = 0xA8;
/// IDA: what a real device answers for a CreateTexture it cannot make.
const D3DERR_INVALIDCALL: u32 = 0x8876_086C;

/// IDA: 0040F760 (device, x, y, scaleX, scaleY, width, height). 0040F7AC..0040F7D5 x + width * scaleX and y + height * scaleY by `x87`, stored as floats; uv 0..1 as a strip. 0040F83C..0040F86F the shaders, FVF and draw. Completion: 100%.
#[allow(clippy::too_many_arguments)] // 0040F760's own seven and the thread's control word.
pub fn draw_movie_quad(
    device: &mut dyn MovieDevice,
    x: f32,
    y: f32,
    scale_x: f32,
    scale_y: f32,
    width: i32,
    height: i32,
    x87: MovieX87,
) {
    let far = |from: f32, size: i32, scale: f32| match x87 {
        MovieX87::Bits53 => stored_float(f64::from(from) + f64::from(size) * f64::from(scale)),
        MovieX87::Bits24 => from + size as f32 * scale,
    };
    let right = far(x, width, scale_x);
    let bottom = far(y, height, scale_y);
    let vertex = |x, y, u, v| MovieVertex {
        x,
        y,
        z: 0.0,
        rhw: 1.0,
        u,
        v,
    };
    let vertices = [
        vertex(x, y, 0.0, 0.0),
        vertex(x, bottom, 0.0, 1.0),
        vertex(right, y, 1.0, 0.0),
        vertex(right, bottom, 1.0, 1.0),
    ];
    device.call(0x0040_F83C, MovieDeviceCall::SetVertexShaderNull);
    device.call(0x0040_F849, MovieDeviceCall::SetPixelShaderNull);
    device.call(
        0x0040_F859,
        MovieDeviceCall::SetFvf {
            fvf: MOVIE_QUAD_FVF,
        },
    );
    device.call(
        0x0040_F86F,
        MovieDeviceCall::DrawPrimitiveUp {
            primitive: D3DPT_TRIANGLESTRIP,
            count: 2,
            stride: 24,
            vertices,
        },
    );
}

/// IDA: 0040FA20 MovieTileSet_Draw. 0040FA2D 0040F970; 0040FA3C..0040FA66 Z off and the color write mask; 0040FA68..0040FA9D the
/// tile size (fild, +2^32 when negative) as a float, its steps; the rows; 0040FB89..0040FC43 the partial row and its corner;
/// 0040FC4B..0040FC8D Z, alpha blend and the null texture. Each `fstp m32` and `fadd` by `x87`. Completion: 100%.
pub fn draw_movie_tiles(
    device: &mut dyn MovieDevice,
    set: &MovieTileSet,
    x: f32,
    y: f32,
    scale_x: f32,
    scale_y: f32,
    x87: MovieX87,
) {
    set_movie_quad_states(device);
    let render = |device: &mut dyn MovieDevice, site, state, value| {
        device.call(site, MovieDeviceCall::SetRenderState { state, value });
    };
    render(device, 0x0040_FA41, D3DRS_ZENABLE, 0);
    render(device, 0x0040_FA52, D3DRS_ZWRITEENABLE, 0);
    render(device, 0x0040_FA66, D3DRS_COLORWRITEENABLE, 0xF);
    let tile = stored_float(f64::from(set.tile_size_3c));
    let step_x = stored_float(f64::from(tile) * f64::from(scale_x));
    let step_y = stored_float(f64::from(tile) * f64::from(scale_y));
    let add = |left: f32, right: f32| match x87 {
        MovieX87::Bits53 => stored_float(f64::from(left) + f64::from(right)),
        MovieX87::Bits24 => left + right,
    };
    let size = set.tile_size_3c as i32;
    let mut textures = set.tiles_40.iter().map(|tile| Some(tile.texture));
    let mut set_texture = |device: &mut dyn MovieDevice, site| {
        device.call(
            site,
            MovieDeviceCall::SetTexture {
                stage: 0,
                texture: textures.next().flatten(),
            },
        );
    };
    let mut row_y = y;
    for _ in 0..set.full_rows_28 {
        let mut column_x = x;
        for _ in 0..set.full_columns_24 {
            set_texture(device, 0x0040_FAD0);
            draw_movie_quad(device, column_x, row_y, scale_x, scale_y, size, size, x87);
            column_x = add(column_x, step_x);
        }
        if set.partial_width_2c != 0 {
            set_texture(device, 0x0040_FB30);
            draw_movie_quad(
                device,
                column_x,
                row_y,
                scale_x,
                scale_y,
                set.partial_width_2c as i32,
                size,
                x87,
            );
        }
        row_y = add(row_y, step_y);
    }
    if set.partial_height_30 != 0 {
        let mut column_x = x;
        for _ in 0..set.full_columns_24 {
            set_texture(device, 0x0040_FBB2);
            draw_movie_quad(
                device,
                column_x,
                row_y,
                scale_x,
                scale_y,
                size,
                set.partial_height_30 as i32,
                x87,
            );
            column_x = add(column_x, step_x);
        }
        if set.partial_width_2c != 0 {
            set_texture(device, 0x0040_FC15);
            draw_movie_quad(
                device,
                column_x,
                row_y,
                scale_x,
                scale_y,
                set.partial_width_2c as i32,
                set.partial_height_30 as i32,
                x87,
            );
        }
    }
    render(device, 0x0040_FC5A, D3DRS_ZENABLE, 1);
    render(device, 0x0040_FC6B, D3DRS_ZWRITEENABLE, 1);
    render(device, 0x0040_FC7C, D3DRS_ALPHABLENDENABLE, 1);
    device.call(
        0x0040_FC8D,
        MovieDeviceCall::SetTexture {
            stage: 0,
            texture: None,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tes::bink_library::ScriptedBink;
    use crate::tes::os_globals::{
        DSERR_NODRIVER, HostWindow, OsSoundGlobals, S_FALSE, ScriptedOsPlatform, Shared,
        scripted_input_settings,
    };
    use recomp_core::fixtures;
    use std::collections::VecDeque;
    use std::sync::mpsc;

    const DIK_SPACE: usize = 0x39;
    const DIK_ESCAPE: usize = 0x01;

    /// The C++ `reset_movie_player`: a fresh object in place of the process globals, with the scripted seams beside it.
    struct Fixture {
        player: MoviePlayer,
        bink: Arc<Mutex<ScriptedBink>>,
        device: Arc<Mutex<ScriptedMovieDevice>>,
        platform: Arc<Mutex<ScriptedMoviePlatform>>,
        os_platform: Arc<Mutex<ScriptedOsPlatform>>,
    }

    fn settings() -> MovieSettings {
        MovieSettings {
            sound_enabled_b16178: true,
            preload_intro_sequence_b030b4: true,
            preload_size_limit_b030bc: 26_214_400,
            intro_sequence_priority_b030c4: 3,
            static_screen_wait_time_b030ac: 0.0,
            intro_sequence_b03084: Vec::new(),
        }
    }

    /// 0040F7AC..0040F7D5: `fmul` rounds at 007F before `fadd`. x C32AAAAC, scale 3F888889, width 208: 424CCCCC at 24 bits, 424CCCCA at 53.
    #[test]
    fn a_quad_rounds_by_its_thread() {
        let right = |x87| {
            let mut device = ScriptedMovieDevice::default();
            draw_movie_quad(
                &mut device,
                f32::from_bits(0xC32A_AAAC),
                0.0,
                f32::from_bits(0x3F88_8889),
                1.0,
                208,
                16,
                x87,
            );
            device
                .trace
                .iter()
                .find_map(|entry| match &entry.call {
                    MovieDeviceCall::DrawPrimitiveUp { vertices, .. } => {
                        Some(vertices[3].x.to_bits())
                    }
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(right(MovieX87::Bits24), 0x424C_CCCC);
        assert_eq!(right(MovieX87::Bits53), 0x424C_CCCA);
    }

    /// 004104FF..004105A1 on either thread, at 1024x768 with a 1280x720 movie. 0040F0BE runs on the main thread at 007F (P06-RUN-002 `S 3 ENTRY`) and each operation rounds to float: `(W - w * s) * 0.5` = C32AAAAC. The movie thread stays 027F (`T 9`), one rounding at the store: C32AAAAD.
    #[test]
    fn prepare_rounds_by_its_thread() {
        let mut fix = fixture("oblivion-recomp-movie-x87");
        fix.use_input();
        fix.services().renderer_width = 1024;
        fix.services().renderer_height = 768;
        {
            let mut bink = lock_shared(&fix.bink);
            bink.width = 1280;
            bink.height = 720;
        }
        let prepared_x = |player: &MoviePlayer| {
            let movie = MoviePlayerMovie::default();
            assert!(movie_prepare(
                player,
                &movie,
                "fixture.bik",
                true,
                false,
                false
            ));
            movie.fields().x_18.to_bits()
        };
        assert_eq!(prepared_x(&fix.player), 0xC32A_AAAC, "main thread");
        let player = fix.player.clone();
        let on_thread = std::thread::spawn(move || prepared_x(&player))
            .join()
            .unwrap();
        assert_eq!(on_thread, 0xC32A_AAAD, "movie thread");
    }

    fn fixture(video_directory: &str) -> Fixture {
        let bink = Arc::new(Mutex::new(ScriptedBink::default()));
        let device = Arc::new(Mutex::new(ScriptedMovieDevice::default()));
        let platform = Arc::new(Mutex::new(ScriptedMoviePlatform::default()));
        let os_platform = Arc::new(Mutex::new(ScriptedOsPlatform::with_devices(true, false)));
        let player = MoviePlayer::new(MoviePlayerServices {
            bink: bink.clone(),
            device: Some(device.clone()),
            platform: platform.clone(),
            os_platform: os_platform.clone(),
            renderer_width: 1920,
            renderer_height: 1080,
            os_globals: None,
            time: SharedTimeInfo::default(),
            interface: InterfaceMovieReads::default(),
            video_directory: std::env::temp_dir().join(video_directory),
            settings: settings(),
        });
        Fixture {
            player,
            bink,
            device,
            platform,
            os_platform,
        }
    }

    impl Fixture {
        fn services(&mut self) -> &mut MoviePlayerServices {
            Arc::get_mut(&mut self.player.services).expect("no thread holds the services yet")
        }

        /// `OSGlobalsData` with the input of the C++ `InputFixture`, as the movie thread sees it.
        fn use_input(&mut self) -> SharedOsInput {
            let settings = scripted_input_settings(true);
            let input = OsInputGlobals::new(
                crate::tes::os_globals::HostInstance(0x0040_0000),
                &settings,
                &mut *lock_shared(&self.os_platform),
            )
            .unwrap();
            let input = Shared::new(*input);
            self.services().os_globals = Some(MovieOsGlobals {
                input_20: Some(input.clone()),
                sound_24: None,
                main_thread_id_10: ScriptedMoviePlatform::MAIN_THREAD_ID,
                main_thread_handle_14: 0,
            });
            input
        }

        fn keyboard(&self, down: &[usize]) {
            let mut state = [0; 256];
            for &scan in down {
                state[scan] = 0x80;
            }
            lock_shared(&self.os_platform)
                .keyboard_states
                .push_back(state);
        }

        fn keyboard_polls(&self) -> usize {
            lock_shared(&self.os_platform)
                .trace
                .iter()
                .filter(|entry| {
                    matches!(
                        entry.call,
                        crate::tes::os_globals::OsCall::Poll {
                            device: ScriptedOsPlatform::KEYBOARD
                        }
                    )
                })
                .count()
        }
    }

    fn scripted_set(width: u32, height: u32) -> (ScriptedMovieDevice, Box<MovieTileSet>) {
        let mut device = ScriptedMovieDevice::default();
        let mut platform = ScriptedMoviePlatform::default();
        let set = create_movie_tile_set(
            &mut device,
            &mut platform,
            width,
            height,
            4,
            256,
            D3DFMT_X8R8G8B8,
        )
        .unwrap();
        (device, set)
    }

    /// This block's rows of `fixtures/rows/phase06.rows`, as `[run, kind, ..., text]`. Other blocks are theirs.
    fn rows(kind: &str) -> Vec<Vec<String>> {
        fixtures::rows("phase06")
            .into_iter()
            .filter_map(|row| row.strip_prefix("MOVIE|").map(str::to_owned))
            .map(|row| row.split('|').map(str::to_owned).collect::<Vec<_>>())
            .filter(|row| row[1] == kind)
            .collect()
    }

    fn evidence(run: &str) -> String {
        let file = match run {
            "P06-RUN-001" => "phase06-run001-2026-10-06.records.txt",
            "P06-RUN-002" => "phase06-run002-2026-10-06.records.txt",
            other => panic!("unknown run {other}"),
        };
        let path = fixtures::root().join("evidence").join(file);
        let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        bytes.iter().map(|&byte| char::from(byte)).collect()
    }

    /// `*` matches any character of a dump.
    fn masked_match(mask: &str, text: &str) -> bool {
        mask.len() == text.len()
            && mask
                .chars()
                .zip(text.chars())
                .all(|(wanted, got)| wanted == '*' || wanted == got)
    }

    fn contains_masked(text: &str, mask: &str) -> bool {
        text.len() >= mask.len()
            && (0..=text.len() - mask.len()).any(|at| {
                text.is_char_boundary(at)
                    && text.is_char_boundary(at + mask.len())
                    && masked_match(mask, &text[at..at + mask.len()])
            })
    }

    fn hex_words(words: &[u32]) -> String {
        let mut text = String::new();
        for word in words {
            for byte in word.to_le_bytes() {
                text.push_str(&format!("{byte:02x}"));
            }
        }
        text
    }

    /// IDA: N/A every row's last column is text of its run's evidence, and every value column is that text.
    #[test]
    fn phase06_movie_rows_match_their_evidence() {
        let lines = fixtures::rows("phase06");
        let mine: Vec<&String> = lines
            .iter()
            .filter(|row| row.starts_with("MOVIE|"))
            .collect();
        assert_eq!(mine.len(), 37);
        for row in mine {
            let columns: Vec<&str> = row.split('|').collect();
            let (run, kind, text) = (columns[1], columns[2], columns[columns.len() - 1]);
            assert!(evidence(run).contains(text), "{row} is not in {run}");
            let hex = |value: u32| hex_words(&[value]);
            let parse = |column: &str| column.parse::<u32>().unwrap();
            let wanted: Vec<String> = match kind {
                "PATHMAP" | "PATHBUILDER" | "PATHTHREAD" => vec![columns[3].to_owned()],
                "TILESET" => vec![columns[5].to_owned()],
                "OBJECT" => vec![columns[4].to_owned()],
                "HBINK" => vec![format!(
                    "{}{}{}{}",
                    hex(parse(columns[3])),
                    hex(parse(columns[4])),
                    hex(parse(columns[5])),
                    hex(parse(columns[6]))
                )],
                "SETTINGS" => vec![
                    format!("soundEnabled@B16178={} ", columns[3]),
                    format!("intro@B03084=\"{}\"", columns[4]),
                    format!("preload@B030B4={} ", columns[5]),
                    format!("limit@B030BC={} ", columns[6]),
                    format!("prio@B030C4={} ", columns[7]),
                    format!("wait@B030AC={}", columns[8]),
                ],
                "INPUT" => vec![
                    format!("flags={} ", columns[3]),
                    format!("joy={} ", columns[4]),
                    format!("dbl={}", columns[5]),
                ],
                "RENDERER" => vec![
                    format!("width={} height={}", columns[3], columns[4]),
                    format!("inScene200={} ", columns[5]),
                    format!("passes38={} ", columns[6]),
                ],
                "SOUND" => vec![format!("eax={} osg+2={}", columns[3], columns[4])],
                "ACTIVE" => vec![
                    format!("insn={} ", columns[3]),
                    format!("value={}", columns[4]),
                ],
                "STARTED" => vec![
                    format!(
                        "prio{}={} ",
                        if run == "P06-RUN-001" { "38" } else { "" },
                        columns[3]
                    ),
                    format!("B33427={}", columns[4]),
                ],
                "OPEN" => vec![format!(
                    "src=\"{}\" quiet=01 preload=01 force=00",
                    columns[4]
                )],
                "SEQUENCE_RET" => vec![format!("={}", columns[3])],
                "CONTROL" => vec![format!("ctrl={}", columns[4])],
                "TILEMENU" => vec![format!("eax={}", columns[3])],
                other => panic!("unknown kind {other}: {row}"),
            };
            for value in wanted {
                assert!(contains_masked(text, &value), "{value} is not in {row}");
            }
        }
    }

    fn set_dump(set: &MovieTileSet) -> String {
        hex_words(&[
            0,
            set.format_04,
            set.bytes_per_pixel_08,
            set.width_0c,
            set.height_10,
            set.lock_index_14,
            set.unk_18,
            set.pitch_1c,
            set.tile_count_20,
            set.full_columns_24,
            set.full_rows_28,
            set.partial_width_2c,
            set.partial_height_30,
            set.width_remainder_34,
            set.height_remainder_38,
            set.tile_size_3c,
        ])
    }

    fn movie_dump(movie: &MoviePlayerMovie) -> String {
        let fields = movie.fields();
        hex_words(&[
            0,
            0,
            0,
            fields.frames_shown_0c,
            fields.skipped_frames_10,
            fields.scale_14.to_bits(),
            fields.x_18.to_bits(),
            fields.y_1c.to_bits(),
            movie.state(),
            u32::from(fields.preparing_24),
        ])
    }

    /// C++ verify_tile_sets: the 0040F640 table and the 0040FEE0/0040F880 layout, against P06-RUN-001 and P06-RUN-002.
    #[test]
    fn verify_tile_sets() {
        for (input, output) in [
            (0, 0),
            (1, 1),
            (2, 2),
            (3, 4),
            (4, 4),
            (5, 8),
            (9, 16),
            (17, 32),
            (33, 64),
            (65, 128),
            (129, 256),
            (208, 256),
            (257, 512),
            (513, 1024),
        ] {
            assert_eq!(
                round_partial_tile_size(input),
                output,
                "0040F640 of {input}"
            );
        }
        let (mut device, mut intro) = scripted_set(720, 480);
        assert_eq!(intro.pitch_1c, 2880);
        assert_eq!(intro.tile_count_20, 6);
        assert_eq!(intro.full_columns_24, 2);
        assert_eq!(intro.full_rows_28, 1);
        assert_eq!(intro.partial_width_2c, 256);
        assert_eq!(intro.partial_height_30, 256);
        assert_eq!(intro.width_remainder_34, 208);
        assert_eq!(intro.height_remainder_38, 224);
        assert_eq!(intro.tile_size_3c, 256);
        assert_eq!(intro.format_04, D3DFMT_X8R8G8B8);
        assert_eq!(intro.bytes_per_pixel_08, 4);
        for [x, y, width, height] in [
            [0, 0, 256, 256],
            [256, 0, 256, 256],
            [512, 0, 256, 256],
            [0, 256, 256, 256],
            [256, 256, 256, 256],
            [512, 256, 256, 256],
        ] {
            let locked = lock_next_movie_tile(&mut device, &mut intro).unwrap();
            assert_eq!(
                [locked.x, locked.y, locked.width, locked.height],
                [x, y, width, height],
                "0040F880 tile rectangle"
            );
            assert_eq!(
                intro.tiles_40[locked.index].pixels[..4],
                [0; 4],
                "0040F6A0 must zero the tile"
            );
        }
        assert!(lock_next_movie_tile(&mut device, &mut intro).is_none());
        assert_eq!(intro.lock_index_14, 0, "0040F880 must reset past the end");

        let (menu_device, menu) = scripted_set(1280, 720);
        assert_eq!(menu.pitch_1c, 5120);
        assert_eq!(menu.tile_count_20, 15);
        assert_eq!(menu.full_columns_24, 5);
        assert_eq!(menu.full_rows_28, 2);
        assert_eq!(menu.partial_width_2c, 0);
        assert_eq!(menu.partial_height_30, 256);
        assert_eq!(menu.height_remainder_38, 208);
        // Partial-row tiles are 256 wide and the rounded 256 tall.
        assert_eq!(
            (menu.tiles_40[14].width, menu.tiles_40[14].height),
            (256, 256)
        );
        let created = menu_device
            .trace
            .iter()
            .rev()
            .find(|entry| matches!(entry.call, MovieDeviceCall::CreateTexture { .. }))
            .unwrap();
        assert_eq!(
            created.call,
            MovieDeviceCall::CreateTexture {
                width: 256,
                height: 256,
                levels: 1,
                usage: D3DUSAGE_DYNAMIC,
                format: D3DFMT_X8R8G8B8,
                pool: D3DPOOL_DEFAULT
            }
        );

        // The evidence rows, whole.
        let tile_rows = rows("TILESET");
        assert_eq!(tile_rows.len(), 3);
        for row in tile_rows {
            let (width, height): (u32, u32) = (row[2].parse().unwrap(), row[3].parse().unwrap());
            let (_, set) = scripted_set(width, height);
            assert!(masked_match(&row[4], &set_dump(&set)), "{row:?}");
        }

        let mut intro = Some(intro);
        destroy_movie_tile_set(&mut device, &mut intro);
        assert!(intro.is_none(), "00410110 must free the set");
        let released = device
            .trace
            .iter()
            .filter(|entry| matches!(entry.call, MovieDeviceCall::ReleaseTexture { .. }))
            .count();
        assert_eq!(released, 6);
    }

    /// 06-E-2: a tile texture that cannot be created releases the set (0040FEE0). C++ verify_tile_allocation_failure.
    #[test]
    fn verify_tile_allocation_failure() {
        let mut device = ScriptedMovieDevice::default();
        let mut platform = ScriptedMoviePlatform::default();
        assert!(
            create_movie_tile_set(&mut device, &mut platform, 720, 480, 4, 256, D3DFMT_UNKNOWN)
                .is_none(),
            "A failed tile CreateTexture must release every tile and return null"
        );
        // DEFAULT, then MANAGED, for each of the six.
        let creates = device
            .trace
            .iter()
            .filter(|entry| matches!(entry.call, MovieDeviceCall::CreateTexture { .. }))
            .count();
        assert_eq!(creates, 12);
        // A failed FormHeap request returns null before any texture.
        let mut device = ScriptedMovieDevice::default();
        let mut platform = ScriptedMoviePlatform {
            failed_allocations: vec![0x48 + 4 * 6],
            ..ScriptedMoviePlatform::default()
        };
        assert!(
            create_movie_tile_set(
                &mut device,
                &mut platform,
                720,
                480,
                4,
                256,
                D3DFMT_X8R8G8B8
            )
            .is_none()
        );
        assert!(device.trace.is_empty());
    }

    /// C++ verify_bink_sound_gate: the 00410310 gate and the 006A9B40/006A9C00 +0xA4 byte.
    #[test]
    fn verify_bink_sound_gate() {
        let mut fix = fixture("oblivion-recomp-movie-sound-gate");
        assert_eq!(configure_bink_sound(&fix.player), 0);
        assert_eq!(
            lock_shared(&fix.bink).sound_calls,
            0,
            "No OS globals must skip BinkSetSoundSystem"
        );
        fix.services().os_globals = Some(MovieOsGlobals::default());
        assert_eq!(configure_bink_sound(&fix.player), 0);
        assert_eq!(
            lock_shared(&fix.bink).sound_calls,
            0,
            "A null OSGlobals+0x24 must skip BinkSetSoundSystem"
        );
        let mut settings = scripted_input_settings(true);
        let mut platform = ScriptedOsPlatform {
            direct_sound: CreatedInterface {
                result: DSERR_NODRIVER,
                object: None,
            },
            co_initialize_result: S_FALSE,
            ..ScriptedOsPlatform::default()
        };
        let sound = OsSoundGlobals::new(HostWindow(1), &mut settings, &mut platform);
        let globals = OsGlobalsData {
            sound_24: Some(sound),
            ..OsGlobalsData::default()
        };
        fix.services().os_globals = Some(MovieOsGlobals::from(&globals));
        fix.services().settings.sound_enabled_b16178 = false;
        assert_eq!(configure_bink_sound(&fix.player), 0);
        assert_eq!(
            lock_shared(&fix.bink).sound_calls,
            0,
            "bSoundEnabled 0 must skip BinkSetSoundSystem"
        );
        fix.services().settings.sound_enabled_b16178 = true;
        assert_eq!(configure_bink_sound(&fix.player), 1);
        assert_eq!(lock_shared(&fix.bink).sound_calls, 1);
        let sound = globals.sound_24.as_ref().unwrap();
        sound.pause_game_sounds(true);
        assert_eq!(
            sound.game_sounds_paused_a4.load(),
            1,
            "006A9B40 must set +0xA4"
        );
        sound.resume_game_sounds(false);
        assert_eq!(
            sound.game_sounds_paused_a4.load(),
            0,
            "006A9C00 must clear +0xA4"
        );
    }

    /// C++ verify_priority_and_thread_queries: 0040FE00 mapping and store; 0040FDA0/0040FDD0/0040FD90.
    #[test]
    fn verify_priority_and_thread_queries() {
        let fix = fixture("oblivion-recomp-movie-priority");
        let globals = &fix.player.globals;
        let thread = MovieThreadHandle::spawn_suspended("idle", || {}).unwrap();
        *lock_shared(&globals.thread_b33434) = Some(thread.clone());
        for (priority, value) in [
            (0, -2),
            (1, -1),
            (2, 0),
            (3, 1),
            (4, 2),
            (5, 15),
            (6, 0),
            (-1, 0),
        ] {
            assert!(set_movie_thread_priority(&fix.player, priority));
            assert_eq!(thread.priority(), value, "0040FE00 mapping of {priority}");
            assert_eq!(
                globals.thread_priority_b33438.load(Ordering::Acquire),
                priority
            );
        }
        assert!(
            movie_player_is_running(&fix.player),
            "A suspended thread is still active (0040FDA0)"
        );
        movie_player_request_skip(&fix.player);
        assert_eq!(
            globals.skip_requested_b33426.load(Ordering::Acquire),
            1,
            "0040FD90 must set 00B33426"
        );
        thread.resume();
        assert_eq!(movie_player_wait_for_thread(&fix.player), WAIT_OBJECT_0);
        assert!(
            lock_shared(&globals.thread_b33434).is_none(),
            "0040FDD0 must clear hThread"
        );
        assert_eq!(globals.waiter_active_b33425.load(Ordering::Acquire), 0);
        assert!(!movie_player_is_running(&fix.player));
        assert_eq!(
            movie_player_wait_for_thread(&fix.player),
            0,
            "Without hThread 0040FDD0 returns 0"
        );
    }

    /// C++ verify_playback, the data part: 004104C0 scale and centre, 00410160 flags and failures, 004106C0's loop.
    #[test]
    fn verify_playback() {
        let directory = "oblivion-recomp-movie-playback";
        let mut fix = fixture(directory);
        let bink = fix.bink.clone();
        let movie = MoviePlayerMovie::default();
        assert!(movie_prepare(
            &fix.player,
            &movie,
            "fixture.bik",
            true,
            false,
            false
        ));
        {
            let fields = movie.fields();
            assert_eq!(fields.scale_14, 2.25, "004104C0 height fit (P06-RUN-001)");
            assert_eq!(fields.x_18, 150.0);
            assert_eq!(fields.y_1c, 0.0);
            assert!(fields.device_04);
            assert!(fields.tiles_08.is_some());
        }
        assert_eq!(lock_shared(&bink).opens.last().unwrap().1, 0);
        movie_close(&fix.player, &movie);
        {
            let fields = movie.fields();
            assert!(fields.bink_00.is_none());
            assert_eq!(fields.scale_14, 1.0, "004102C0 must reset the object");
            assert!(fields.tiles_08.is_none());
        }
        assert_eq!(movie.state(), 0);

        assert!(movie_prepare(
            &fix.player,
            &movie,
            "fixture.bik",
            true,
            false,
            true
        ));
        {
            let fields = movie.fields();
            assert_eq!(fields.scale_14, (1920.0f64 / 720.0) as f32);
            assert_eq!(fields.x_18, 0.0);
            assert_eq!(
                fields.y_1c,
                ((1080.0 - f64::from(fields.scale_14) * 480.0) * 0.5) as f32,
                "004104C0 width fit"
            );
        }
        movie_close(&fix.player, &movie);

        // 00410160: preload by size, the forced preload, the off-main-thread flag.
        let root = std::env::temp_dir().join(directory);
        std::fs::create_dir_all(&root).unwrap();
        let stub = root.join("oblivion-recomp-phase06.bik");
        std::fs::write(&stub, vec![b'B'; 312]).unwrap();
        let stub_name = "oblivion-recomp-phase06.bik";
        fix.services().os_globals = Some(MovieOsGlobals {
            main_thread_id_10: ScriptedMoviePlatform::MAIN_THREAD_ID,
            ..MovieOsGlobals::default()
        });
        lock_shared(&bink).opens.clear();
        assert!(movie_open(
            &fix.player,
            &movie,
            stub_name,
            true,
            true,
            false
        ));
        assert_eq!(
            lock_shared(&bink).opens.last().unwrap(),
            &(stub.clone(), BINK_PRELOAD_ALL),
            "A preloaded file within the limit must open with 0x2000"
        );
        movie_close(&fix.player, &movie);
        fix.services().settings.preload_size_limit_b030bc = 100;
        assert!(movie_open(
            &fix.player,
            &movie,
            stub_name,
            true,
            true,
            false
        ));
        assert_eq!(
            lock_shared(&bink).opens.last().unwrap().1,
            0,
            "A file over iPreloadSizeLimit must not preload"
        );
        movie_close(&fix.player, &movie);
        assert!(movie_open(
            &fix.player,
            &movie,
            stub_name,
            true,
            false,
            true
        ));
        assert_eq!(
            lock_shared(&bink).opens.last().unwrap().1,
            BINK_PRELOAD_ALL,
            "A forced preload ignores the limit"
        );
        movie_close(&fix.player, &movie);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                assert!(movie_open(
                    &fix.player,
                    &movie,
                    stub_name,
                    true,
                    false,
                    false
                ));
            });
        });
        assert_eq!(
            lock_shared(&bink).opens.last().unwrap().1,
            BINK_NO_THREADED_IO,
            "Off the main thread the open must add 0x8000000"
        );
        // The original's bracket: SuspendThread and ResumeThread on +0x14 (a null handle in P06-RUN-001).
        assert_eq!(
            lock_shared(&fix.platform).suspends,
            [(0x0041_0246, 0)],
            "00410246 off the main thread only"
        );
        assert_eq!(lock_shared(&fix.platform).resumes, [(0x0041_0299, 0)]);
        movie_close(&fix.player, &movie);
        std::fs::remove_file(&stub).unwrap();
        let _ = std::fs::remove_dir(&root);

        lock_shared(&bink).fail_open = true;
        movie.set_state(1);
        assert!(!movie_open(
            &fix.player,
            &movie,
            "missing.bik",
            true,
            false,
            false
        ));
        assert_eq!(movie.state(), 0, "A failed open must clear the state");
        assert!(
            lock_shared(&fix.platform).warnings.is_empty(),
            "A quiet failed open must not warn"
        );
        assert!(!movie_open(
            &fix.player,
            &movie,
            "missing.bik",
            false,
            false,
            false
        ));
        assert_eq!(
            lock_shared(&fix.platform).warnings,
            ["Could not open missing.bik for playback."]
        );
        lock_shared(&bink).fail_open = false;
        fix.services().os_globals = None;

        // 00410A70/004106C0: 3 frames, one early wait (Sleep), copies per tile.
        {
            let mut script = lock_shared(&bink);
            script.frames = 3;
            script.wait_script = VecDeque::from([1, 0, 0, 0]);
            script.copy_result = 0;
            script.copies.clear();
        }
        let played = MoviePlayerMovie::default();
        assert!(
            movie_play(
                &fix.player,
                &played,
                "three.bik",
                false,
                true,
                false,
                false,
                false
            ),
            "00410A70 must show the three frames and close"
        );
        {
            let script = lock_shared(&bink);
            assert_eq!(script.do_frames, 3);
            assert_eq!(script.next_frames, 3);
            assert_eq!(played.fields().frames_shown_0c, 0);
            assert_eq!(script.copies.len(), 18);
            assert_eq!(script.copies[0].destination_height, 480);
            assert_eq!(script.copies[0].destination_x, 0);
            assert_eq!(script.copies[0].destination_y, 0);
            assert_eq!(script.copies[2].source_x, 512);
            assert_eq!(script.copies[2].source_width, 256);
            assert_eq!(script.copies[5].source_y, 256);
            assert_eq!(script.copies[0].flags, 3, "flags 3");
        }
        assert!(lock_shared(&fix.platform).debug_logs.is_empty());
        assert_eq!(lock_shared(&fix.platform).sleeps, [1]);

        {
            let mut script = lock_shared(&bink);
            script.frames = 1;
            script.wait_script.clear();
            script.copy_result = 1;
        }
        let skipped = MoviePlayerMovie::default();
        assert!(movie_play(
            &fix.player,
            &skipped,
            "skip.bik",
            false,
            true,
            false,
            false,
            false
        ));
        assert_eq!(
            lock_shared(&fix.platform).debug_logs,
            ["Bink playback: Skipped frame #0.  Total skipped: 1"],
            "A skipped copy must log the frame number and the count"
        );

        // loop = 1 keeps 004106C0 returning 1.
        lock_shared(&bink).copy_result = 0;
        let looping = MoviePlayerMovie::default();
        assert!(movie_prepare(
            &fix.player,
            &looping,
            "loop.bik",
            true,
            false,
            false
        ));
        let mut presented = 0;
        while movie_present_frame(&fix.player, &looping, false, true) {
            presented += 1;
            if presented == 5 {
                break;
            }
        }
        assert_eq!(presented, 5, "loop = 1 must keep 004106C0 returning 1");
        movie_close(&fix.player, &looping);
    }

    /// The object bytes P06-RUN-001 and P06-RUN-002 dumped, whole, from the port's 004104C0 and 004102C0.
    #[test]
    fn movie_objects_match_the_runs() {
        let fix = fixture("oblivion-recomp-movie-objects");
        let movie = MoviePlayerMovie::default();
        // 00410A79 and 00410AA6 run first in 00410A70, and 4105E3 reads them inside 004104C0.
        movie.set_state(1);
        movie.fields().preparing_24 = 1;
        assert!(movie_prepare(
            &fix.player,
            &movie,
            "fixture.bik",
            true,
            false,
            false
        ));
        let dump = movie_dump(&movie);
        let prepared = rows("OBJECT");
        let prepared: Vec<_> = prepared.iter().filter(|row| row[2] == "PREPARED").collect();
        assert_eq!(prepared.len(), 2);
        for row in prepared {
            assert!(masked_match(&row[3], &dump), "{row:?}: {dump}");
        }
        movie_close(&fix.player, &movie);
        let dump = movie_dump(&movie);
        let closed = rows("OBJECT");
        let closed: Vec<_> = closed.iter().filter(|row| row[2] == "CLOSED").collect();
        assert_eq!(closed.len(), 2);
        for row in closed {
            assert!(masked_match(&row[3], &dump), "{row:?}: {dump}");
        }

        // Map loop.bik (P06-RUN-001 M 57): 1280x720 by height at 1920x1080, menu-owned, past the prepare.
        recomp_core::fixtures::ledger_relaxed(
            "phase06",
            "MOVIE|P06-RUN-001|HBINK",
            1,
            "the Bink handle's size and counts: the host's script takes them as inputs, and nothing in the port decodes the file",
        );
        recomp_core::fixtures::ledger_relaxed(
            "phase06",
            "MOVIE|P06-RUN-002|HBINK",
            1,
            "the Bink handle's size and counts: the host's script takes them as inputs, and nothing in the port decodes the file",
        );
        let hbinks = rows("HBINK");
        let map_loop = hbinks.iter().find(|row| row[2] == "1280").unwrap();
        {
            let mut script = lock_shared(&fix.bink);
            script.width = map_loop[2].parse().unwrap();
            script.height = map_loop[3].parse().unwrap();
            script.frames = map_loop[4].parse().unwrap();
        }
        let menu = MoviePlayerMovie::default();
        assert!(movie_prepare(
            &fix.player,
            &menu,
            "Map loop.bik",
            true,
            true,
            false
        ));
        menu.set_state(MOVIE_STATE_MENU_OWNED);
        let dump = movie_dump(&menu);
        let row = rows("OBJECT");
        let row = row.iter().find(|row| row[2] == "MENU").unwrap();
        assert!(masked_match(&row[3], &dump), "{row:?}: {dump}");
        movie_close(&fix.player, &menu);

        // The script's default movie is the intro stub the runs opened.
        let script = ScriptedBink::default();
        // Both runs recorded it.
        let intros: Vec<_> = hbinks.iter().filter(|row| row[2] != "1280").collect();
        assert_eq!(intros.len(), 2);
        for intro in intros {
            assert_eq!(script.width.to_string(), intro[2]);
            assert_eq!(script.height.to_string(), intro[3]);
            assert_eq!(script.frames.to_string(), intro[4]);
        }
    }

    /// What the thread saw at its start in P06-RUN-001: the renderer's size and scene state, no default shader pass (the host
    /// keeps none, so `set_default_shader_pass_fvf` has nothing to do), and an empty 1001 slot.
    #[test]
    fn thread_view_matches_run_001() {
        let fix = fixture("oblivion-recomp-movie-view");
        let row = &rows("RENDERER")[0];
        assert_eq!(fix.player.services.renderer_width.to_string(), row[2]);
        assert_eq!(fix.player.services.renderer_height.to_string(), row[3]);
        assert_eq!(
            u32::from(HostMoviePlatform::default().renderer_in_scene(0x0041_0757)).to_string(),
            row[4]
        );
        assert_eq!(row[5], "0");
        let row = &rows("TILEMENU")[0];
        assert_eq!(
            u32::from(
                fix.player
                    .services
                    .interface
                    .tile_menu_present(MENU_ID_MESSAGE)
            )
            .to_string(),
            row[2]
        );
    }

    /// C++ verify_play_movie_objects: 00410BA0/00410340/00410B00 current, paused and cached objects.
    #[test]
    fn verify_play_movie_objects() {
        let fix = fixture("oblivion-recomp-movie-objects-play");
        let globals = &fix.player.globals;
        let outer = Arc::new(MoviePlayerMovie::default());
        let outer_bink = lock_shared(&fix.bink).open(Path::new("outer.bik"), 0);
        outer.fields().bink_00 = outer_bink;
        *lock_shared(&globals.current_b33428) = Some(outer.clone());
        assert!(
            play_movie(
                &fix.player,
                "inner.bik",
                false,
                true,
                false,
                false,
                false,
                false
            ),
            "Nested PlayMovie must play"
        );
        assert_eq!(
            lock_shared(&fix.bink).pauses,
            [1, 0],
            "00410340 must pause the current movie and 00410B00 must resume it"
        );
        let current = lock_shared(&globals.current_b33428).clone().unwrap();
        assert!(Arc::ptr_eq(&current, &outer));
        assert!(lock_shared(&globals.paused_b3342c).is_none());
        lock_shared(&fix.bink).close(outer_bink.unwrap());
        *lock_shared(&globals.current_b33428) = None;

        *lock_shared(&globals.cached_b33440) = Some(Arc::new(MoviePlayerMovie::default()));
        globals.cached_valid_b33431.store(1, Ordering::Release);
        assert!(play_movie(
            &fix.player,
            "cached.bik",
            false,
            true,
            false,
            false,
            false,
            false
        ));
        assert!(
            lock_shared(&globals.cached_b33440).is_none(),
            "A valid cached object must be used and freed"
        );
        assert_eq!(globals.cached_valid_b33431.load(Ordering::Acquire), 0);
        assert!(lock_shared(&globals.current_b33428).is_none());
    }

    /// C++ verify_sequence: 00410D10 tokenization, the skip flag and the suffix dispatch.
    #[test]
    fn verify_sequence() {
        let fix = fixture("oblivion-recomp-movie-sequence");
        let globals = &fix.player.globals;
        play_movie_sequence(&fix.player, b"ik,a.bik,b.BIK,c.dds,d.txt,e.bi");
        {
            let script = lock_shared(&fix.bink);
            assert_eq!(
                script.opens.len(),
                2,
                "00410D10 must play only the .bik tokens"
            );
            assert!(script.opens[0].0.ends_with("a.bik"));
            assert!(script.opens[1].0.ends_with("b.BIK"));
            assert_eq!(script.opens[0].1, 0);
        }
        // c.dds reached 00410840, whose load fails as a missing file does, so it returned before drawing.
        let splash_loads = lock_shared(&fix.device)
            .trace
            .iter()
            .filter(|entry| matches!(entry.call, MovieDeviceCall::CreateTextureFromFile { .. }))
            .count();
        assert_eq!(splash_loads, 1);
        assert_eq!(
            globals.skip_requested_b33426.load(Ordering::Acquire),
            0,
            "00410E14 clears the skip flag"
        );
        lock_shared(&fix.bink).opens.clear();
        globals.skip_requested_b33426.store(1, Ordering::Release);
        play_movie_sequence(&fix.player, b"x.bik,y.bik");
        assert!(
            lock_shared(&fix.bink).opens.is_empty(),
            "A set skip flag must skip every token"
        );
        assert_eq!(globals.skip_requested_b33426.load(Ordering::Acquire), 0);
    }

    /// C++ verify_continue_test: 00410390.
    #[test]
    fn verify_continue_test() {
        let mut fix = fixture("oblivion-recomp-movie-continue");
        assert!(
            movie_should_continue(&fix.player, false),
            "No skip, no menu, no poll: continue"
        );
        fix.player
            .globals
            .skip_requested_b33426
            .store(1, Ordering::Release);
        assert!(
            !movie_should_continue(&fix.player, false),
            "The skip flag stops"
        );
        fix.player
            .globals
            .skip_requested_b33426
            .store(0, Ordering::Release);
        let current = Arc::new(MoviePlayerMovie::default());
        current.set_state(1);
        *lock_shared(&fix.player.globals.current_b33428) = Some(current.clone());
        // D6: 00B3A6E0 and the TileMenu table (slot 1001 - 1001).
        let reads = fix.player.services.interface.clone();
        reads.publish_interface_manager(true);
        assert!(
            movie_should_continue(&fix.player, false),
            "No Message menu (1001): continue"
        );
        reads.set_tile_menu(MENU_ID_MESSAGE, true);
        assert!(
            !movie_should_continue(&fix.player, false),
            "An open Message menu (1001) stops the movie"
        );
        current.set_state(MOVIE_STATE_MENU_OWNED);
        assert!(
            movie_should_continue(&fix.player, false),
            "A menu-owned movie (state 2) skips the menu query"
        );
        current.set_state(1);
        reads.publish_interface_manager(false);
        assert!(
            movie_should_continue(&fix.player, false),
            "Without an InterfaceManager: continue"
        );
        reads.set_tile_menu(MENU_ID_MESSAGE, false);
        *lock_shared(&fix.player.globals.current_b33428) = None;

        // Activate (Space) just pressed with a running thread sets the skip flag.
        fix.use_input();
        let globals = &fix.player.globals;
        let thread = MovieThreadHandle::spawn_suspended("idle", || {}).unwrap();
        *lock_shared(&globals.thread_b33434) = Some(thread.clone());
        assert!(movie_should_continue(&fix.player, true), "No key: continue");
        fix.keyboard(&[DIK_SPACE]);
        let polls = fix.keyboard_polls();
        assert!(
            !movie_should_continue(&fix.player, true),
            "Activate must stop"
        );
        assert_eq!(globals.skip_requested_b33426.load(Ordering::Acquire), 1);
        assert_eq!(fix.keyboard_polls(), polls + 2, "and poll twice");
        globals.skip_requested_b33426.store(0, Ordering::Release);
        fix.keyboard(&[]);
        assert!(
            movie_should_continue(&fix.player, true),
            "Released key: continue"
        );
        fix.keyboard(&[DIK_ESCAPE]);
        assert!(
            !movie_should_continue(&fix.player, true),
            "Escape (control 29) must stop"
        );
        assert_eq!(globals.skip_requested_b33426.load(Ordering::Acquire), 1);
        thread.resume();
        thread.wait();
        *lock_shared(&globals.thread_b33434) = None;
        globals.skip_requested_b33426.store(0, Ordering::Release);
        fix.keyboard(&[]);
        assert!(movie_should_continue(&fix.player, true), "Release");
        fix.keyboard(&[DIK_SPACE]);
        assert!(
            !movie_should_continue(&fix.player, true),
            "Without a thread the key stops"
        );
        assert_eq!(
            globals.skip_requested_b33426.load(Ordering::Acquire),
            0,
            "but leaves the skip flag"
        );
    }

    /// C++ verify_thread: 00410E40/00410E20 on a real thread with the scripted library. The C++ `Sleep(200)` is the script's
    /// semaphore, which its open releases.
    #[test]
    fn verify_thread() {
        let fix = fixture("oblivion-recomp-movie-thread");
        let globals = &fix.player.globals;
        start_async_movie_sequence(&fix.player, b"first.bik,second.bik", false, -1);
        assert_eq!(
            globals.thread_priority_b33438.load(Ordering::Acquire),
            3,
            "-1 must use iIntroSequencePriority (3)"
        );
        let owner = lock_shared(&globals.host_thread_owner).clone().unwrap();
        owner.wait();
        assert!(
            lock_shared(&globals.thread_b33434).is_none(),
            "The thread must clear hThread"
        );
        {
            let script = lock_shared(&fix.bink);
            assert_eq!(script.opens.len(), 2, "The thread must play both movies");
            assert!(script.opens[1].0.ends_with("second.bik"));
        }
        movie_player_shutdown(&fix.player);
        assert!(
            lock_shared(&globals.host_thread_owner).is_none(),
            "Shutdown closes the owned handle"
        );

        // 06-E-2: closing during an intro: the host shutdown sets the skip flag, the continue test stops the endless movie
        // and the thread exits.
        let endless = fixture("oblivion-recomp-movie-thread-endless");
        let opened = {
            let mut script = lock_shared(&endless.bink);
            script.frames = 0x7FFF_FFFF;
            Arc::clone(&script.opened)
        };
        let globals = &endless.player.globals;
        start_async_movie_sequence(&endless.player, b"endless.bik,never.bik", false, -1);
        opened.acquire();
        movie_player_shutdown(&endless.player);
        assert!(lock_shared(&globals.host_thread_owner).is_none());
        assert!(lock_shared(&globals.thread_b33434).is_none());
        {
            let script = lock_shared(&endless.bink);
            assert_eq!(
                script.opens.len(),
                1,
                "Shutdown during an intro must skip the rest"
            );
            assert_eq!(script.closes, 1, "and close the movie");
        }
        assert_eq!(globals.skip_requested_b33426.load(Ordering::Acquire), 0);
    }

    /// The thread reaches the device only through the queue, and the main thread reads it in the thread's order.
    #[test]
    fn thread_device_calls_arrive_by_queue() {
        let (sender, receiver) = mpsc::channel();
        let mut fix = fixture("oblivion-recomp-movie-queue");
        fix.services().device = Some(Arc::new(Mutex::new(QueuedMovieDevice::new(sender))));
        start_async_movie_sequence(&fix.player, b"one.bik", false, -1);
        lock_shared(&fix.player.globals.host_thread_owner)
            .clone()
            .unwrap()
            .wait();
        movie_player_shutdown(&fix.player);
        drop(fix);
        let entries: Vec<MovieQueueEntry> = receiver.iter().collect();
        assert_eq!(
            entries[0].entry.call,
            MovieDeviceCall::TestCooperativeLevel,
            "00410390 first"
        );
        let presents = entries
            .iter()
            .filter(|entry| entry.entry.call == MovieDeviceCall::Present)
            .count();
        assert_eq!(presents, 1, "the one-frame stub");
        // Each of the six tiles comes as a zeroed upload, and then with its copied frame.
        let copied = entries
            .iter()
            .filter(|entry| entry.entry.site == 0x0040_FD32)
            .map(|entry| entry.tile_bytes.as_ref().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(copied.len(), 6);
        assert!(
            copied
                .iter()
                .all(|bytes| { bytes[..4] == ScriptedBink::COPIED_PIXEL.to_le_bytes() })
        );
    }

    /// 004106C0's frame on the device: the C++ test has no check of this order. Compared with the disassembly only.
    #[test]
    fn frame_calls_in_order() {
        let fix = fixture("oblivion-recomp-movie-frame");
        let movie = MoviePlayerMovie::default();
        assert!(movie_prepare(
            &fix.player,
            &movie,
            "fixture.bik",
            true,
            false,
            false
        ));
        lock_shared(&fix.device).trace.clear();
        assert!(!movie_present_frame(&fix.player, &movie, false, false));
        let trace = lock_shared(&fix.device).trace.clone();
        let sites: Vec<u32> = trace.iter().map(|entry| entry.site).collect();
        assert_eq!(sites[0], 0x0041_03F5, "00410390 first");
        assert_eq!(sites[1], 0x0040_F8AB, "the first tile's lock");
        let clear = sites.iter().position(|&site| site == 0x0041_0755).unwrap();
        assert_eq!(sites[clear + 1], 0x0041_076C, "BeginScene");
        assert_eq!(sites[sites.len() - 2], 0x0041_07AD, "EndScene");
        assert_eq!(sites[sites.len() - 1], 0x0041_07C6, "Present");
        // Six quads, the first at x = 150 and 256 * 2.25 wide (P06-RUN-001's scale and x).
        let draws: Vec<&MovieDeviceEntry> = trace
            .iter()
            .filter(|entry| entry.site == 0x0040_F86F)
            .collect();
        assert_eq!(draws.len(), 6);
        let MovieDeviceCall::DrawPrimitiveUp { vertices, .. } = &draws[0].call else {
            panic!("a draw");
        };
        assert_eq!(vertices[0].x, 150.0);
        assert_eq!(vertices[3].x, 726.0);
        assert_eq!(vertices[3].y, 576.0);
        // 00768960 once per frame, as the disassembly.
        assert_eq!(lock_shared(&fix.platform).fvf_calls, [MOVIE_QUAD_FVF]);
        // A renderer in a scene skips both.
        lock_shared(&fix.platform).in_scene = true;
        lock_shared(&fix.device).trace.clear();
        movie_present_frame(&fix.player, &movie, false, false);
        let trace = lock_shared(&fix.device).trace.clone();
        assert!(
            trace
                .iter()
                .all(|entry| { entry.site != 0x0041_076C && entry.site != 0x0041_07AD })
        );
        // A menu-owned movie does not present.
        movie.set_state(MOVIE_STATE_MENU_OWNED);
        lock_shared(&fix.device).trace.clear();
        movie_present_frame(&fix.player, &movie, false, false);
        let trace = lock_shared(&fix.device).trace.clone();
        assert!(trace.iter().all(|entry| entry.site != 0x0041_07C6));
    }

    /// 00410840 over the seam: the host decode is M6, but the order and the arithmetic run here. Compared with the C++ only.
    #[test]
    fn splash_order_without_the_decode() {
        let mut fix = fixture("oblivion-recomp-movie-splash");
        assert_ne!(show_movie_splash(&fix.player, "missing.dds"), 0);
        lock_shared(&fix.device).trace.clear();
        lock_shared(&fix.device).splash_size = Some((8, 4));
        fix.services().settings.static_screen_wait_time_b030ac = 3.0;
        assert_eq!(show_movie_splash(&fix.player, "present.dds"), 0);
        let trace = lock_shared(&fix.device).trace.clone();
        let draw = trace
            .iter()
            .find(|entry| entry.site == 0x0040_F86F)
            .unwrap();
        let MovieDeviceCall::DrawPrimitiveUp { vertices, .. } = &draw.call else {
            panic!("a draw");
        };
        // scale = 1080 / 4 = 270, x = (1920 - 270 * 8) * 0.5 = -120, width = 1920 - (x + x) = 2160.
        assert_eq!(vertices[0].x, -120.0);
        assert_eq!(vertices[3].x, 2040.0);
        assert_eq!(vertices[3].y, 1080.0);
        assert_eq!(
            trace.last().unwrap().site,
            0x0041_0A53,
            "the Release ends it"
        );
        // deadline = 1000 + 3000 from a clock of 1000 a read: 2000, 3000 and 4000 wait until `deadline > now` fails.
        assert_eq!(lock_shared(&fix.platform).tick, 4000);
    }

    /// The real intro files through 00410160's stat and flags, on the movie thread's path (off the main thread).
    /// P06-RUN-001 reads no flags; the files are the stubs of 312 bytes that its OPEN rows name.
    #[test]
    fn intro_files_preload_in_real_data() {
        let names = rows("OPEN");
        let mut fix = fixture("unused");
        fix.services().video_directory = fixtures::data_dir().join("Video");
        fix.services().os_globals = Some(MovieOsGlobals {
            main_thread_id_10: ScriptedMoviePlatform::MAIN_THREAD_ID,
            ..MovieOsGlobals::default()
        });
        for row in names {
            let movie = MoviePlayerMovie::default();
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    assert!(movie_open(&fix.player, &movie, &row[3], true, true, false));
                });
            });
            let script = lock_shared(&fix.bink);
            let (path, flags) = script.opens.last().unwrap();
            assert!(path.is_file(), "{path:?} from {row:?}");
            assert_eq!(*flags, BINK_PRELOAD_ALL | BINK_NO_THREADED_IO);
        }
    }
}
