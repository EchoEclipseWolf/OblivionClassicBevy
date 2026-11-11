//! The music half of OSSoundGlobals: 006AA170, 006AA1A0, 006A8D00, 006A8D50, 006A8DB0, 006AB160, 006AB420, 006ACD10 and the part of 006AD030 a menu reaches. The original drives a DirectShow filter graph; here each COM call is one `MusicDevice` call with its original site, so the order stays visible and `oblivion_host` answers them over `bevy_audio`.
//!
//! x87: all of this runs on the main thread after device creation, 24-bit (control word 007F, P04-RUN-002), so `f32` arithmetic in the instructions' grouping is right. `__CIlog10` goes through `f64` and rounds once at its store (D6).
//!
//! Recorded host substitutions: the DirectShow graph is the seam, and the `+0x70`/`+0x74` interfaces are a host value and a flag; the byte at 00B3C0EC lives here because only sound reads it past MainMenu's own stores; the wide-character conversion at 006AB38C is the seam's: it takes the original's bytes.

use std::sync::{Arc, Mutex};

use bevy_ecs::prelude::{Resource, World};
use oblivion_core::x87::ftol;

use super::os_globals::{CreatedInterface, OsSoundGlobals};

/// IDA: a host IFilterGraph. Never compared with evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MusicGraph(pub u32);

/// IDA: 006AB160's second-argument values that select an arm.
pub const MUSIC_TYPE_BATTLE: u16 = 4;
pub const MUSIC_TYPE_SPECIAL: u16 = 8;
pub const MUSIC_TYPE_ANY: u16 = 0xFFFF;

/// IDA: `+0xDC`'s bits, as 006AB3FC, 006AB4C9, 006A8D00 and 006ACD10 use them.
pub const MUSIC_FLAG_LOADED: u32 = 1;
pub const MUSIC_FLAG_PLAYING: u32 = 2;
pub const MUSIC_FLAG_PAUSED: u32 = 4;
pub const MUSIC_FLAG_FADE_OUT: u32 = 8;
pub const MUSIC_FLAG_FADE_IN: u32 = 0x10;

/// IDA: 006AA24A `-10000`: IBasicAudio's silence.
pub const MUSIC_VOLUME_SILENT: i32 = -10000;

/// IDA: EC_COMPLETE, EC_USERABORT and EC_ERRORABORT, the events 006AD96C's loop stops on (`> 0 && <= 3`).
const MUSIC_EVENT_LAST_STOP: i32 = 3;

/// IDA: A771C0 1700.0, the fade's length at 006AD2C7.
const MUSIC_FADE_MS: f32 = 1700.0;

/// IDA: N/A one COM call of the graph, with the original's arguments. `f64` keeps its bits because the method takes a double.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MusicGraphCall {
    /// QueryInterface(IID_IMediaControl A78FA4).
    QueryMediaControl,
    /// QueryInterface(IID_IMediaPosition A78F84).
    QueryMediaPosition,
    /// IMediaPosition::put_CurrentPosition, vtable +0x20, 006AB486.
    PutCurrentPosition { position_bits: u64 },
    /// IUnknown::Release of IMediaPosition, vtable +8.
    ReleaseMediaPosition,
    /// IMediaControl::Run, vtable +0x1C, 006AB49F and 006A8D50.
    Run,
    /// IMediaControl::Pause, vtable +0x20, 006AB4B6 and 006A8D00.
    Pause,
    /// IMediaControl::Stop, vtable +0x24, 006A8DE3.
    Stop,
    /// IUnknown::Release of IMediaControl, vtable +8.
    ReleaseMediaControl,
    /// QueryInterface(IID_IBasicAudio A78F74) into +0x74.
    QueryBasicAudio,
    /// IBasicAudio::put_Volume, vtable +0x1C.
    PutVolume { volume: i32 },
    /// IUnknown::Release of +0x74.
    ReleaseBasicAudio,
    /// IUnknown::Release of +0x70.
    ReleaseGraph,
}

/// IDA: N/A the seam in place of the DirectShow calls. `site` is the original call site. Nothing here touches an audio device; `oblivion_host` answers it, and a host with no device answers as a graph that plays nothing. Completion: 100% of the calls 006AA170..006ACD10 make.
pub trait MusicDevice: Send {
    /// IDA: see the words here, which name the addresses this relates to.
    /// `_access(path, 0)` (006AB2FB), on the original's relative bytes.
    fn access(&mut self, site: u32, path: &[u8]) -> bool;
    /// IDA: 006A8E80's FindFirstFileA and FindNextFileA over `folder`*.mp3: the names, in the device's order.
    fn music_files(&mut self, _site: u32, _folder: &[u8]) -> Vec<Vec<u8>> {
        Vec::new()
    }
    /// IDA: see the words here, which name the addresses this relates to.
    /// CoCreateInstance(CLSID_FilterGraph, IGraphBuilder) (006AB366).
    fn create_graph(&mut self, site: u32) -> CreatedInterface<MusicGraph>;
    /// IDA: see the words here, which name the addresses this relates to.
    /// IGraphBuilder::RenderFile, vtable +0x34 (006AB3A4): the HRESULT.
    fn render_file(&mut self, site: u32, graph: MusicGraph, path: &[u8]) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// A call whose only result is its HRESULT.
    fn call(&mut self, site: u32, graph: MusicGraph, call: MusicGraphCall) -> u32;
    /// IDA: see the words here, which name the addresses this relates to.
    /// IMediaEventEx::GetEvent, vtable +0x20 (006AD999): the next event code, `None` where the original gets a failure.
    fn next_event(&mut self, site: u32, graph: MusicGraph) -> Option<i32>;
}

/// IDA: N/A one recorded call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MusicTraceCall {
    Access(Vec<u8>),
    CreateGraph,
    RenderFile(Vec<u8>),
    Graph(MusicGraphCall),
    NextEvent,
}

/// IDA: N/A the seam's script and its trace, for tests and the driver's default: every call succeeds, and no track ever ends.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptedMusicDevice {
    pub trace: Vec<(u32, MusicTraceCall)>,
    pub missing_files: Vec<Vec<u8>>,
    /// 006A8E80's listing of any folder.
    pub listing: Vec<Vec<u8>>,
    pub fail_create: bool,
    pub fail_render: bool,
    pub events: Vec<i32>,
    next_graph: u32,
}

