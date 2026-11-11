//! IDA: N/A the recorded substitution for binkw32: `BinkLibrary` over `/usr/bin/ffmpeg`, as the C++ x64 build did. binkw32 is x86-only, so no 64-bit build can call it. Each open starts a decoder process whose raw BGRA frames a reader thread keeps a small queue of, so `BinkDoFrame` doesn't stall its caller on the decoder. A subprocess rather than the FFmpeg libraries, which would link system libraries: this adds no dependency.
//!
//! What differs from binkw32, and is recorded: `BinkCopyToBufferRect` always copies the whole rectangle, so no frame is reported skipped and the flags aren't read; YUV to RGB is FFmpeg's matrix, not Bink's; `BinkOpen`'s preload and threaded-IO flags have no effect; and the movie's own sound isn't played, which no movie before the MainMenu has (`2k games.bik` and the rest of `sIntroSequence`, `Oblivion iv logo.bik` and `Map loop.bik` have no audio track; `OblivionIntro.bik` does, and is reached from the new-game path 0066F2D1).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oblivion_core::logging::{Level, emit};
use oblivion_sim::tes::bink_library::{BinkFrameInfo, BinkHandle, BinkLibrary};
use oblivion_sim::tes::os_globals::lock_shared;

/// IDA: N/A how many decoded frames may wait: enough that a frame's copy doesn't wait for the decoder.
const DECODED_FRAME_QUEUE: usize = 3;
/// IDA: N/A how long a `BinkDoFrame` waits on a decoder before it keeps the last frame. Recorded limit: it waits holding the Bink lock, so a stalled decoder holds a main thread that presents a movie for up to this long.
const DECODER_STALL: Duration = Duration::from_secs(20);
/// IDA: N/A the largest frame's bytes this accepts: what a 32-bit process could address. `_BinkOpen@8` has no bound of its own: its allocations fail and it returns null, at a size that depends on the machine.
const LARGEST_FRAME_BYTES: u64 = 0x8000_0000;

/// IDA: N/A the fields of a `.bik` header the movie player needs: BIK<version>, then little-endian dwords.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BinkFileHeader {
    /// +0x08.
    pub frames: u32,
    /// +0x14.
    pub width: u32,
    /// +0x18.
    pub height: u32,
    /// +0x1C.
    pub rate: u32,
    /// +0x20.
    pub rate_divisor: u32,
    /// +0x28.
    pub audio_tracks: u32,
}

/// IDA: N/A the C++ `read_bink_header`: `None` for a file that isn't a Bink file or is too short.
#[must_use]
pub fn read_bink_header(path: &Path) -> Option<BinkFileHeader> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut bytes = [0u8; 0x2C];
    file.read_exact(&mut bytes).ok()?;
    // _BinkOpen@8, 1800F4B0 in the shipped binkw32.dll: 0x664B4942..0x694B4942, BIKf to BIKi, else "Not a Bink file."
    if !matches!(&bytes[..4], b"BIKf" | b"BIKg" | b"BIKh" | b"BIKi") {
        return None;
    }
    let word = |offset: usize| {
        u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    };
    Some(BinkFileHeader {
        frames: word(0x08),
        width: word(0x14),
        height: word(0x18),
        rate: word(0x1C),
        rate_divisor: word(0x20),
        audio_tracks: word(0x28),
    })
}

/// IDA: N/A one frame out of the decoder: which pass over the file it's from, 1-based in the pass.
struct DecodedFrame {
    pass: u32,
    frame: u32,
    bgra: Vec<u8>,
}

struct Decoder {
    /// The frames, in order, over and over: a Bink movie's next frame after the last is its first.
    frames: Receiver<DecodedFrame>,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    reader: Option<JoinHandle<()>>,
}

