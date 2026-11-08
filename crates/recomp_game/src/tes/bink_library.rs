//! Phase 06 rows 06-C-1 and 06-C-5: the host seam for the binkw32 calls of the MoviePlayer. No decoder exists here:
//! `recomp_host::ffmpeg_bink` is the implementation in the real app (the recorded substitution for binkw32, which is x86-only).
//! Frames and sound come through it, so `ScriptedBink` stands in for the movie files in tests.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::movie_player::HostSemaphore;

/// IDA: N/A a host HBINK. Never compared with evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BinkHandle(pub u32);

/// IDA: the HBINK fields the MoviePlayer reads (P06-RUN-001 dumps).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BinkFrameInfo {
    pub width_00: u32,
    pub height_04: u32,
    pub frames_08: u32,
    pub frame_num_0c: u32,
}

/// IDA: N/A each method names the binkw32 export the original imports at 00A28330..00A28350. The worker thread reaches it,
/// so an implementation must be `Send`. Completion: 100% of its scoped role.
pub trait BinkLibrary: Send {
    /// IDA: see the words here, which name the addresses this relates to.
    /// _BinkSetSoundSystem@8(_BinkOpenDirectSound@4, 0) (00410324..0041032D).
    fn set_direct_sound_system(&mut self) -> i32;
    /// IDA: see the words here, which name the addresses this relates to.
    /// _BinkOpen@8 (00410258); `None` on failure.
    fn open(&mut self, path: &Path, flags: u32) -> Option<BinkHandle>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// _BinkClose@4.
    fn close(&mut self, bink: BinkHandle);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// _BinkWait@4: nonzero means not due yet.
    fn wait(&mut self, bink: BinkHandle) -> i32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// _BinkDoFrame@4.
    fn do_frame(&mut self, bink: BinkHandle) -> i32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// _BinkNextFrame@4.
    fn next_frame(&mut self, bink: BinkHandle);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// _BinkPause@8.
    fn pause(&mut self, bink: BinkHandle, pause: i32) -> i32;
    /// IDA: see the words here, which name the addresses this relates to.
    /// _BinkCopyToBufferRect@44: nonzero means the copy was skipped. `destination` is the locked tile.
    #[allow(clippy::too_many_arguments)] // 0040FD18 _BinkCopyToBufferRect@44's own eleven arguments.
    fn copy_to_buffer_rect(
        &mut self,
        bink: BinkHandle,
        destination: &mut [u8],
        pitch: i32,
        destination_height: u32,
        destination_x: u32,
        destination_y: u32,
        source_x: u32,
        source_y: u32,
        source_width: u32,
        source_height: u32,
        flags: u32,
    ) -> i32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// The HBINK fields of `bink`.
    fn info(&self, bink: BinkHandle) -> BinkFrameInfo;
}

/// IDA: N/A one recorded `copy_to_buffer_rect`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BinkCopy {
    pub destination_height: u32,
    pub destination_x: u32,
    pub destination_y: u32,
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub flags: u32,
}

/// IDA: N/A the seam's script (the C++ `FakeBink`): scripted frames and every call recorded. `opened` lets a test wait for
/// a thread's open rather than sleep.
pub struct ScriptedBink {
    pub opens: Vec<(PathBuf, u32)>,
    pub wait_script: VecDeque<i32>,
    pub copies: Vec<BinkCopy>,
    pub pauses: Vec<i32>,
    pub sound_calls: u32,
    pub do_frames: u32,
    pub next_frames: u32,
    pub closes: u32,
    pub copy_result: i32,
    pub fail_open: bool,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub opened: Arc<HostSemaphore>,
    movies: Vec<(BinkHandle, BinkFrameInfo)>,
    next_handle: u32,
}