impl MusicDevice for ScriptedMusicDevice {
    /// IDA: N/A `ScriptedMusicDevice`'s side of `MusicDevice`.
    fn music_files(&mut self, _site: u32, _folder: &[u8]) -> Vec<Vec<u8>> {
        self.listing.clone()
    }
    /// IDA: N/A `ScriptedMusicDevice`'s side of `MusicDevice`.
    fn access(&mut self, site: u32, path: &[u8]) -> bool {
        self.trace
            .push((site, MusicTraceCall::Access(path.to_vec())));
        !self.missing_files.iter().any(|file| file == path)
    }
    /// IDA: N/A `ScriptedMusicDevice`'s side of `MusicDevice`.
    fn create_graph(&mut self, site: u32) -> CreatedInterface<MusicGraph> {
        self.trace.push((site, MusicTraceCall::CreateGraph));
        if self.fail_create {
            return CreatedInterface {
                result: super::os_globals::E_FAIL,
                object: None,
            };
        }
        self.next_graph += 1;
        CreatedInterface {
            result: super::os_globals::S_OK,
            object: Some(MusicGraph(self.next_graph)),
        }
    }
    /// IDA: N/A `ScriptedMusicDevice`'s side of `MusicDevice`.
    fn render_file(&mut self, site: u32, _graph: MusicGraph, path: &[u8]) -> u32 {
        self.trace
            .push((site, MusicTraceCall::RenderFile(path.to_vec())));
        if self.fail_render {
            super::os_globals::E_FAIL
        } else {
            super::os_globals::S_OK
        }
    }
    /// IDA: N/A `ScriptedMusicDevice`'s side of `MusicDevice`.
    fn call(&mut self, site: u32, _graph: MusicGraph, call: MusicGraphCall) -> u32 {
        self.trace.push((site, MusicTraceCall::Graph(call)));
        super::os_globals::S_OK
    }
    /// IDA: N/A `ScriptedMusicDevice`'s side of `MusicDevice`.
    fn next_event(&mut self, site: u32, _graph: MusicGraph) -> Option<i32> {
        self.trace.push((site, MusicTraceCall::NextEvent));
        if self.events.is_empty() {
            None
        } else {
            Some(self.events.remove(0))
        }
    }
}

/// IDA: N/A the driver publishes this at the first scripts frame. Both cell and intro music
/// use the sampled device; cell music also uses the frame's INI snapshot and the main CRT.
#[derive(Clone, Resource)]
pub(crate) struct ScriptMusic {
    device: Arc<Mutex<dyn MusicDevice>>,
    settings: MusicSettings,
    draw: Arc<dyn Fn() -> i32 + Send + Sync>,
}

impl ScriptMusic {
    /// IDA: N/A bind the driver's device and random source once.
    pub(crate) fn new(
        device: Arc<Mutex<dyn MusicDevice>>,
        settings: MusicSettings,
        draw: impl Fn() -> i32 + Send + Sync + 'static,
    ) -> Self {
        Self {
            device,
            settings,
            draw: Arc::new(draw),
        }
    }

    /// IDA: N/A the device and INI were sampled at idle entry for this scripts frame.
    pub(crate) fn refresh(
        &mut self,
        device: &Arc<Mutex<dyn MusicDevice>>,
        settings: MusicSettings,
    ) {
        self.device.clone_from(device);
        self.settings = settings;
    }

    /// IDA: 0066EAF0's interior music calls, with 00440AF0's resume/start gates.
    pub(crate) fn cell_music(
        &self,
        world: &mut World,
        music_type: u16,
        resume: bool,
        start: bool,
    ) -> bool {
        let mut device = super::os_globals::lock_shared(&self.device);
        let mut os = world.resource_mut::<super::os_globals::OsGlobalsData>();
        let Some(sound) = os.sound_24.as_mut() else {
            return false;
        };
        let loaded = sound.load_random_music_track(
            music_type,
            true,
            &self.settings,
            &mut *device,
            &mut || (self.draw)(),
        );
        if loaded && resume {
            sound.resume_music_playback(&mut *device);
        }
        if loaded && start {
            sound.start_music_playback(&self.settings, &mut *device);
        }
        loaded
    }

    /// IDA: 0066F2B2 and 0066F2DB pause or resume the intro's music.
    pub(crate) fn intro_music(&self, world: &mut World, pause: bool) {
        let mut device = super::os_globals::lock_shared(&self.device);
        let mut os = world.resource_mut::<super::os_globals::OsGlobalsData>();
        if let Some(sound) = os.sound_24.as_mut() {
            if pause {
                sound.pause_music_playback(&mut *device);
            } else {
                sound.resume_music_playback(&mut *device);
            }
        }
    }
}

/// IDA: the settings these functions read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MusicSettings {
    /// 00B16180 bMusicEnabled:Audio, its low byte.
    pub music_enabled_b16180: bool,
    /// 00B16188 fMainMenuMusicVolume:Audio.
    pub main_menu_music_volume_b16188: f32,
}

// IDA: `_strstr(+0x1E4, needle)`, on bytes.
fn path_contains(path: &[u8], needle: &[u8]) -> bool {
    path.windows(needle.len()).any(|window| window == needle)
}

// IDA: `test eax, eax; jl`.
const fn failed(result: u32) -> bool {
    (result as i32) < 0
}

impl OsSoundGlobals {
    /// IDA: 006AA1A0 OSSoundGlobals_SetMusicVolume(volume, updateStoredVolume). 006AA1A0..006AA1D4 clamps to 0..1, keeping a NaN (each `fcom` sends it the keep way); 006AA1DD..006AA1E3 +0x2F8 and +0x2F0; 006AA1E9 a set 00B3C0EC replaces it by fMainMenuMusicVolume; 006AA209..006AA219 (v * master), rounded, squared, rounded; 006AA21D no track returns; 006AA229 `< 0.01` -> -10000, else log10 rounded to float, * 2000.0, truncated. Completion: 100%, with the device absent only as `+0x74` is unset, where the original faults.
    pub fn set_music_volume(
        &mut self,
        volume: f32,
        update_stored: bool,
        settings: &MusicSettings,
        device: &mut dyn MusicDevice,
    ) {
        // `clamp` keeps a NaN too.
        let mut value = volume.clamp(0.0, 1.0);
        if update_stored {
            self.music_volume2_2f8 = value;
            self.music_volume_2f0 = value;
        }
        if self.main_menu_music_b3c0ec != 0 {
            value = settings.main_menu_music_volume_b16188;
        }
        let scaled = value * self.master_volume_b8;
        let squared = scaled * scaled;
        if self.music_flags_dc & MUSIC_FLAG_LOADED == 0 {
            return;
        }
        let Some(graph) = self.music_graph_70 else {
            return;
        };
        let level = if squared < 0.01f32 {
            MUSIC_VOLUME_SILENT
        } else {
            // 006AA253 __CIlog10, 006AA258 fstp m32, 006AA260 fmul 2000.0 at 24 bits, 006AA266 __ftol2_sse.
            let log = f64::from(squared).log10() as f32;
            ftol(log * 2000.0f32)
        };
        device.call(
            0x006A_A26E,
            graph,
            MusicGraphCall::PutVolume { volume: level },
        );
    }