// IDA: N/A one decoder process.
fn start_process(program: &Path, movie: &Path) -> std::io::Result<Child> {
    Command::new(program)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(movie)
        .args([
            "-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "bgra", "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

// IDA: N/A hands a frame over unless the movie has closed.
fn send_frame(
    sender: &SyncSender<DecodedFrame>,
    stop: &AtomicBool,
    mut frame: DecodedFrame,
) -> bool {
    loop {
        if stop.load(Ordering::Acquire) {
            return false;
        }
        match sender.try_send(frame) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(returned)) => {
                frame = returned;
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

// IDA: N/A the reader thread: pass after pass until the movie closes. A pass with no frame ends it, so a broken file can't start a process forever.
fn read_passes(
    program: &Path,
    movie: &Path,
    frame_bytes: usize,
    first: Child,
    shared: &Arc<Mutex<Option<Child>>>,
    stop: &AtomicBool,
    sender: &SyncSender<DecodedFrame>,
) {
    let mut next = Some(first);
    let mut pass = 0;
    loop {
        let Some(mut child) = next.take().or_else(|| start_process(program, movie).ok()) else {
            return;
        };
        let Some(mut stdout) = child.stdout.take() else {
            return;
        };
        *lock_shared(shared) = Some(child);
        let mut frame = 0;
        loop {
            if stop.load(Ordering::Acquire) {
                return;
            }
            let mut bgra = vec![0; frame_bytes];
            if stdout.read_exact(&mut bgra).is_err() {
                break;
            }
            frame += 1;
            if !send_frame(sender, stop, DecodedFrame { pass, frame, bgra }) {
                return;
            }
        }
        if let Some(mut child) = lock_shared(shared).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if frame == 0 {
            return;
        }
        pass += 1;
    }
}

impl Decoder {
    // IDA: N/A the first process, at `BinkOpen`, so that a missing decoder fails the open; then the reader.
    fn start(program: &Path, movie: &Path, frame_bytes: usize) -> std::io::Result<Self> {
        let first = start_process(program, movie)?;
        let (sender, frames) = sync_channel(DECODED_FRAME_QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(None));
        let reader = {
            let (program, movie) = (program.to_path_buf(), movie.to_path_buf());
            let (stop, child) = (Arc::clone(&stop), Arc::clone(&child));
            std::thread::Builder::new()
                .name("BinkDecoder".to_owned())
                .spawn(move || {
                    read_passes(&program, &movie, frame_bytes, first, &child, &stop, &sender);
                })?
        };
        Ok(Self {
            frames,
            stop,
            child,
            reader: Some(reader),
        })
    }
}

impl Drop for Decoder {
    /// IDA: N/A Rust's `Drop`.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // A read blocked on the pipe ends when the process does.
        if let Some(mut child) = lock_shared(&self.child).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(mut child) = lock_shared(&self.child).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct Movie {
    handle: BinkHandle,
    header: BinkFileHeader,
    decoder: Decoder,
    /// The frame the copies read: the last one `BinkDoFrame` took.
    frame: Vec<u8>,
    /// HBINK +0xC, 1-based.
    frame_number: u32,
    /// Which pass over the file `frame_number` belongs to.
    pass: u32,
    /// What `frame` is, as (pass, frame).
    held: (u32, u32),
    /// A frame already taken past what was asked for.
    early: Option<DecodedFrame>,
    decoder_ended: bool,
    start: Instant,
    paused_at: Option<Instant>,
}

impl Movie {
    // IDA: N/A when `frame_number` is due, from the file's rate.
    fn due(&self) -> Instant {
        let nanos = u128::from(self.frame_number - 1)
            * 1_000_000_000
            * u128::from(self.header.rate_divisor)
            / u128::from(self.header.rate);
        self.start + Duration::from_nanos(nanos as u64)
    }
}

/// IDA: N/A `BinkLibrary` over FFmpeg. Completion: 90% of the Bink contract the MoviePlayer uses; see the module's differences.
/// TODO: [ ] The movie's own sound, for `OblivionIntro.bik` and `OblivionOutro.bik`, when their paths are ported.
pub struct FfmpegBink {
    program: PathBuf,
    movies: Vec<Movie>,
    next_handle: u32,
    /// `BinkSetSoundSystem` has run.
    pub sound_system_set: bool,
}

impl Default for FfmpegBink {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self::with_program("ffmpeg")
    }
}

impl FfmpegBink {
    /// IDA: N/A names the program to run, so a test can name one that doesn't exist without touching `PATH`.
    #[must_use]
    pub fn with_program(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            movies: Vec::new(),
            next_handle: 1,
            sound_system_set: false,
        }
    }

    // IDA: N/A a movie by its host value.
    fn movie(&self, bink: BinkHandle) -> Option<&Movie> {
        self.movies.iter().find(|movie| movie.handle == bink)
    }

    // IDA: N/A the same, mutably.
    fn movie_mut(&mut self, bink: BinkHandle) -> Option<&mut Movie> {
        self.movies.iter_mut().find(|movie| movie.handle == bink)
    }
}

impl BinkLibrary for FfmpegBink {
    /// IDA: `_BinkSetSoundSystem@8`, called from 0041032D. P06-RUN-001 and P06-RUN-002 record 00410310 leaving 1 in eax; the C++ x64 build's 0 was its own choice. Nothing reads the result (the exe ignores it). A host with no audio device still answers 1, since the game's sound object exists regardless.
    fn set_direct_sound_system(&mut self) -> i32 {
        self.sound_system_set = true;
        1
    }

    /// IDA: `_BinkOpen@8`, read in the shipped DLL at 1800F4B0: a null HBINK for a file that won't open or read 44 bytes, a magic other than BIKf..BIKi, frames 0 ("The file doesn't contain any compressed frames yet."), or tables that won't read: 3 dwords per audio track and frames + 1 dwords of index ("Error reading Bink header."). IDA: N/A beyond it: a zero size or rate, which the host can't play (a zero divisor faults in the DLL, 1800F4B0's `div`); a frame past `LARGEST_FRAME_BYTES`; a decoder that won't start; and a file whose first frame doesn't decode, which binkw32 only finds at BinkDoFrame: refused here, so no movie runs black for its header's count. A file that ends later keeps its last frame to the header's count, with one warning.
    fn open(&mut self, path: &Path, _flags: u32) -> Option<BinkHandle> {
        let header = read_bink_header(path)?;
        let tables =
            0x2C + 12 * u64::from(header.audio_tracks) + 4 * (u64::from(header.frames) + 1);
        let frame_bytes = (u64::from(header.width) * u64::from(header.height)).saturating_mul(4);
        if header.frames == 0
            || header.width == 0
            || header.height == 0
            || header.rate == 0
            || header.rate_divisor == 0
            || frame_bytes > LARGEST_FRAME_BYTES
            || std::fs::metadata(path).ok()?.len() < tables
        {
            return None;
        }
        let bytes = frame_bytes as usize;
        let decoder = match Decoder::start(&self.program, path, bytes) {
            Ok(decoder) => decoder,
            Err(error) => {
                emit(
                    Level::Warning,
                    &format!(
                        "Bink decoder {} didn't start: {error}",
                        self.program.display()
                    ),
                );
                return None;
            }
        };
        let first = match decoder.frames.recv_timeout(DECODER_STALL) {
            Ok(first) => first,
            Err(_) => {
                emit(
                    Level::Warning,
                    &format!("Bink decoder gave no frame of {}", path.display()),
                );
                return None;
            }
        };
        let handle = BinkHandle(self.next_handle);
        self.next_handle += 1;
        self.movies.push(Movie {
            handle,
            header,
            decoder,
            frame: vec![0; bytes],
            frame_number: 1,
            pass: 0,
            held: (u32::MAX, 0),
            early: Some(first),
            decoder_ended: false,
            start: Instant::now(),
            paused_at: None,
        });
        Some(handle)
    }

    /// IDA: `_BinkClose@4`.
    fn close(&mut self, bink: BinkHandle) {
        self.movies.retain(|movie| movie.handle != bink);
    }

    /// IDA: `_BinkWait@4`: nonzero while the frame isn't due, or while paused.
    fn wait(&mut self, bink: BinkHandle) -> i32 {
        let Some(movie) = self.movie(bink) else {
            return 0;
        };
        i32::from(movie.paused_at.is_some() || Instant::now() < movie.due())
    }

    /// IDA: `_BinkDoFrame@4`: takes the decoded frame `frame_number` names. A decoder that ended early or stalls leaves the last frame showing, with one warning.
    fn do_frame(&mut self, bink: BinkHandle) -> i32 {
        let Some(movie) = self.movie_mut(bink) else {
            return 0;
        };
        let wanted = (movie.pass, movie.frame_number);
        while movie.held != wanted {
            let decoded = match movie.early.take() {
                Some(decoded) => decoded,
                None => match movie.decoder.frames.recv_timeout(DECODER_STALL) {
                    Ok(decoded) => decoded,
                    Err(error) => {
                        if !movie.decoder_ended {
                            movie.decoder_ended = true;
                            let reason = match error {
                                RecvTimeoutError::Timeout => "stalled",
                                RecvTimeoutError::Disconnected => "ended",
                            };
                            emit(
                                Level::Warning,
                                &format!(
                                    "Bink decoder {reason} before frame {} of {}",
                                    wanted.1, movie.header.frames
                                ),
                            );
                        }
                        return 0;
                    }
                },
            };
            // Past what's asked for: the file had fewer frames than its header said. Show the last and keep this.
            if decoded.pass > wanted.0 || (decoded.pass == wanted.0 && decoded.frame > wanted.1) {
                movie.early = Some(decoded);
                return 0;
            }
            movie.held = (decoded.pass, decoded.frame);
            movie.frame = decoded.bgra;
        }
        0
    }

    /// IDA: `_BinkNextFrame@4`: the frame number after the last is the first, with the clock restarted.
    fn next_frame(&mut self, bink: BinkHandle) {
        let Some(movie) = self.movie_mut(bink) else {
            return;
        };
        if movie.frame_number >= movie.header.frames {
            movie.frame_number = 1;
            movie.pass += 1;
            movie.start = Instant::now();
        } else {
            movie.frame_number += 1;
        }
    }

    /// IDA: `_BinkPause@8`: the paused state as a BOOL. The clock doesn't count the pause.
    fn pause(&mut self, bink: BinkHandle, pause: i32) -> i32 {
        let Some(movie) = self.movie_mut(bink) else {
            return 0;
        };
        match (pause != 0, movie.paused_at) {
            (true, None) => movie.paused_at = Some(Instant::now()),
            (false, Some(at)) => {
                movie.start += at.elapsed();
                movie.paused_at = None;
            }
            _ => {}
        }
        i32::from(movie.paused_at.is_some())
    }

    /// IDA: `_BinkCopyToBufferRect@44`, from 0040FD18: a rectangle of the frame into a locked tile. A partial tile's rectangle is larger than what's left of the frame (720 wide: the 256-wide column has 208), and that part stays as it was. Completion: 100% of the rectangle; always 0, no frame is ever skipped.
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
        _flags: u32,
    ) -> i32 {
        let Some(movie) = self.movie(bink) else {
            return 0;
        };
        let (width, height) = (movie.header.width, movie.header.height);
        if source_x >= width || source_y >= height || pitch <= 0 {
            return 0;
        }
        let columns = source_width.min(width - source_x) as usize;
        let pitch = pitch as usize;
        for row in 0..source_height {
            if source_y + row >= height || destination_y + row >= destination_height {
                break;
            }
            let to = (destination_y + row) as usize * pitch + destination_x as usize * 4;
            let from = ((source_y + row) as usize * width as usize + source_x as usize) * 4;
            let bytes = columns * 4;
            let Some(target) = destination.get_mut(to..to + bytes) else {
                break;
            };
            target.copy_from_slice(&movie.frame[from..from + bytes]);
        }
        0
    }

    /// IDA: HBINK +0, +4, +8, +0xC.
    fn info(&self, bink: BinkHandle) -> BinkFrameInfo {
        self.movie(bink)
            .map_or(BinkFrameInfo::default(), |movie| BinkFrameInfo {
                width_00: movie.header.width,
                height_04: movie.header.height,
                frames_08: movie.header.frames,
                frame_num_0c: movie.frame_number,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(name: &str) -> Option<PathBuf> {
        let folder = std::env::var_os("OBLIVION_DATA_DIR")?;
        let path = Path::new(&folder).join("Video").join(name);
        path.exists().then_some(path)
    }

    /// The headers against P06-RUN-001's `HBINK` rows, and what `ffprobe` reports: 720x480, 1 frame; 1280x720, 750 frames at 2997/100, no audio; the intro with one track.
    #[test]
    fn headers_of_the_real_movies() {
        let Some(stub) = video("2k games.bik") else {
            return;
        };
        let stub = read_bink_header(&stub).unwrap();
        assert_eq!(
            (stub.width, stub.height, stub.frames, stub.audio_tracks),
            (720, 480, 1, 0)
        );
        let map = read_bink_header(&video("Map loop.bik").unwrap()).unwrap();
        assert_eq!(
            (
                map.width,
                map.height,
                map.frames,
                map.rate,
                map.rate_divisor,
                map.audio_tracks
            ),
            (1280, 720, 750, 2997, 100, 0)
        );
        let intro = read_bink_header(&video("OblivionIntro.bik").unwrap()).unwrap();
        assert_eq!(
            (intro.width, intro.height, intro.audio_tracks),
            (1280, 720, 1)
        );
    }

    #[test]
    fn not_a_bink_file_is_a_null_open() {
        let folder = std::env::temp_dir().join(format!("recomp_bink_{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let bad = folder.join("bad.bik");
        std::fs::write(
            &bad,
            b"not a movie, and long enough to read a whole header from it",
        )
        .unwrap();
        let mut bink = FfmpegBink::default();
        assert!(bink.open(&bad, 0).is_none());
        assert!(bink.open(&folder.join("missing.bik"), 0).is_none());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    // A file with this header: BIK + version, then the fields, then `tail` bytes.
    fn forged(name: &str, magic: &[u8; 4], frames: u32, size: (u32, u32), tail: usize) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("recomp_bink_forged_{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let mut bytes = magic.to_vec();
        for word in [0, frames, 16, frames, size.0, size.1, 30, 1, 0, 0] {
            bytes.extend_from_slice(&u32::to_le_bytes(word));
        }
        bytes.resize(bytes.len() + tail, 0x5A);
        let path = folder.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// binkw32's `_BinkOpen@8` (1800F4B0 in the shipped DLL, read): magic BIKf, g, h or i, else "Not a Bink file."; frames 0, "The file doesn't contain any compressed frames yet."; the index of frames + 1 dwords must read, else "Error reading Bink header."
    #[test]
    fn what_binkopen_refuses_is_a_null_open() {
        let mut bink = FfmpegBink::default();
        let good_tail = 8 + 64;
        assert!(
            bink.open(&forged("magic.bik", b"BIKz", 1, (16, 16), good_tail), 0)
                .is_none()
        );
        assert!(
            bink.open(&forged("frames.bik", b"BIKi", 0, (16, 16), good_tail), 0)
                .is_none()
        );
        assert!(
            bink.open(&forged("short.bik", b"BIKi", 100, (16, 16), 8), 0)
                .is_none()
        );
        // IDA: N/A a frame no 32-bit process could hold: BinkOpen's allocations fail, at no bound of its own.
        assert!(
            bink.open(
                &forged(
                    "huge.bik",
                    b"BIKi",
                    1,
                    (0xFFFF_FFFF, 0xFFFF_FFFF),
                    good_tail
                ),
                0
            )
            .is_none()
        );
        assert!(bink.movies.is_empty());
    }

    /// A header BinkOpen would take over data that doesn't decode: refused at the open, so no movie runs black for its header's count.
    #[test]
    fn a_file_with_no_decodable_frame_is_a_null_open() {
        let mut bink = FfmpegBink::default();
        let bad = forged("undecodable.bik", b"BIKi", 1000, (16, 16), 8 + 4096);
        assert!(bink.open(&bad, 0).is_none());
        assert!(bink.movies.is_empty());
    }

    /// The C++ `verify_local_stub`'s 06-E-2: without the decoder, the open fails like BinkOpen, and nothing else happens.
    #[test]
    fn a_missing_decoder_is_a_null_open() {
        let Some(stub) = video("2k games.bik") else {
            return;
        };
        let mut bink = FfmpegBink::with_program("/nonexistent/ffmpeg");
        assert!(bink.open(&stub, 0).is_none());
    }

    #[test]
    fn sound_system_answers_the_recorded_value() {
        assert_eq!(FfmpegBink::default().set_direct_sound_system(), 1);
    }

    // First frame of a movie, in BGRA.
    fn first_frame(path: &Path) -> (BinkFrameInfo, Vec<u8>) {
        let mut bink = FfmpegBink::default();
        let handle = bink.open(path, 0x2000).unwrap();
        assert_eq!(bink.wait(handle), 0, "the first frame is due at once");
        bink.do_frame(handle);
        let info = bink.info(handle);
        let mut pixels = vec![0; (info.width_00 * info.height_04 * 4) as usize];
        assert_eq!(
            bink.copy_to_buffer_rect(
                handle,
                &mut pixels,
                info.width_00 as i32 * 4,
                info.height_04,
                0,
                0,
                0,
                0,
                info.width_00,
                info.height_04,
                3,
            ),
            0
        );
        bink.close(handle);
        (info, pixels)
    }

    #[test]
    fn map_loop_decodes_a_real_frame() {
        let Some(map) = video("Map loop.bik") else {
            return;
        };
        let (info, pixels) = first_frame(&map);
        assert_eq!(
            (
                info.width_00,
                info.height_04,
                info.frames_08,
                info.frame_num_0c
            ),
            (1280, 720, 750, 1)
        );
        assert!(pixels.chunks(4).any(|pixel| pixel[..3] != [0, 0, 0]));
        assert!(pixels.chunks(4).all(|pixel| pixel[3] == 255));
    }

    /// A rectangle lands at its own place in the tile, and a partial column stops at the frame's edge.
    #[test]
    fn rectangles_match_the_whole_frame() {
        let Some(map) = video("Map loop.bik") else {
            return;
        };
        let (_, whole) = first_frame(&map);
        let mut bink = FfmpegBink::default();
        let handle = bink.open(&map, 0).unwrap();
        bink.do_frame(handle);
        let mut tile = vec![0xAB; 256 * 256 * 4];
        bink.copy_to_buffer_rect(
            handle,
            &mut tile,
            256 * 4,
            720,
            0,
            0,
            1024,
            512,
            256,
            256,
            3,
        );
        // Source row 512, column 1024.
        let from = (512 * 1280 + 1024) * 4;
        assert_eq!(&tile[..256 * 4], &whole[from..from + 256 * 4]);
        // The rectangle's 208 rows are all the frame has; the rest of the tile is untouched.
        assert_eq!(
            &tile[207 * 256 * 4..207 * 256 * 4 + 4],
            &whole[((512 + 207) * 1280 + 1024) * 4..((512 + 207) * 1280 + 1024) * 4 + 4]
        );
        assert_eq!(&tile[208 * 256 * 4..208 * 256 * 4 + 4], &[0xAB; 4]);
        bink.close(handle);
    }

    /// The next frame after the last is the first, with the same bytes.
    #[test]
    fn a_movie_wraps_to_its_first_frame() {
        let Some(stub) = video("2k games.bik") else {
            return;
        };
        let (_, first) = first_frame(&stub);
        let mut bink = FfmpegBink::default();
        let handle = bink.open(&stub, 0).unwrap();
        bink.do_frame(handle);
        bink.next_frame(handle);
        assert_eq!(bink.info(handle).frame_num_0c, 1);
        bink.do_frame(handle);
        let mut again = vec![0; 720 * 480 * 4];
        bink.copy_to_buffer_rect(handle, &mut again, 720 * 4, 480, 0, 0, 0, 0, 720, 480, 3);
        assert_eq!(first, again);
        bink.close(handle);
    }

    #[test]
    fn pause_holds_the_clock() {
        let Some(map) = video("Map loop.bik") else {
            return;
        };
        let mut bink = FfmpegBink::default();
        let handle = bink.open(&map, 0).unwrap();
        assert_eq!(bink.pause(handle, 1), 1);
        assert_eq!(bink.wait(handle), 1, "a paused movie is never due");
        assert_eq!(bink.pause(handle, 0), 0);
        assert_eq!(bink.wait(handle), 0);
        bink.next_frame(handle);
        assert_eq!(bink.wait(handle), 1, "frame 2 is 33 ms later");
        bink.close(handle);
    }
}