impl Default for ScriptedBink {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            opens: Vec::new(),
            wait_script: VecDeque::new(),
            copies: Vec::new(),
            pauses: Vec::new(),
            sound_calls: 0,
            do_frames: 0,
            next_frames: 0,
            closes: 0,
            copy_result: 0,
            fail_open: false,
            width: 720,
            height: 480,
            frames: 1,
            opened: Arc::new(HostSemaphore::new(0)),
            movies: Vec::new(),
            next_handle: 1,
        }
    }
}

impl ScriptedBink {
    /// The value a skipped-frame fixture writes, as the C++ fake does: one pixel of the locked tile.
    pub const COPIED_PIXEL: u32 = 0x00FF_8040;

    // IDA: N/A a helper, with no address of its own.
    fn movie(&mut self, bink: BinkHandle) -> &mut BinkFrameInfo {
        let (_, info) = self
            .movies
            .iter_mut()
            .find(|(handle, _)| *handle == bink)
            .expect("a BinkHandle this script opened and has not closed");
        info
    }
}

impl BinkLibrary for ScriptedBink {
    /// IDA: N/A `ScriptedBink`'s side of `BinkLibrary`.
    fn set_direct_sound_system(&mut self) -> i32 {
        self.sound_calls += 1;
        1
    }
    /// IDA: N/A `ScriptedBink`'s side of `BinkLibrary`.
    fn open(&mut self, path: &Path, flags: u32) -> Option<BinkHandle> {
        self.opens.push((path.to_path_buf(), flags));
        self.opened.release();
        if self.fail_open {
            return None;
        }
        let bink = BinkHandle(self.next_handle);
        self.next_handle += 1;
        self.movies.push((
            bink,
            BinkFrameInfo {
                width_00: self.width,
                height_04: self.height,
                frames_08: self.frames,
                frame_num_0c: 1,
            },
        ));
        Some(bink)
    }
    /// IDA: N/A `ScriptedBink`'s side of `BinkLibrary`.
    fn close(&mut self, bink: BinkHandle) {
        self.closes += 1;
        self.movies.retain(|(handle, _)| *handle != bink);
    }
    /// IDA: N/A `ScriptedBink`'s side of `BinkLibrary`.
    fn wait(&mut self, _bink: BinkHandle) -> i32 {
        self.wait_script.pop_front().unwrap_or(0)
    }
    /// IDA: N/A `ScriptedBink`'s side of `BinkLibrary`.
    fn do_frame(&mut self, _bink: BinkHandle) -> i32 {
        self.do_frames += 1;
        0
    }
    /// IDA: N/A `ScriptedBink`'s side of `BinkLibrary`.
    fn next_frame(&mut self, bink: BinkHandle) {
        self.next_frames += 1;
        let info = self.movie(bink);
        info.frame_num_0c = if info.frame_num_0c >= info.frames_08 {
            1
        } else {
            info.frame_num_0c + 1
        };
    }
    /// IDA: N/A `ScriptedBink`'s side of `BinkLibrary`.
    fn pause(&mut self, _bink: BinkHandle, pause: i32) -> i32 {
        self.pauses.push(pause);
        pause
    }
    /// IDA: N/A `ScriptedBink`'s side of `BinkLibrary`.
    fn copy_to_buffer_rect(
        &mut self,
        _bink: BinkHandle,
        destination: &mut [u8],
        _pitch: i32,
        destination_height: u32,
        destination_x: u32,
        destination_y: u32,
        source_x: u32,
        source_y: u32,
        source_width: u32,
        source_height: u32,
        flags: u32,
    ) -> i32 {
        self.copies.push(BinkCopy {
            destination_height,
            destination_x,
            destination_y,
            source_x,
            source_y,
            source_width,
            source_height,
            flags,
        });
        destination[..4].copy_from_slice(&Self::COPIED_PIXEL.to_le_bytes());
        self.copy_result
    }
    /// IDA: N/A `ScriptedBink`'s side of `BinkLibrary`.
    fn info(&self, bink: BinkHandle) -> BinkFrameInfo {
        self.movies
            .iter()
            .find(|(handle, _)| *handle == bink)
            .map_or(BinkFrameInfo::default(), |(_, info)| *info)
    }
}