    /// IDA: 006A8DB0 OSSoundGlobals_StopMusicPlayback. Only with +0xDC & 2 and a graph: QueryInterface(IMediaControl), Stop, Release, then `+0xDC &= ~3`, so a stop also clears the loaded bit. Completion: 100%.
    pub fn stop_music_playback(&mut self, device: &mut dyn MusicDevice) {
        if self.music_flags_dc & MUSIC_FLAG_PLAYING == 0 {
            return;
        }
        let Some(graph) = self.music_graph_70 else {
            return;
        };
        if failed(device.call(0x006A_8DD3, graph, MusicGraphCall::QueryMediaControl)) {
            return;
        }
        device.call(0x006A_8DE3, graph, MusicGraphCall::Stop);
        device.call(0x006A_8DEF, graph, MusicGraphCall::ReleaseMediaControl);
        self.music_flags_dc &= !(MUSIC_FLAG_LOADED | MUSIC_FLAG_PLAYING);
    }

    /// IDA: 006AA170 OSSoundGlobals_ReleaseMusicTrack. With +0xDC & 1: 006AA17C stop (which clears bit 0 only when it was playing), Release +0x74, Release +0x70, `+0xDC &= ~1`. The pointers stay, as the original leaves them. Completion: 100%.
    pub fn release_music_track(&mut self, device: &mut dyn MusicDevice) {
        if self.music_flags_dc & MUSIC_FLAG_LOADED == 0 {
            return;
        }
        self.stop_music_playback(device);
        if let Some(graph) = self.music_graph_70 {
            device.call(0x006A_A18A, graph, MusicGraphCall::ReleaseBasicAudio);
            device.call(0x006A_A195, graph, MusicGraphCall::ReleaseGraph);
        }
        self.music_flags_dc &= !MUSIC_FLAG_LOADED;
    }

    /// IDA: 006A8D00. With a graph, QueryInterface(IMediaControl), Pause, Release, `+0xDC |= 4`. Completion: 100%.
    pub fn pause_music_playback(&mut self, device: &mut dyn MusicDevice) {
        let Some(graph) = self.music_graph_70 else {
            return;
        };
        if failed(device.call(0x006A_8D1A, graph, MusicGraphCall::QueryMediaControl)) {
            return;
        }
        device.call(0x006A_8D2E, graph, MusicGraphCall::Pause);
        device.call(0x006A_8D3A, graph, MusicGraphCall::ReleaseMediaControl);
        self.music_flags_dc |= MUSIC_FLAG_PAUSED;
    }

    /// IDA: 006A8D50. Playing and with a graph: QueryInterface(IMediaControl), Run, Release, `+0xDC &= ~4`; not playing: `+0xDC &= ~4`. A failed query leaves the bit. Completion: 100%.
    pub fn resume_music_playback(&mut self, device: &mut dyn MusicDevice) {
        if self.music_flags_dc & MUSIC_FLAG_PLAYING == 0 {
            self.music_flags_dc &= !MUSIC_FLAG_PAUSED;
            return;
        }
        let Some(graph) = self.music_graph_70 else {
            return;
        };
        if failed(device.call(0x006A_8D80, graph, MusicGraphCall::QueryMediaControl)) {
            return;
        }
        device.call(0x006A_8D90, graph, MusicGraphCall::Run);
        device.call(0x006A_8D9C, graph, MusicGraphCall::ReleaseMediaControl);
        self.music_flags_dc &= !MUSIC_FLAG_PAUSED;
    }

    /// IDA: 006AB160 OSSoundGlobals_LoadMusicTrack(musicType, pathOverride, allowDeathTrack). 006AB174 bMusicEnabled; 006AB1A2..006AB1CA a "death" track holds against type -1 unless allowed; 006AB1CC..006AB1F3 a "success" track under type 8 only takes the type; 006AB20F..006AB235 under type 8 and not playing, a null path means the current one; 006AB23E..006AB249 type 4 asked for 8 clears the type; 006AB252..006AB28D the holds; 006AB293.. the path; 006AB2FB `_access`; 006AB30C..006AB347 the same path holds unless the type is 8; 006AB34F release; 006AB366 CoCreateInstance; 006AB3A4 RenderFile; 006AB3BE QueryInterface, unchecked; 006AB3C0 the volume unless fading; 006AB3DC.. the path, +0xDC |= 1, +0xB0. Returns whether a new track is loaded. Completion: 95%.
    /// TODO: [ ] 006AB2AE..006AB2EE: no path at all reaches TES_GetCurrentCell, TESObjectCELL_ResolveMusicType and Music_SelectRandomTrackPath. Counted in `music_unported_arms`, and it returns false as a failed selection does.
    pub fn load_music_track(
        &mut self,
        music_type: u16,
        path_override: Option<&[u8]>,
        allow_death_track: bool,
        settings: &MusicSettings,
        device: &mut dyn MusicDevice,
    ) -> bool {
        if !settings.music_enabled_b16180 {
            return false;
        }
        if path_contains(&self.music_path_1e4, b"death")
            && music_type == MUSIC_TYPE_ANY
            && !allow_death_track
        {
            return false;
        }
        if path_contains(&self.music_path_1e4, b"success")
            && self.current_music_type_b0 == MUSIC_TYPE_SPECIAL
        {
            self.current_music_type_b0 = music_type;
            return false;
        }
        let mut current = self.current_music_type_b0;
        let cached = current == MUSIC_TYPE_SPECIAL
            && self.music_flags_dc & MUSIC_FLAG_PLAYING == 0
            && path_override.is_none();
        if current == MUSIC_TYPE_BATTLE && music_type == MUSIC_TYPE_SPECIAL {
            self.current_music_type_b0 = 0;
        }
        current = self.current_music_type_b0;
        let held_by_type = (current == MUSIC_TYPE_SPECIAL || current == MUSIC_TYPE_BATTLE)
            && music_type != MUSIC_TYPE_ANY;
        if (held_by_type || current == music_type) && self.music_flags_dc & MUSIC_FLAG_PLAYING != 0
        {
            return false;
        }
        if current != MUSIC_TYPE_SPECIAL
            && current != MUSIC_TYPE_BATTLE
            && music_type == MUSIC_TYPE_ANY
        {
            return false;
        }
        let path = match (path_override, cached) {
            (Some(path), _) => path.to_vec(),
            (None, true) => self.music_path_1e4.clone(),
            (None, false) => {
                self.music_unported_arms += 1;
                return false;
            }
        };
        self.open_music_graph(&path, music_type, settings, device)
    }

    /// IDA: 006AB160 with no path: 006AB2AE's type, with `music_type` -1 asking the current cell, then 006A8E80 Music_SelectRandomTrackPath: the folder by type, a count of its *.mp3, `Random_Next(0) % count`, that file. `has_cell` is 0043F270's answer: no cell is type 0. `0xFFFF` and a held track return as 006AB160's own tests do. FindFirstFileA's order is the device's. Completion: 100% with no cell.
    /// TODO: [ ] 006AB2C0's TESObjectCELL_ResolveMusicType, with a cell.
    pub fn load_random_music_track(
        &mut self,
        music_type: u16,
        has_cell: bool,
        settings: &MusicSettings,
        device: &mut dyn MusicDevice,
        random: &mut dyn FnMut() -> i32,
    ) -> bool {
        if !settings.music_enabled_b16180 {
            return false;
        }
        if path_contains(&self.music_path_1e4, b"death") && music_type == MUSIC_TYPE_ANY {
            return false;
        }
        if path_contains(&self.music_path_1e4, b"success")
            && self.current_music_type_b0 == MUSIC_TYPE_SPECIAL
        {
            self.current_music_type_b0 = music_type;
            return false;
        }
        let mut current = self.current_music_type_b0;
        let cached = current == MUSIC_TYPE_SPECIAL && self.music_flags_dc & MUSIC_FLAG_PLAYING == 0;
        if current == MUSIC_TYPE_BATTLE && music_type == MUSIC_TYPE_SPECIAL {
            self.current_music_type_b0 = 0;
        }
        current = self.current_music_type_b0;
        let held_by_type = (current == MUSIC_TYPE_SPECIAL || current == MUSIC_TYPE_BATTLE)
            && music_type != MUSIC_TYPE_ANY;
        if (held_by_type || current == music_type) && self.music_flags_dc & MUSIC_FLAG_PLAYING != 0
        {
            return false;
        }
        if current != MUSIC_TYPE_SPECIAL
            && current != MUSIC_TYPE_BATTLE
            && music_type == MUSIC_TYPE_ANY
        {
            return false;
        }
        if cached {
            let path = self.music_path_1e4.clone();
            return self.open_music_graph(&path, music_type, settings, device);
        }
        self.open_random_track(music_type, has_cell, settings, device, random)
    }

    /// IDA: the no-path arm 006AB160 and 006ACD10 share (006AB2AE.., 006ACE7E..): `music_type` -1 asks the current cell (none: type 0), then 006A8E80 Music_SelectRandomTrackPath: the folder by type, a count of its *.mp3, `Random_Next(0) % count`, that file; none answers 0 before any draw. Then the load's tail. Completion: 100% with no cell.
    fn open_random_track(
        &mut self,
        music_type: u16,
        has_cell: bool,
        settings: &MusicSettings,
        device: &mut dyn MusicDevice,
        random: &mut dyn FnMut() -> i32,
    ) -> bool {
        let mut selected = music_type;
        if music_type == MUSIC_TYPE_ANY {
            if has_cell {
                self.music_unported_arms += 1;
                return false;
            }
            selected = 0;
        }
        let folder: &[u8] = match selected {
            1 => b"Data\\Music\\Public\\",
            2 => b"Data\\Music\\Dungeon\\",
            4 => b"Data\\Music\\Battle\\",
            _ => b"Data\\Music\\Explore\\",
        };
        let files = device.music_files(0x006A_8EC8, folder);
        if files.is_empty() {
            return false;
        }
        let at = random().rem_euclid(files.len() as i32) as usize;
        let mut path = folder.to_vec();
        path.extend_from_slice(&files[at]);
        self.open_music_graph(&path, selected, settings, device)
    }

    /// IDA: the tail both 006AB160 and 006ACD10 inline (006AB2F4..006AB403): `_access`, the same-path hold, release, CoCreateInstance, RenderFile, QueryInterface, the volume, the path, +0xDC |= 1 and +0xB0. Returns whether the track loaded. Completion: 100%.
    fn open_music_graph(
        &mut self,
        path: &[u8],
        music_type: u16,
        settings: &MusicSettings,
        device: &mut dyn MusicDevice,
    ) -> bool {
        if !device.access(0x006A_B2FB, path) {
            return false;
        }
        if self.current_music_type_b0 != MUSIC_TYPE_SPECIAL && self.music_path_1e4 == path {
            return false;
        }
        self.release_music_track(device);
        let created = device.create_graph(0x006A_B366);
        self.music_graph_70 = created.object;
        let Some(graph) = created.object.filter(|_| !failed(created.result)) else {
            return false;
        };
        if failed(device.render_file(0x006A_B3A4, graph, path)) {
            return false;
        }
        device.call(0x006A_B3BE, graph, MusicGraphCall::QueryBasicAudio);
        self.music_audio_74 = true;
        if self.music_flags_dc & (MUSIC_FLAG_FADE_OUT | MUSIC_FLAG_FADE_IN) == 0 {
            // 006AB3D7 runs before +0xDC |= 1, so this one only stores nothing and calls nothing: `test +0xDC, 1` is false on a fresh load.
            self.set_music_volume(self.music_volume_2f0, false, settings, device);
        }
        self.music_path_1e4 = path.to_vec();
        self.music_flags_dc |= MUSIC_FLAG_LOADED;
        self.current_music_type_b0 = music_type;
        true
    }

    /// IDA: 006AB420 OSSoundGlobals_StartMusicPlayback. Needs bMusicEnabled and +0xDC & 1. QueryInterface(IMediaControl) < 0 returns; QueryInterface(IMediaPosition) >= 0: put_CurrentPosition(0.0), Release; Run; +0xDC & 4: Pause; Release; Run >= 0: +0xDC |= 2; then, still loaded and not (8 and 0x10) both, the volume. Completion: 100%.
    pub fn start_music_playback(&mut self, settings: &MusicSettings, device: &mut dyn MusicDevice) {
        if !settings.music_enabled_b16180 || self.music_flags_dc & MUSIC_FLAG_LOADED == 0 {
            return;
        }
        let Some(graph) = self.music_graph_70 else {
            return;
        };
        if failed(device.call(0x006A_B452, graph, MusicGraphCall::QueryMediaControl)) {
            return;
        }
        if !failed(device.call(0x006A_B46E, graph, MusicGraphCall::QueryMediaPosition)) {
            device.call(
                0x006A_B486,
                graph,
                MusicGraphCall::PutCurrentPosition {
                    position_bits: 0.0f64.to_bits(),
                },
            );
            device.call(0x006A_B493, graph, MusicGraphCall::ReleaseMediaPosition);
        }
        let run = device.call(0x006A_B49F, graph, MusicGraphCall::Run);
        if self.music_flags_dc & MUSIC_FLAG_PAUSED != 0 {
            device.call(0x006A_B4B6, graph, MusicGraphCall::Pause);
        }
        device.call(0x006A_B4C2, graph, MusicGraphCall::ReleaseMediaControl);
        if !failed(run) {
            self.music_flags_dc |= MUSIC_FLAG_PLAYING;
        }
        let flags = self.music_flags_dc;
        if flags & MUSIC_FLAG_LOADED != 0
            && (flags & MUSIC_FLAG_FADE_OUT == 0 || flags & MUSIC_FLAG_FADE_IN == 0)
        {
            self.set_music_volume(self.music_volume_2f0, false, settings, device);
        }
    }

    /// IDA: 006ACD10 OSSoundGlobals_RequestMusicType(musicType, a3, a4), named here; a3 and a4 are not read. 006ACD17..006ACD36 +0x2E8 = tick, +0x2EC = tick + 1700, +0x2FC = type; playing: a different type only sets +0x2F4 = +0x2F8 and +0xDC |= 8. Not playing: +0x2F4 = +0x2F8, +0x2F0 = 0.0, `+0xDC = +0xDC & ~0x18 | 0x10`, then the load's holds, the no-path arm's random track (006ACEC5, `open_random_track`) or the cached path, and 006AB420 in every case. Completion: 100% with no cell.
    pub fn request_music_type(
        &mut self,
        music_type: u16,
        tick: u32,
        has_cell: bool,
        settings: &MusicSettings,
        device: &mut dyn MusicDevice,
        random: &mut dyn FnMut() -> i32,
    ) {
        self.music_start_tick_2e8 = tick;
        self.music_fade_end_2ec = tick.wrapping_add(1700);
        self.music_next_type_2fc = music_type;
        if self.music_flags_dc & MUSIC_FLAG_PLAYING != 0 {
            if self.current_music_type_b0 != music_type {
                self.unk_2f4 = self.music_volume2_2f8;
                self.music_flags_dc |= MUSIC_FLAG_FADE_OUT;
            }
            return;
        }
        self.unk_2f4 = self.music_volume2_2f8;
        self.music_volume_2f0 = 0.0;
        self.music_flags_dc =
            self.music_flags_dc & !(MUSIC_FLAG_FADE_OUT | MUSIC_FLAG_FADE_IN) | MUSIC_FLAG_FADE_IN;
        self.request_load_tail(music_type, has_cell, settings, device, random);
        self.start_music_playback(settings, device);
    }

    /// IDA: 006ACD71..006ACFDD between the fade's stores and 006ACFE8: the holds of 006AB160 without a path argument. Completion: 90%, as `request_music_type`.
    fn request_load_tail(
        &mut self,
        music_type: u16,
        has_cell: bool,
        settings: &MusicSettings,
        device: &mut dyn MusicDevice,
        random: &mut dyn FnMut() -> i32,
    ) {
        if !settings.music_enabled_b16180
            || (path_contains(&self.music_path_1e4, b"death") && music_type == MUSIC_TYPE_ANY)
        {
            return;
        }
        if path_contains(&self.music_path_1e4, b"success")
            && self.current_music_type_b0 == MUSIC_TYPE_SPECIAL
        {
            self.current_music_type_b0 = music_type;
            return;
        }
        let first = self.current_music_type_b0;
        let cached = first == MUSIC_TYPE_SPECIAL && self.music_flags_dc & MUSIC_FLAG_PLAYING == 0;
        if first == MUSIC_TYPE_BATTLE && music_type == MUSIC_TYPE_SPECIAL {
            self.current_music_type_b0 = 0;
        }
        let current = self.current_music_type_b0;
        let special = current == MUSIC_TYPE_SPECIAL || current == MUSIC_TYPE_BATTLE;
        if ((special && music_type != MUSIC_TYPE_ANY) || current == music_type)
            && self.music_flags_dc & MUSIC_FLAG_PLAYING != 0
        {
            return;
        }
        if !special && music_type == MUSIC_TYPE_ANY {
            return;
        }
        if !cached {
            self.open_random_track(music_type, has_cell, settings, device, random);
            return;
        }
        let path = self.music_path_1e4.clone();
        self.open_music_graph(&path, music_type, settings, device);
    }

    /// IDA: 006AD030 OSSoundGlobals_UpdateMusic, named here, as far as a player with no cell reaches. 006AD045 bMusicEnabled; 006AD068 a null graph with a null cell returns; 006AD2A7..006AD305 the fade's ratio, (tick - +0x2E8) as unsigned over 1700.0, clamped to 0..1 and rounded; 006AD30E a null +0x74 queries it; `+0xDC & 8`, `& 1 and & 0x10`, and otherwise the 0.02 per frame ramp toward +0x2F8; 0x1A clear: 006ACD10(+0xB0, 0, 1.0), which is how a finished track starts again; `& 1`: 006AD96C QueryInterface(IMediaEvent), GetEvent until it fails, stopping on codes 1..3. Completion: 70% of the function, which is 100% of what a player with no cell reaches under type 8.
    /// TODO: [ ] The cell, worldspace and combat arms: `player_has_cell`, type 4, and the fade-out's own load at 006AD351..: each counts in `music_unported_arms` and goes on. Phase 17 onward reaches them.
    pub fn update_music(
        &mut self,
        tick: u32,
        player_has_cell: bool,
        settings: &MusicSettings,
        device: &mut dyn MusicDevice,
        random: &mut dyn FnMut() -> i32,
    ) {
        self.update_music_in_cell(tick, player_has_cell, None, settings, device, random);
    }

    /// IDA: 006AD030 as `update_music`, with the player's interior cell's music type (`cell_music_type`, 004CAD00's answer)
    /// for the null-graph arm: 006AD062..006AD0F1 a player with no cell returns; an interior cell (an exterior's 0044A270
    /// is counted as unported) with +0xB0 other than 4 (battle) runs 006ACD10 RequestMusicType(that type, 0, +0x2F8) and
    /// returns; the battle arm (006AD0FB..) is counted. The unloaded arm (006AD64C) takes the interior's type the same way.
    /// Completion: as `update_music`, plus the interior's null-graph and unloaded arms.
    pub fn update_music_in_cell(
        &mut self,
        tick: u32,
        player_has_cell: bool,
        cell_music_type: Option<u16>,
        settings: &MusicSettings,
        device: &mut dyn MusicDevice,
        random: &mut dyn FnMut() -> i32,
    ) {
        if !settings.music_enabled_b16180 {
            return;
        }
        let Some(graph) = self.music_graph_70 else {
            if let Some(music_type) = cell_music_type
                && self.current_music_type_b0 != MUSIC_TYPE_BATTLE
            {
                self.request_music_type(music_type, tick, true, settings, device, random);
                return;
            }
            if player_has_cell {
                self.music_unported_arms += 1;
            }
            return;
        };
        let elapsed = tick.wrapping_sub(self.music_start_tick_2e8);
        // 006AD2BB fild, +2^32 when negative; 006AD2C7 fdiv; 006AD2CD fstp m32.
        let ratio = (elapsed as f32 / MUSIC_FADE_MS).clamp(0.0, 1.0);
        if !self.music_audio_74 {
            device.call(0x006A_D31F, graph, MusicGraphCall::QueryBasicAudio);
            self.music_audio_74 = true;
        }
        let flags = self.music_flags_dc;
        let mut next_volume = None;
        if flags & MUSIC_FLAG_FADE_OUT != 0 {
            if self.music_volume_2f0 < 0.05 {
                // 006AD351: the fade-out's load of +0x2FC.
                self.music_flags_dc =
                    flags & !(MUSIC_FLAG_FADE_OUT | MUSIC_FLAG_FADE_IN) | MUSIC_FLAG_FADE_IN;
                let wanted = self.music_next_type_2fc;
                self.request_load_tail(wanted, player_has_cell, settings, device, random);
                self.start_music_playback(settings, device);
            } else if flags & MUSIC_FLAG_LOADED == 0 {
                self.music_flags_dc = flags & !MUSIC_FLAG_FADE_OUT;
            } else if ratio <= 0.5 {
                next_volume =
                    Some(self.music_volume2_2f8 - (ratio + ratio) * self.music_volume2_2f8);
            } else {
                next_volume = Some(0.0);
            }
        } else if flags & MUSIC_FLAG_LOADED != 0 && flags & MUSIC_FLAG_FADE_IN != 0 {
            if self.unk_2f4 <= self.music_volume_2f0 {
                self.music_flags_dc = flags & !MUSIC_FLAG_FADE_IN;
            } else if tick <= self.music_fade_end_2ec {
                next_volume = Some((ratio - 0.5 + (ratio - 0.5)) * self.music_volume2_2f8);
            } else {
                next_volume = Some(self.music_volume2_2f8);
            }
        } else if flags & MUSIC_FLAG_LOADED == 0 {
            // 006AD64C: the player's cell. With none, it goes to the tail below. An interior's: a path holding "death"
            // makes +0xB0 0; +0xB0 other than 4 runs 006ACD10(004CAD00's type, 0, 1.0), then the tail. An exterior's
            // (0044A270) and the battle arm are counted.
            if let Some(music_type) = cell_music_type {
                if path_contains(&self.music_path_1e4, b"death") {
                    self.current_music_type_b0 = 0;
                }
                if self.current_music_type_b0 != MUSIC_TYPE_BATTLE {
                    self.request_music_type(music_type, tick, true, settings, device, random);
                } else {
                    self.music_unported_arms += 1;
                }
            } else if player_has_cell {
                self.music_unported_arms += 1;
            }
        } else if self.music_volume2_2f8 != self.music_volume_2f0 {
            let gap = self.music_volume2_2f8 - self.music_volume_2f0;
            next_volume = Some(if gap.abs() < 0.02f32 {
                self.music_volume2_2f8
            } else if self.music_volume_2f0 >= self.music_volume2_2f8 {
                self.music_volume_2f0 - 0.02f32
            } else {
                self.music_volume_2f0 + 0.02f32
            });
        }
        if let Some(volume) = next_volume {
            self.music_volume_2f0 = volume;
            self.set_music_volume(volume, false, settings, device);
        }
        let flags = self.music_flags_dc;
        if flags & (MUSIC_FLAG_PLAYING | MUSIC_FLAG_FADE_OUT | MUSIC_FLAG_FADE_IN) == 0 {
            self.request_music_type(
                self.current_music_type_b0,
                tick,
                player_has_cell,
                settings,
                device,
                random,
            );
            return;
        }
        if flags & MUSIC_FLAG_LOADED != 0 {
            while let Some(event) = device.next_event(0x006A_D999, graph) {
                if event > 0 && event <= MUSIC_EVENT_LAST_STOP {
                    self.stop_music_playback(device);
                }
            }
        }
        // 006AD9FC: type 8 returns; type 4 and the others ask PlayerCharacter_PruneAndHasCombatOpponents.
        if self.current_music_type_b0 != MUSIC_TYPE_SPECIAL {
            self.music_unported_arms += 1;
        }
    }

    /// IDA: 006AE860, the head a frame with no game sounds reaches, from 0040DBB4. 006AE869 no +0x08 and 006AE876 no 00B16178 return before anything; 006AE883..006AE8AE bl = a current movie in a state other than 0 and 2, or 00578FE0 == 0x3EF, or `menu_mode`; 006AE8B2..006AE8EA from +0xA6 and bl: a menu arriving pauses the game sounds (no 006AD030 that frame), a menu leaving resumes them, and a menu staying runs 006AD030; 006AE8F2 +0xA6 = bl. With bl clear, 006AE9DC..006AEA2C return unless more than 29 ms have passed since +0xCC, which then takes the tick; 006AF319 is 006AD030 after the walk of the sounds. `Seed` (0040DBAA) is 0 on this path. Completion: 60% of the function, 100% of the music gates.
    /// TODO: [ ] 006AE909.. and 006AEA5E..006AF319, the +0x300 and +0x324 walks and the listener, when game sounds exist. The early return of 006AEA2D is the only one taken before them here. With a menu and no sounds, 006AE909's empty map and 006AE97B's empty +0x320 return at 006AF64C, so a menu frame reaches 006AD030 once. With none, the listener calls at 006AEA5E..006AEB03, 006AA730 and the +0xC8 store at 006AF646 aren't run; none of them stores +0xB8.
    pub fn update_frame(
        &mut self,
        frame: &SoundFrame,
        settings: &MusicSettings,
        device: &mut dyn MusicDevice,
        random: &mut dyn FnMut() -> i32,
    ) {
        if !self.direct_sound_08 || !frame.sound_enabled_b16178 {
            return;
        }
        let menu = frame
            .movie_state
            .is_some_and(|state| state != 0 && state != 2)
            || frame.top_menu_mode == 0x3EF
            || frame.menu_mode;
        let was_menu = self.byte_a6 != 0;
        if menu && !was_menu {
            self.pause_game_sounds(true);
        } else if !menu && was_menu {
            self.resume_game_sounds(true);
        } else if menu {
            self.update_music_in_cell(
                frame.time_info_10,
                frame.player_has_cell,
                frame.player_cell_music_type,
                settings,
                device,
                random,
            );
        }
        self.byte_a6 = u8::from(menu);
        if menu {
            return;
        }
        // 006AE9E9 fild, +2^32 when negative; 006AE9FF fstp m32; 006AEA1B __ftol2_sse of (now - +0xCC); 006AEA20 -1, `ja` unsigned.
        let now = frame.time_info_10 as f32;
        let since = ftol(now - self.time_c8[1] as f32) as u32;
        if since.wrapping_sub(1) <= 0x1C {
            return;
        }
        // 006AEA36..006AEA54 a truncating fistp of the rounded tick.
        self.time_c8[1] = ftol(now) as u32;
        self.update_music_in_cell(
            frame.time_info_10,
            frame.player_has_cell,
            frame.player_cell_music_type,
            settings,
            device,
            random,
        );
    }
}

/// IDA: N/A what 006AE860's head reads outside OSSoundGlobals.
#[derive(Clone, Copy, Debug)]
pub struct SoundFrame {
    /// 00B16178.
    pub sound_enabled_b16178: bool,
    /// 00B33428's +0x20, with a current movie.
    pub movie_state: Option<u32>,
    /// 00578FE0.
    pub top_menu_mode: u32,
    /// 0040DBAB 00578F60.
    pub menu_mode: bool,
    /// 00B33E90 +0x10.
    pub time_info_10: u32,
    /// 00B333C4 +0x40.
    pub player_has_cell: bool,
    /// IDA: 004CAD00 TESObjectCELL_ResolveMusicType(cell, null) for the player's interior cell; `None` with no cell or an
    /// exterior one.
    pub player_cell_music_type: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tes::os_globals::{HostWindow, ScriptedOsPlatform, scripted_input_settings};

    const SETTINGS: MusicSettings = MusicSettings {
        music_enabled_b16180: true,
        main_menu_music_volume_b16188: 0.6,
    };
    const TITLE: &[u8] = b"Data\\Music\\Special\\TES4Title.mp3";

    fn sound() -> Box<OsSoundGlobals> {
        let mut settings = scripted_input_settings(true);
        let mut platform = ScriptedOsPlatform::default();
        let mut sound = OsSoundGlobals::new(HostWindow(1), &mut settings, &mut platform);
        sound.master_volume_b8 = 1.0;
        sound
    }

    fn frame(menu_mode: bool, time_info_10: u32) -> SoundFrame {
        SoundFrame {
            sound_enabled_b16178: true,
            movie_state: None,
            top_menu_mode: 0,
            menu_mode,
            time_info_10,
            player_has_cell: false,
            player_cell_music_type: None,
        }
    }

    // A loaded title track, with its next ramp step owed, so that 006AD030 can be seen: it steps +0x2F0 by 0.02.
    fn ramping() -> (Box<OsSoundGlobals>, ScriptedMusicDevice) {
        let mut sound = sound();
        sound.direct_sound_08 = true;
        let mut device = ScriptedMusicDevice::default();
        assert!(sound.load_music_track(
            MUSIC_TYPE_SPECIAL,
            Some(TITLE),
            false,
            &SETTINGS,
            &mut device
        ));
        sound.start_music_playback(&SETTINGS, &mut device);
        sound.music_volume2_2f8 = 1.0;
        sound.music_volume_2f0 = 0.0;
        (sound, device)
    }

    /// 006AE876: `bSoundEnabled=0` with `bMusicEnabled=1` plays the title track (005B5AC0 tests neither) and never updates it. 006AE869: no more with no DirectSound.
    #[test]
    fn a_frame_without_sound_does_not_reach_the_music() {
        let (mut sound, mut device) = ramping();
        let off = SoundFrame {
            sound_enabled_b16178: false,
            ..frame(true, 100)
        };
        sound.update_frame(&off, &SETTINGS, &mut device, &mut || 0);
        sound.update_frame(&off, &SETTINGS, &mut device, &mut || 0);
        assert_eq!(sound.music_volume_2f0, 0.0);
        assert_eq!(sound.byte_a6, 0, "006AE8F2 is past the gate");
        sound.direct_sound_08 = false;
        sound.update_frame(&frame(true, 100), &SETTINGS, &mut device, &mut || 0);
        sound.update_frame(&frame(true, 100), &SETTINGS, &mut device, &mut || 0);
        assert_eq!(sound.music_volume_2f0, 0.0);
    }

    /// 006AE8BA..006AE8EA: the frame a menu arrives pauses and doesn't update; each frame after updates; the frame it leaves resumes.
    #[test]
    fn a_menu_arriving_pauses_and_staying_updates() {
        let (mut sound, mut device) = ramping();
        sound.update_frame(&frame(true, 100), &SETTINGS, &mut device, &mut || 0);
        assert_eq!(sound.game_sounds_paused_a4.load(), 1);
        assert_eq!(sound.byte_a6, 1);
        assert_eq!(sound.music_volume_2f0, 0.0, "no 006AD030 this frame");
        sound.update_frame(&frame(true, 116), &SETTINGS, &mut device, &mut || 0);
        assert_eq!(sound.music_volume_2f0, 0.02);
        sound.update_frame(&frame(false, 132), &SETTINGS, &mut device, &mut || 0);
        assert_eq!(sound.game_sounds_paused_a4.load(), 0);
        assert_eq!(sound.byte_a6, 0);
    }

    /// 006AE883..006AE8A8: a movie the menu doesn't own, or a LoadingMenu on top, counts as a menu; a menu-owned one doesn't.
    #[test]
    fn a_movie_or_the_loading_menu_counts_as_a_menu() {
        let (mut sound, mut device) = ramping();
        let with = |movie_state, top_menu_mode| SoundFrame {
            movie_state,
            top_menu_mode,
            ..frame(false, 10)
        };
        sound.update_frame(&with(Some(1), 0), &SETTINGS, &mut device, &mut || 0);
        assert_eq!(sound.byte_a6, 1);
        sound.byte_a6 = 0;
        sound.update_frame(&with(None, 0x3EF), &SETTINGS, &mut device, &mut || 0);
        assert_eq!(sound.byte_a6, 1);
        sound.byte_a6 = 0;
        sound.update_frame(&with(Some(2), 0), &SETTINGS, &mut device, &mut || 0);
        assert_eq!(sound.byte_a6, 0);
        sound.update_frame(&with(Some(0), 0), &SETTINGS, &mut device, &mut || 0);
        assert_eq!(sound.byte_a6, 0);
    }

    /// 006AE9DC..006AEA54: with no menu, 1 to 29 ms since +0xCC returns before 006AD030, and 0 or 30 goes on and takes the tick.
    #[test]
    fn no_menu_updates_on_its_own_clock() {
        let (mut sound, mut device) = ramping();
        sound.time_c8[1] = 1000;
        sound.update_frame(&frame(false, 1029), &SETTINGS, &mut device, &mut || 0);
        assert_eq!((sound.music_volume_2f0, sound.time_c8[1]), (0.0, 1000));
        sound.update_frame(&frame(false, 1030), &SETTINGS, &mut device, &mut || 0);
        assert_eq!((sound.music_volume_2f0, sound.time_c8[1]), (0.02, 1030));
        sound.update_frame(&frame(false, 1030), &SETTINGS, &mut device, &mut || 0);
        assert_eq!((sound.music_volume_2f0, sound.time_c8[1]), (0.04, 1030));
    }

    fn graph_calls(device: &ScriptedMusicDevice) -> Vec<MusicGraphCall> {
        device
            .trace
            .iter()
            .filter_map(|(_, call)| match call {
                MusicTraceCall::Graph(call) => Some(call.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn title_track_loads_and_starts_in_the_exe_s_order() {
        let mut sound = sound();
        sound.main_menu_music_b3c0ec = 1;
        let mut device = ScriptedMusicDevice::default();
        assert!(sound.load_music_track(
            MUSIC_TYPE_SPECIAL,
            Some(TITLE),
            false,
            &SETTINGS,
            &mut device
        ));
        assert_eq!(sound.music_flags_dc, MUSIC_FLAG_LOADED);
        assert_eq!(sound.current_music_type_b0, MUSIC_TYPE_SPECIAL);
        assert_eq!(sound.music_path_1e4, TITLE);
        sound.start_music_playback(&SETTINGS, &mut device);
        assert_eq!(sound.music_flags_dc, MUSIC_FLAG_LOADED | MUSIC_FLAG_PLAYING);
        // 0.6 * 1.0, squared, 0.36: log10 -0.44370 * 2000 = -887.
        let volume = ((0.6f32 * 0.6f32) as f64).log10() as f32 * 2000.0f32;
        assert_eq!(volume as i32, -887);
        assert_eq!(
            graph_calls(&device),
            vec![
                MusicGraphCall::QueryBasicAudio,
                MusicGraphCall::QueryMediaControl,
                MusicGraphCall::QueryMediaPosition,
                MusicGraphCall::PutCurrentPosition { position_bits: 0 },
                MusicGraphCall::ReleaseMediaPosition,
                MusicGraphCall::Run,
                MusicGraphCall::ReleaseMediaControl,
                MusicGraphCall::PutVolume { volume: -887 },
            ]
        );
    }

    #[test]
    fn a_disabled_setting_or_a_missing_file_loads_nothing() {
        let mut sound = sound();
        let mut device = ScriptedMusicDevice::default();
        let off = MusicSettings {
            music_enabled_b16180: false,
            ..SETTINGS
        };
        assert!(!sound.load_music_track(MUSIC_TYPE_SPECIAL, Some(TITLE), false, &off, &mut device));
        assert!(device.trace.is_empty());
        device.missing_files.push(TITLE.to_vec());
        assert!(!sound.load_music_track(
            MUSIC_TYPE_SPECIAL,
            Some(TITLE),
            false,
            &SETTINGS,
            &mut device
        ));
        assert_eq!(sound.music_flags_dc, 0);
        assert!(sound.music_graph_70.is_none());
        assert_eq!(sound.current_music_type_b0, 0, "006AB306 leaves the type");
    }

    /// 006AB3A8: a failed RenderFile returns with the graph at +0x70 and no track, and a Start does nothing.
    #[test]
    fn a_failed_render_leaves_the_graph_and_no_track() {
        let mut sound = sound();
        let mut device = ScriptedMusicDevice {
            fail_render: true,
            ..ScriptedMusicDevice::default()
        };
        assert!(!sound.load_music_track(
            MUSIC_TYPE_SPECIAL,
            Some(TITLE),
            false,
            &SETTINGS,
            &mut device
        ));
        assert!(sound.music_graph_70.is_some());
        assert_eq!(sound.music_flags_dc, 0);
        sound.start_music_playback(&SETTINGS, &mut device);
        assert!(graph_calls(&device).is_empty());
    }

    /// 006AB270: a second title request while it plays does nothing, as 0040F0AA does after 005B5F20 started it.
    #[test]
    fn a_playing_title_holds_a_second_request() {
        let mut sound = sound();
        let mut device = ScriptedMusicDevice::default();
        sound.load_music_track(
            MUSIC_TYPE_SPECIAL,
            Some(TITLE),
            false,
            &SETTINGS,
            &mut device,
        );
        sound.start_music_playback(&SETTINGS, &mut device);
        let calls = device.trace.len();
        assert!(!sound.load_music_track(
            MUSIC_TYPE_SPECIAL,
            Some(TITLE),
            false,
            &SETTINGS,
            &mut device
        ));
        assert_eq!(device.trace.len(), calls);
    }

    /// 006A8DB0 clears both bits, so the next update reaches 006ACD10, and the same track starts again with a fade-in: the exe's loop.
    #[test]
    fn a_finished_track_starts_again_with_a_fade() {
        let mut sound = sound();
        let mut device = ScriptedMusicDevice::default();
        sound.load_music_track(
            MUSIC_TYPE_SPECIAL,
            Some(TITLE),
            false,
            &SETTINGS,
            &mut device,
        );
        sound.start_music_playback(&SETTINGS, &mut device);
        sound.music_volume_2f0 = 0.6;
        sound.music_volume2_2f8 = 0.6;
        device.events = vec![1];
        device.trace.clear();
        sound.update_music(1000, false, &SETTINGS, &mut device, &mut || 0);
        assert_eq!(sound.music_flags_dc, 0);
        assert!(graph_calls(&device).contains(&MusicGraphCall::Stop));
        device.trace.clear();
        sound.update_music(1016, false, &SETTINGS, &mut device, &mut || 0);
        assert_eq!(
            sound.music_flags_dc,
            MUSIC_FLAG_LOADED | MUSIC_FLAG_PLAYING | MUSIC_FLAG_FADE_IN
        );
        assert_eq!(sound.music_volume_2f0, 0.0);
        assert_eq!(sound.unk_2f4, 0.6);
        assert_eq!(sound.music_start_tick_2e8, 1016);
        assert_eq!(sound.music_fade_end_2ec, 1016 + 1700);
        let opened: Vec<_> = device
            .trace
            .iter()
            .filter(|(_, call)| matches!(call, MusicTraceCall::RenderFile(path) if path == TITLE))
            .collect();
        assert_eq!(opened.len(), 1, "the same path, because the type is 8");
        // The fade: at 1700 * 3/4 the volume is (0.75 - 0.5) * 2 of the stored one.
        sound.update_music(1016 + 1275, false, &SETTINGS, &mut device, &mut || 0);
        assert_eq!(sound.music_volume_2f0, (0.75f32 - 0.5 + (0.75 - 0.5)) * 0.6);
        // Past the end it sets the stored volume, and the next update clears the fade.
        sound.update_music(1016 + 1701, false, &SETTINGS, &mut device, &mut || 0);
        assert_eq!(sound.music_volume_2f0, 0.6);
        sound.update_music(1016 + 1702, false, &SETTINGS, &mut device, &mut || 0);
        assert_eq!(sound.music_flags_dc & MUSIC_FLAG_FADE_IN, 0);
    }

    #[test]
    fn volume_floor_clamps_and_the_stored_values() {
        let mut sound = sound();
        let mut device = ScriptedMusicDevice::default();
        sound.load_music_track(
            MUSIC_TYPE_SPECIAL,
            Some(TITLE),
            false,
            &SETTINGS,
            &mut device,
        );
        device.trace.clear();
        // 0.05 squared is 0.0025, under 0.01.
        sound.set_music_volume(0.05, true, &SETTINGS, &mut device);
        assert_eq!(sound.music_volume_2f0, 0.05);
        assert_eq!(sound.music_volume2_2f8, 0.05);
        assert_eq!(
            graph_calls(&device),
            vec![MusicGraphCall::PutVolume {
                volume: MUSIC_VOLUME_SILENT
            }]
        );
        sound.set_music_volume(7.0, true, &SETTINGS, &mut device);
        assert_eq!(sound.music_volume_2f0, 1.0);
        sound.set_music_volume(-7.0, true, &SETTINGS, &mut device);
        assert_eq!(sound.music_volume_2f0, 0.0);
    }

    #[test]
    fn pause_and_resume_flags() {
        let mut sound = sound();
        let mut device = ScriptedMusicDevice::default();
        sound.load_music_track(
            MUSIC_TYPE_SPECIAL,
            Some(TITLE),
            false,
            &SETTINGS,
            &mut device,
        );
        sound.pause_music_playback(&mut device);
        assert_eq!(sound.music_flags_dc & MUSIC_FLAG_PAUSED, MUSIC_FLAG_PAUSED);
        // Not playing: only the bit.
        device.trace.clear();
        sound.resume_music_playback(&mut device);
        assert_eq!(sound.music_flags_dc & MUSIC_FLAG_PAUSED, 0);
        assert!(device.trace.is_empty());
    }
}
