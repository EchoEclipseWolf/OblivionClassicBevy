//! IDA: N/A the host's sound, the recorded substitution for XAudio2 and Media Foundation in the C++ host and for DirectShow and DirectSound in the original: `bevy_audio` with rodio's decoders, behind the C++ `AudioService`'s policy: six buses, a cap of 24 voices with a priority rule, fades, mute and focus.
//!
//! **Sound never stops the app.** A file that won't decode is refused before `bevy_audio` sees it, because `Decodable::decoder` panics on bad data; a machine with no audio device, or `--no-audio-device`, ends in `AudioDeviceReport` and every voice failing, and the game's own sound state goes on as data. `HostMusicDevice` is what OSSoundGlobals' music sees: the seam `recomp_game::tes::os_sound_music::MusicDevice`.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use bevy::audio::{
    AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, Volume,
};
use bevy::prelude::*;
use recomp_core::host_path;
use recomp_core::logging::{Level, emit};
use recomp_game::tes::os_globals::{CreatedInterface, E_FAIL, S_OK, lock_shared};
use recomp_game::tes::os_sound_music::{
    MUSIC_VOLUME_SILENT, MusicDevice, MusicGraph, MusicGraphCall,
};
use rodio::Source;

/// IDA: N/A the C++ host's limit.
pub const MAX_VOICES: usize = 24;
/// IDA: N/A the C++ host's short-asset cache: 8 MiB in all, 1 MiB an entry.
const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHED_ENTRY: usize = 1024 * 1024;
/// IDA: N/A how many frames a queued voice may wait for its sink before the device counts as absent. `play_queued_audio_system` runs every frame, so two would do.
const SINK_WAIT_FRAMES: u32 = 4;

/// IDA: N/A the C++ `Bus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bus {
    Music,
    Ui,
    Effects,
    Ambience,
    Voice,
    Video,
}

const BUS_COUNT: usize = 6;

/// IDA: N/A a voice's number. 0 is never one.
pub type AudioHandle = u64;

/// IDA: N/A the C++ `PlaybackStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackStatus {
    Unknown,
    Queued,
    Playing,
    Paused,
    Completed,
    Failed,
}

/// IDA: N/A the C++ `Request`.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioRequest {
    pub asset: String,
    pub bus: Bus,
    pub looping: bool,
    pub gain: f32,
    pub priority: u32,
}

impl Default for AudioRequest {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            asset: String::new(),
            bus: Bus::Effects,
            looping: false,
            gain: 1.0,
            priority: 128,
        }
    }
}

/// IDA: N/A the C++ `linear_fade`. Completion: 100%.
#[must_use]
pub fn linear_fade(start: f32, target: f32, elapsed_ms: u32, duration_ms: u32) -> f32 {
    if duration_ms == 0 {
        return target;
    }
    let t = (elapsed_ms as f32 / duration_ms as f32).min(1.0);
    start + (target - start) * t
}

/// IDA: N/A the C++ `output_gain`. Completion: 100%.
#[must_use]
pub fn output_gain(voice: f32, bus: f32, muted: bool, focused: bool) -> f32 {
    if muted || !focused { 0.0 } else { voice * bus }
}

/// IDA: N/A the C++ `bounded_gain`. Completion: 100%.
#[must_use]
pub fn bounded_gain(gain: f32) -> f32 {
    if gain.is_finite() {
        gain.clamp(0.0, 2.0)
    } else {
        0.0
    }
}

/// IDA: N/A the C++ `ui_asset_for_id`: TES4 SOUN editor ID to FNAM paths the C++ verified in Oblivion.esm. Not a table from the exe: the original reaches these through its SOUN forms, which the sound phases will replace this with. The focus sound names a folder, and the host cycles its three files. Completion: 100% of the C++.
#[must_use]
pub fn ui_asset_for_id(sound_id: u32, focus_variant: u32) -> Option<String> {
    match sound_id {
        1 => Some("Sound\\fx\\ui\\menu\\ui_menu_ok.wav".to_owned()),
        2 | 0x15 => Some("Sound\\fx\\ui\\menu\\ui_menu_cancel.wav".to_owned()),
        4 => Some(format!(
            "Sound\\fx\\ui\\menu\\focus\\ui_menu_focus_0{}.wav",
            1 + focus_variant % 3
        )),
        _ => None,
    }
}

/// IDA: N/A the C++ `DecodeProbe`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeProbe {
    pub channels: u16,
    pub sample_rate: u32,
    pub decoded_frames: usize,
    pub non_silent: bool,
}

/// IDA: N/A a whole file's samples, for checks against the file.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedAudio {
    pub channels: u16,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

// IDA: N/A the same decoder `bevy_audio`'s `Decodable` builds, with its failure kept.
fn decoder(bytes: Arc<[u8]>) -> Result<rodio::Decoder<Cursor<Arc<[u8]>>>, String> {
    let length = bytes.len() as u64;
    rodio::Decoder::builder()
        .with_byte_len(length)
        .with_data(Cursor::new(bytes))
        .build()
        .map_err(|error| error.to_string())
}

/// IDA: N/A the C++ `probe_decode`: a bounded prefix with no device. An error for a file `bevy_audio` couldn't play. Completion: 100%.
///
/// # Errors
/// The bytes don't decode, or decode to nothing.
pub fn probe_decode(bytes: Arc<[u8]>) -> Result<DecodeProbe, String> {
    let source = decoder(bytes)?;
    let channels = source.channels().get();
    let sample_rate = source.sample_rate().get();
    let limit = sample_rate as usize * usize::from(channels) * 5;
    let mut samples = 0;
    let mut non_silent = false;
    for sample in source.take(limit) {
        samples += 1;
        non_silent |= sample != 0.0;
    }
    if samples == 0 {
        return Err("Audio produced no PCM samples".to_owned());
    }
    Ok(DecodeProbe {
        channels,
        sample_rate,
        decoded_frames: samples / usize::from(channels),
        non_silent,
    })
}

/// IDA: N/A a whole file, for a check.
///
/// # Errors
/// The bytes don't decode.
pub fn decode_all(bytes: Arc<[u8]>) -> Result<DecodedAudio, String> {
    let source = decoder(bytes)?;
    Ok(DecodedAudio {
        channels: source.channels().get(),
        sample_rate: source.sample_rate().get(),
        samples: source.collect(),
    })
}

/// IDA: N/A what became of the machine's device. Filled by the first voice's fate, since `bevy_audio` keeps its own output private.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioDeviceReport {
    /// No voice has asked yet.
    #[default]
    Unknown,
    /// A sink came, so a device took it.
    Open,
    /// `bevy_audio` found no device: voices fail, and the game's sound state goes on.
    Absent,
    /// `--no-audio-device`, or the virtual display without its own ALSA config.
    Disabled,
}

enum VoiceSource {
    Asset,
    Bytes(Arc<[u8]>),
}

enum AudioCommand {
    Play {
        handle: AudioHandle,
        request: AudioRequest,
        source: VoiceSource,
    },
    Stop(AudioHandle),
    Pause(AudioHandle, bool),
    Fade(AudioHandle, f32, u32),
    SetGain(AudioHandle, f32),
    BusGain(Bus, f32),
    Muted(bool),
    Focus(bool),
}

struct Voice {
    handle: AudioHandle,
    request: AudioRequest,
    gain: f32,
    fade: Option<(f32, f32, Instant, u32)>,
    paused: bool,
    entity: Option<Entity>,
    waited_frames: u32,
}

/// IDA: N/A a change `apply_commands` wants made on a voice's entity.
enum VoiceEffect {
    Spawn {
        handle: AudioHandle,
        bytes: Arc<[u8]>,
        looping: bool,
    },
    Despawn(Option<Entity>),
    Pause(Entity, bool),
}

/// IDA: N/A the C++ `AudioService::State`, as plain data, so that its rules run with no device and no `App`. Completion: 100% of the C++'s voice, bus, fade, cap and status rules. Host substitution: commands apply once a frame on the main thread, where the C++ has a worker, and `bevy_audio`'s output thread decodes.
pub struct AudioMixer {
    commands: Vec<AudioCommand>,
    statuses: HashMap<AudioHandle, PlaybackStatus>,
    voices: Vec<Voice>,
    cache: HashMap<String, Arc<[u8]>>,
    cache_bytes: usize,
    bus_gains: [f32; BUS_COUNT],
    muted: bool,
    focused: bool,
    next_handle: AudioHandle,
    next_focus: u32,
    /// `false` once the device is known absent: a play fails at once, as the C++ does with no engine.
    device_available: bool,
}

impl Default for AudioMixer {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            statuses: HashMap::new(),
            voices: Vec::new(),
            cache: HashMap::new(),
            cache_bytes: 0,
            bus_gains: [1.0; BUS_COUNT],
            muted: false,
            focused: true,
            next_handle: 1,
            next_focus: 0,
            device_available: true,
        }
    }
}

// IDA: N/A a bus's slot.
const fn bus_index(bus: Bus) -> usize {
    match bus {
        Bus::Music => 0,
        Bus::Ui => 1,
        Bus::Effects => 2,
        Bus::Ambience => 3,
        Bus::Voice => 4,
        Bus::Video => 5,
    }
}

impl AudioMixer {
    // IDA: N/A `AudioService::play`'s id, status and queue.
    fn queue_play(&mut self, request: AudioRequest, source: VoiceSource) -> AudioHandle {
        let handle = self.next_handle;
        self.next_handle += 1;
        self.statuses.insert(handle, PlaybackStatus::Queued);
        self.commands.push(AudioCommand::Play {
            handle,
            request,
            source,
        });
        handle
    }

    // IDA: N/A the C++ `set_status`, with its bound.
    fn set_status(&mut self, handle: AudioHandle, status: PlaybackStatus) {
        self.statuses.insert(handle, status);
        // The C++ bound: finished ones go first.
        if self.statuses.len() > 2048 {
            self.statuses.retain(|_, status| {
                !matches!(status, PlaybackStatus::Completed | PlaybackStatus::Failed)
            });
        }
    }

    // IDA: N/A the bytes of an asset, through the short cache unless it's long.
    fn read_asset(
        &mut self,
        asset: &str,
        read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    ) -> Option<Arc<[u8]>> {
        if let Some(bytes) = self.cache.get(asset) {
            return Some(Arc::clone(bytes));
        }
        let bytes: Arc<[u8]> = read(asset)?.into();
        if bytes.len() <= MAX_CACHED_ENTRY && bytes.len() <= MAX_CACHE_BYTES - self.cache_bytes {
            self.cache_bytes += bytes.len();
            self.cache.insert(asset.to_owned(), Arc::clone(&bytes));
        }
        Some(bytes)
    }

    // IDA: N/A the C++ `run` loop's queue, in order, and what each play did to the table.
    fn apply_commands(
        &mut self,
        read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    ) -> Vec<VoiceEffect> {
        let mut effects = Vec::new();
        for command in std::mem::take(&mut self.commands) {
            match command {
                AudioCommand::Play {
                    handle,
                    request,
                    source,
                } => self.apply_play(handle, request, source, read, &mut effects),
                AudioCommand::Stop(handle) => {
                    if let Some(index) = self.voices.iter().position(|voice| voice.handle == handle)
                    {
                        let voice = self.voices.remove(index);
                        effects.push(VoiceEffect::Despawn(voice.entity));
                        self.set_status(handle, PlaybackStatus::Completed);
                    }
                }
                AudioCommand::Pause(handle, paused) => {
                    if let Some(voice) = self.voices.iter_mut().find(|voice| voice.handle == handle)
                    {
                        voice.paused = paused;
                        if let Some(entity) = voice.entity {
                            effects.push(VoiceEffect::Pause(entity, paused));
                        }
                        let status = if paused {
                            PlaybackStatus::Paused
                        } else {
                            PlaybackStatus::Playing
                        };
                        self.set_status(handle, status);
                    }
                }
                AudioCommand::Fade(handle, target, duration) => {
                    if let Some(voice) = self.voices.iter_mut().find(|voice| voice.handle == handle)
                    {
                        let target = bounded_gain(target);
                        voice.fade = Some((voice.gain, target, Instant::now(), duration));
                        if duration == 0 {
                            voice.gain = target;
                            voice.fade = None;
                        }
                    }
                }
                AudioCommand::SetGain(handle, gain) => {
                    if let Some(voice) = self.voices.iter_mut().find(|voice| voice.handle == handle)
                    {
                        voice.gain = bounded_gain(gain);
                        voice.fade = None;
                    }
                }
                AudioCommand::BusGain(bus, gain) => {
                    self.bus_gains[bus_index(bus)] = bounded_gain(gain);
                }
                AudioCommand::Muted(muted) => self.muted = muted,
                AudioCommand::Focus(focused) => self.focused = focused,
            }
        }
        effects
    }

    // IDA: N/A the C++ `apply`'s play arm: the cap, the read, the check, the voice.
    fn apply_play(
        &mut self,
        handle: AudioHandle,
        request: AudioRequest,
        source: VoiceSource,
        read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
        effects: &mut Vec<VoiceEffect>,
    ) {
        if !self.device_available {
            self.set_status(handle, PlaybackStatus::Failed);
            return;
        }
        if self.voices.len() >= MAX_VOICES {
            let victim = self
                .voices
                .iter()
                .enumerate()
                .min_by_key(|(_, voice)| voice.request.priority)
                .map(|(index, _)| index);
            match victim {
                Some(index) if self.voices[index].request.priority <= request.priority => {
                    let voice = self.voices.remove(index);
                    effects.push(VoiceEffect::Despawn(voice.entity));
                    self.set_status(voice.handle, PlaybackStatus::Completed);
                }
                _ => {
                    self.set_status(handle, PlaybackStatus::Failed);
                    return;
                }
            }
        }
        let bytes = match source {
            VoiceSource::Bytes(bytes) => Some(bytes),
            VoiceSource::Asset => self.read_asset(&request.asset, read),
        };
        let Some(bytes) = bytes else {
            emit(
                Level::Error,
                &format!("Audio request {handle} failed: {} not found", request.asset),
            );
            self.set_status(handle, PlaybackStatus::Failed);
            return;
        };
        if let Err(error) = probe_decode(Arc::clone(&bytes)) {
            emit(
                Level::Error,
                &format!("Audio request {handle} failed: {}: {error}", request.asset),
            );
            self.set_status(handle, PlaybackStatus::Failed);
            return;
        }
        emit(
            Level::Info,
            &format!("Audio start handle={handle} asset={}", request.asset),
        );
        effects.push(VoiceEffect::Spawn {
            handle,
            bytes,
            looping: request.looping,
        });
        self.voices.push(Voice {
            handle,
            gain: bounded_gain(request.gain),
            request,
            fade: None,
            paused: false,
            entity: None,
            waited_frames: 0,
        });
        self.set_status(handle, PlaybackStatus::Playing);
    }

    // IDA: N/A the C++ `update`'s fade and `output_gain`: the volume a voice should have now.
    fn audible(&mut self, index: usize, now: Instant) -> f32 {
        let voice = &mut self.voices[index];
        if let Some((start, target, began, duration)) = voice.fade {
            let elapsed = now.duration_since(began).as_millis() as u32;
            voice.gain = linear_fade(start, target, elapsed, duration);
            if elapsed >= duration {
                voice.fade = None;
            }
        }
        output_gain(
            voice.gain,
            self.bus_gains[bus_index(voice.request.bus)],
            self.muted,
            self.focused,
        )
    }

    // IDA: N/A every voice has failed, because no device will take them.
    fn fail_all_voices(&mut self) {
        self.device_available = false;
        let voices = std::mem::take(&mut self.voices);
        for voice in voices {
            self.set_status(voice.handle, PlaybackStatus::Failed);
        }
    }
}

/// IDA: N/A the C++ `AudioService`: the handle anything on the main thread holds. Asset reads, decoding checks and device work happen when the frame applies the queue. Handles stop meaning anything at `stop` or completion. Completion: 100%.
#[derive(Resource, Clone, Default)]
pub struct HostAudio(Arc<Mutex<AudioMixer>>);

impl HostAudio {
    // IDA: N/A the lock.
    fn mixer(&self) -> MutexGuard<'_, AudioMixer> {
        lock_shared(&self.0)
    }

    /// IDA: N/A `AudioService::play`: 0 for an empty asset.
    #[must_use]
    pub fn play(&self, request: AudioRequest) -> AudioHandle {
        if request.asset.is_empty() {
            return 0;
        }
        self.mixer().queue_play(request, VoiceSource::Asset)
    }

    /// IDA: N/A a voice from bytes already read, for the music graph's RenderFile.
    #[must_use]
    pub fn play_bytes(&self, request: AudioRequest, bytes: Arc<[u8]>) -> AudioHandle {
        self.mixer().queue_play(request, VoiceSource::Bytes(bytes))
    }

    /// IDA: N/A `AudioService::play_ui`: 0 for an unknown ID. IDs 1, 2, 4 and 0x15 are the C++'s; the original's is a SOUN form.
    #[must_use]
    pub fn play_ui(&self, sound_id: u32) -> AudioHandle {
        let variant = if sound_id == 4 {
            let mut mixer = self.mixer();
            let variant = mixer.next_focus;
            mixer.next_focus = variant.wrapping_add(1);
            variant
        } else {
            0
        };
        ui_asset_for_id(sound_id, variant).map_or(0, |asset| {
            self.play(AudioRequest {
                asset,
                bus: Bus::Ui,
                priority: 200,
                ..AudioRequest::default()
            })
        })
    }

    /// IDA: N/A `AudioService::stop`.
    pub fn stop(&self, handle: AudioHandle) {
        if handle != 0 {
            self.mixer().commands.push(AudioCommand::Stop(handle));
        }
    }

    /// IDA: N/A `AudioService::pause`.
    pub fn pause(&self, handle: AudioHandle, paused: bool) {
        if handle != 0 {
            self.mixer()
                .commands
                .push(AudioCommand::Pause(handle, paused));
        }
    }

    /// IDA: N/A `AudioService::fade`.
    pub fn fade(&self, handle: AudioHandle, target_gain: f32, milliseconds: u32) {
        if handle != 0 {
            self.mixer()
                .commands
                .push(AudioCommand::Fade(handle, target_gain, milliseconds));
        }
    }

    /// IDA: N/A a voice's gain at once, for `IBasicAudio::put_Volume`.
    pub fn set_gain(&self, handle: AudioHandle, gain: f32) {
        if handle != 0 {
            self.mixer()
                .commands
                .push(AudioCommand::SetGain(handle, gain));
        }
    }

    /// IDA: N/A `AudioService::set_bus_gain`.
    pub fn set_bus_gain(&self, bus: Bus, gain: f32) {
        self.mixer().commands.push(AudioCommand::BusGain(bus, gain));
    }

    /// IDA: N/A `AudioService::set_muted`.
    pub fn set_muted(&self, muted: bool) {
        self.mixer().commands.push(AudioCommand::Muted(muted));
    }

    /// IDA: N/A `AudioService::set_focus`.
    pub fn set_focus(&self, focused: bool) {
        self.mixer().commands.push(AudioCommand::Focus(focused));
    }

    /// IDA: N/A `AudioService::status`.
    #[must_use]
    pub fn status(&self, handle: AudioHandle) -> PlaybackStatus {
        self.mixer()
            .statuses
            .get(&handle)
            .copied()
            .unwrap_or(PlaybackStatus::Unknown)
    }
}

/// IDA: N/A how a voice's asset is read: through the game's own file finder and archives, once the driver has them.
pub type ReadAudioAsset = dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync;

/// IDA: N/A the resource holding `ReadAudioAsset`.
#[derive(Resource, Clone)]
pub struct AudioAssetReader(pub Arc<ReadAudioAsset>);

impl Default for AudioAssetReader {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self(Arc::new(|_| None))
    }
}

/// IDA: N/A a playing voice's entity.
#[derive(Component)]
pub struct HostVoice(pub AudioHandle);

/// IDA: N/A the frame's sound: the queue, then each voice's volume and end. Exclusive, because it spawns in order. Completion: 100% of the host role.
pub fn apply_host_audio(world: &mut World) {
    let audio = world.resource::<HostAudio>().clone();
    let reader = world
        .get_resource::<AudioAssetReader>()
        .cloned()
        .unwrap_or_default();
    let disabled = *world.resource::<AudioDeviceReport>() == AudioDeviceReport::Disabled;
    let mut mixer = audio.mixer();
    if disabled {
        mixer.device_available = false;
    }
    let effects = mixer.apply_commands(&mut |asset| (reader.0)(asset));
    for effect in effects {
        match effect {
            VoiceEffect::Spawn {
                handle,
                bytes,
                looping,
            } => {
                let source = world
                    .resource_mut::<Assets<AudioSource>>()
                    .add(AudioSource { bytes });
                let settings = if looping {
                    PlaybackSettings::LOOP
                } else {
                    PlaybackSettings::ONCE
                };
                // The queue's gain from its first sample: put_Volume comes right after Run.
                let index = mixer.voices.iter().position(|voice| voice.handle == handle);
                let audible = index.map_or(1.0, |index| mixer.audible(index, Instant::now()));
                let settings = settings.with_volume(Volume::Linear(audible));
                let entity = world
                    .spawn((AudioPlayer(source), settings, HostVoice(handle)))
                    .id();
                if let Some(voice) = mixer.voices.iter_mut().find(|voice| voice.handle == handle) {
                    voice.entity = Some(entity);
                }
            }
            VoiceEffect::Despawn(entity) => {
                if let Some(entity) = entity {
                    world.despawn(entity);
                }
            }
            VoiceEffect::Pause(entity, paused) => {
                if let Some(sink) = world.get::<AudioSink>(entity) {
                    if paused {
                        sink.pause();
                    } else {
                        sink.play();
                    }
                }
            }
        }
    }
    let now = Instant::now();
    let mut device_absent = false;
    let mut index = 0;
    while index < mixer.voices.len() {
        let audible = mixer.audible(index, now);
        let voice = &mut mixer.voices[index];
        let Some(entity) = voice.entity else {
            index += 1;
            continue;
        };
        let Some(mut sink) = world.get_mut::<AudioSink>(entity) else {
            voice.waited_frames += 1;
            if voice.waited_frames > SINK_WAIT_FRAMES {
                device_absent = true;
            }
            index += 1;
            continue;
        };
        sink.set_volume(Volume::Linear(audible));
        if voice.paused != sink.is_paused() {
            if voice.paused {
                sink.pause();
            } else {
                sink.play();
            }
        }
        if sink.empty() {
            let handle = voice.handle;
            emit(Level::Debug, &format!("Audio complete handle={handle}"));
            mixer.voices.remove(index);
            mixer.set_status(handle, PlaybackStatus::Completed);
            world.despawn(entity);
            continue;
        }
        index += 1;
        *world.resource_mut::<AudioDeviceReport>() = AudioDeviceReport::Open;
    }
    if device_absent {
        for voice in &mixer.voices {
            if let Some(entity) = voice.entity {
                world.despawn(entity);
            }
        }
        mixer.fail_all_voices();
        *world.resource_mut::<AudioDeviceReport>() = AudioDeviceReport::Absent;
        emit(
            Level::Warning,
            "Audio device unavailable; audio requests remain silent",
        );
    }
}

/// IDA: N/A IBasicAudio's hundredths of a decibel as a linear gain: -10000 is off. Host substitution for `put_Volume`'s device. Completion: 100%.
#[must_use]
pub fn volume_to_gain(volume: i32) -> f32 {
    if volume <= MUSIC_VOLUME_SILENT {
        0.0
    } else {
        10f64.powf(f64::from(volume) / 2000.0) as f32
    }
}

struct HostGraph {
    id: MusicGraph,
    /// RenderFile has checked `path`. Run reads the file again: the voice holds the only copy, so the graphs a looping track leaves behind (006AA170 releases none after a stop) hold no file.
    rendered: bool,
    path: String,
    voice: AudioHandle,
    gain: f32,
    paused: bool,
    ended: bool,
}

/// IDA: N/A `MusicDevice` over `HostAudio`: a filter graph is a voice on the music bus, once. Paths are relative to the game's folder, as the original's are to its process's. Host substitution for DirectShow: RenderFile decodes a prefix with rodio, and Run reads the file for its voice. EC_COMPLETE is `Completed`; a graph on an absent device never ends, so it never restarts. Completion: 100% of the seam.
pub struct HostMusicDevice {
    audio: HostAudio,
    root: PathBuf,
    graphs: Vec<HostGraph>,
    next_graph: u32,
}

impl HostMusicDevice {
    /// IDA: N/A a device with no graph yet.
    #[must_use]
    pub const fn new(audio: HostAudio, root: PathBuf) -> Self {
        Self {
            audio,
            root,
            graphs: Vec::new(),
            next_graph: 1,
        }
    }

    // IDA: N/A a graph by its host value.
    fn graph(&mut self, graph: MusicGraph) -> Option<&mut HostGraph> {
        self.graphs.iter_mut().find(|entry| entry.id == graph)
    }
}

// IDA: N/A the original's path bytes as a host path's text.
fn latin1(path: &[u8]) -> String {
    path.iter().map(|&byte| char::from(byte)).collect()
}

impl MusicDevice for HostMusicDevice {
    /// IDA: N/A 006A8E80's listing: a sorted one, where FindFirstFileA's order is the file system's, a recorded substitution.
    fn music_files(&mut self, _site: u32, folder: &[u8]) -> Vec<Vec<u8>> {
        let Ok(entries) = std::fs::read_dir(host_path::resolve(&self.root, &latin1(folder))) else {
            return Vec::new();
        };
        let mut names: Vec<Vec<u8>> = entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().bytes().collect())
            .filter(|name: &Vec<u8>| {
                name.len() > 4 && name[name.len() - 4..].eq_ignore_ascii_case(b".mp3")
            })
            .collect();
        names.sort();
        names
    }
    /// IDA: N/A `HostMusicDevice`'s side of `MusicDevice`.
    fn access(&mut self, _site: u32, path: &[u8]) -> bool {
        host_path::resolve(&self.root, &latin1(path)).exists()
    }
    /// IDA: N/A `HostMusicDevice`'s side of `MusicDevice`.
    fn create_graph(&mut self, _site: u32) -> CreatedInterface<MusicGraph> {
        let id = MusicGraph(self.next_graph);
        self.next_graph += 1;
        self.graphs.push(HostGraph {
            id,
            rendered: false,
            path: String::new(),
            voice: 0,
            gain: 1.0,
            paused: false,
            ended: false,
        });
        CreatedInterface {
            result: S_OK,
            object: Some(id),
        }
    }
    /// IDA: N/A `HostMusicDevice`'s side of `MusicDevice`.
    fn render_file(&mut self, _site: u32, graph: MusicGraph, path: &[u8]) -> u32 {
        let text = latin1(path);
        let host = host_path::resolve(&self.root, &text);
        let bytes: Option<Arc<[u8]>> = std::fs::read(&host).ok().map(Into::into);
        let usable = bytes.is_some_and(|bytes| match probe_decode(bytes) {
            Ok(_) => true,
            Err(error) => {
                emit(Level::Error, &format!("Music {text} won't decode: {error}"));
                false
            }
        });
        let Some(entry) = self.graph(graph) else {
            return E_FAIL;
        };
        entry.path = text;
        if usable {
            entry.rendered = true;
            S_OK
        } else {
            E_FAIL
        }
    }
    /// IDA: N/A `HostMusicDevice`'s side of `MusicDevice`.
    fn call(&mut self, _site: u32, graph: MusicGraph, call: MusicGraphCall) -> u32 {
        let audio = self.audio.clone();
        let Some(entry) = self.graphs.iter_mut().find(|entry| entry.id == graph) else {
            return E_FAIL;
        };
        match call {
            MusicGraphCall::Run => {
                entry.paused = false;
                if entry.voice == 0 {
                    if !entry.rendered {
                        return E_FAIL;
                    }
                    let host = host_path::resolve(&self.root, &entry.path);
                    let Ok(bytes) = std::fs::read(host) else {
                        return E_FAIL;
                    };
                    entry.ended = false;
                    entry.voice = audio.play_bytes(
                        AudioRequest {
                            asset: entry.path.clone(),
                            bus: Bus::Music,
                            gain: entry.gain,
                            priority: 255,
                            ..AudioRequest::default()
                        },
                        bytes.into(),
                    );
                } else {
                    audio.pause(entry.voice, false);
                }
            }
            MusicGraphCall::Pause => {
                entry.paused = true;
                audio.pause(entry.voice, true);
            }
            MusicGraphCall::Stop => {
                audio.stop(entry.voice);
                entry.voice = 0;
            }
            MusicGraphCall::PutVolume { volume } => {
                entry.gain = volume_to_gain(volume);
                audio.set_gain(entry.voice, entry.gain);
            }
            MusicGraphCall::ReleaseGraph => {
                let voice = entry.voice;
                audio.stop(voice);
                self.graphs.retain(|entry| entry.id != graph);
            }
            MusicGraphCall::QueryMediaControl
            | MusicGraphCall::QueryMediaPosition
            | MusicGraphCall::PutCurrentPosition { .. }
            | MusicGraphCall::ReleaseMediaPosition
            | MusicGraphCall::ReleaseMediaControl
            | MusicGraphCall::QueryBasicAudio
            | MusicGraphCall::ReleaseBasicAudio => {}
        }
        S_OK
    }
    /// IDA: N/A `HostMusicDevice`'s side of `MusicDevice`.
    fn next_event(&mut self, _site: u32, graph: MusicGraph) -> Option<i32> {
        let audio = self.audio.clone();
        let entry = self.graph(graph)?;
        if entry.ended || entry.voice == 0 || audio.status(entry.voice) != PlaybackStatus::Completed
        {
            return None;
        }
        entry.ended = true;
        // EC_COMPLETE.
        Some(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- the C++ `audio_assets` pure checks ----

    #[test]
    fn ui_ids_map_to_their_assets() {
        assert_eq!(
            ui_asset_for_id(1, 0).unwrap(),
            "Sound\\fx\\ui\\menu\\ui_menu_ok.wav"
        );
        assert_eq!(
            ui_asset_for_id(2, 0).unwrap(),
            "Sound\\fx\\ui\\menu\\ui_menu_cancel.wav"
        );
        assert_eq!(ui_asset_for_id(0x15, 0), ui_asset_for_id(2, 0));
        assert_eq!(
            ui_asset_for_id(4, 2).unwrap(),
            "Sound\\fx\\ui\\menu\\focus\\ui_menu_focus_03.wav"
        );
        assert_eq!(ui_asset_for_id(99, 0), None, "an unknown ID is silent");
    }

    #[test]
    fn fade_midpoint_and_end_boundary() {
        assert_eq!(linear_fade(1.0, 0.0, 500, 1000), 0.5);
        assert_eq!(linear_fade(1.0, 0.0, 1000, 1000), 0.0);
        assert_eq!(linear_fade(1.0, 0.0, 1500, 1000), 0.0);
        assert_eq!(linear_fade(1.0, 0.25, 0, 0), 0.25);
    }

    #[test]
    fn mute_focus_and_bus_gain() {
        assert_eq!(output_gain(0.8, 0.5, false, true), 0.4);
        assert_eq!(output_gain(0.8, 0.5, true, true), 0.0);
        assert_eq!(output_gain(0.8, 0.5, false, false), 0.0);
    }

    /// 006AD030's restart makes a new graph per loop and 006AA170 releases none of them (bit 0 is clear after a stop). The graphs stay as state; none holds the file.
    #[test]
    fn a_looping_track_keeps_its_graphs_and_no_copy_of_the_file() {
        let folder =
            std::env::temp_dir().join(format!("recomp_music_loops_{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("t.wav"), wav(4410)).unwrap();
        let audio = HostAudio::default();
        let mut device = HostMusicDevice::new(audio.clone(), folder.clone());
        for loops in 1..=3 {
            let graph = device.create_graph(0).object.unwrap();
            assert_eq!(device.render_file(0, graph, b"t.wav"), S_OK);
            assert_eq!(device.call(0, graph, MusicGraphCall::Run), S_OK);
            let voice = device.graph(graph).unwrap().voice;
            assert_eq!(audio.status(voice), PlaybackStatus::Queued);
            device.call(0, graph, MusicGraphCall::Stop);
            assert_eq!(device.graphs.len(), loops);
        }
        // No graph holds the file: Run reads it, and a gone file fails Run's graph, not the earlier state.
        let graph = device.create_graph(0).object.unwrap();
        assert_eq!(device.render_file(0, graph, b"t.wav"), S_OK);
        std::fs::remove_dir_all(&folder).unwrap();
        assert_eq!(device.call(0, graph, MusicGraphCall::Run), E_FAIL);
    }

    #[test]
    fn a_truncated_wav_fails_without_a_panic() {
        assert!(probe_decode(Arc::from(&b"RIFF1234WAVE"[..])).is_err());
        assert!(probe_decode(Arc::from(&b""[..])).is_err());
    }

    // ---- the voice rules, from audio_service.cpp, with no device ----

    fn wav(frames: usize) -> Vec<u8> {
        // 44-byte PCM header, mono 16-bit at 44100.
        let data = (frames * 2) as u32;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(36 + data).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&44100u32.to_le_bytes());
        bytes.extend_from_slice(&88200u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        for frame in 0..frames {
            bytes.extend_from_slice(&((frame as i16).wrapping_mul(50)).to_le_bytes());
        }
        bytes
    }

    fn apply(mixer: &mut AudioMixer) -> Vec<VoiceEffect> {
        mixer.apply_commands(&mut |_| Some(wav(2000)))
    }

    #[test]
    fn empty_asset_and_unknown_ui_are_no_voice() {
        let audio = HostAudio::default();
        assert_eq!(
            audio.play(AudioRequest {
                asset: String::new(),
                ..AudioRequest::default()
            }),
            0
        );
        assert_eq!(audio.play_ui(99), 0);
        assert_eq!(audio.status(0), PlaybackStatus::Unknown);
    }

    #[test]
    fn a_missing_or_bad_asset_fails_its_voice_alone() {
        let audio = HostAudio::default();
        let missing = audio.play(AudioRequest {
            asset: "missing.wav".to_owned(),
            ..AudioRequest::default()
        });
        assert_eq!(audio.status(missing), PlaybackStatus::Queued);
        let mut mixer = audio.mixer();
        let effects = mixer.apply_commands(&mut |_| None);
        assert!(effects.is_empty());
        drop(mixer);
        assert_eq!(audio.status(missing), PlaybackStatus::Failed);
        let bad = audio.play(AudioRequest {
            asset: "bad.wav".to_owned(),
            ..AudioRequest::default()
        });
        let effects = audio
            .mixer()
            .apply_commands(&mut |_| Some(b"RIFF1234WAVE".to_vec()));
        assert!(effects.is_empty());
        assert_eq!(audio.status(bad), PlaybackStatus::Failed);
    }

    #[test]
    fn the_voice_cap_takes_the_lowest_priority_or_refuses() {
        let audio = HostAudio::default();
        let mut mixer = audio.mixer();
        let request = |priority| AudioRequest {
            asset: "a.wav".to_owned(),
            priority,
            ..AudioRequest::default()
        };
        let first = mixer.queue_play(request(10), VoiceSource::Asset);
        for _ in 1..MAX_VOICES {
            mixer.queue_play(request(100), VoiceSource::Asset);
        }
        assert_eq!(apply(&mut mixer).len(), MAX_VOICES);
        // Below every priority: refused.
        let low = mixer.queue_play(request(5), VoiceSource::Asset);
        assert_eq!(apply(&mut mixer).len(), 0);
        assert_eq!(mixer.statuses[&low], PlaybackStatus::Failed);
        // Above the lowest: it goes.
        let high = mixer.queue_play(request(50), VoiceSource::Asset);
        let effects = apply(&mut mixer);
        assert_eq!(effects.len(), 2);
        assert!(matches!(effects[0], VoiceEffect::Despawn(_)));
        assert_eq!(mixer.statuses[&first], PlaybackStatus::Completed);
        assert_eq!(mixer.statuses[&high], PlaybackStatus::Playing);
        assert_eq!(mixer.voices.len(), MAX_VOICES);
    }

    #[test]
    fn bus_gain_mute_and_focus_reach_a_voice() {
        let audio = HostAudio::default();
        let handle = audio.play(AudioRequest {
            asset: "a.wav".to_owned(),
            bus: Bus::Music,
            gain: 0.5,
            ..AudioRequest::default()
        });
        audio.set_bus_gain(Bus::Music, 0.5);
        audio.set_bus_gain(Bus::Ui, 0.0);
        let mut mixer = audio.mixer();
        apply(&mut mixer);
        let now = Instant::now();
        assert_eq!(mixer.audible(0, now), 0.25);
        mixer.commands.push(AudioCommand::Muted(true));
        apply(&mut mixer);
        assert_eq!(mixer.audible(0, now), 0.0);
        mixer.commands.push(AudioCommand::Muted(false));
        mixer.commands.push(AudioCommand::Focus(false));
        apply(&mut mixer);
        assert_eq!(mixer.audible(0, now), 0.0);
        mixer.commands.push(AudioCommand::Focus(true));
        mixer.commands.push(AudioCommand::Pause(handle, true));
        apply(&mut mixer);
        assert_eq!(mixer.statuses[&handle], PlaybackStatus::Paused);
        mixer.commands.push(AudioCommand::Stop(handle));
        apply(&mut mixer);
        assert_eq!(mixer.statuses[&handle], PlaybackStatus::Completed);
        assert!(mixer.voices.is_empty());
    }

    #[test]
    fn a_fade_runs_and_ends_at_its_target() {
        let audio = HostAudio::default();
        let handle = audio.play(AudioRequest {
            asset: "a.wav".to_owned(),
            ..AudioRequest::default()
        });
        audio.fade(handle, 0.0, 1000);
        let mut mixer = audio.mixer();
        apply(&mut mixer);
        let began = mixer.voices[0].fade.unwrap().2;
        let middle = mixer.audible(0, began + std::time::Duration::from_millis(500));
        assert_eq!(middle, 0.5);
        let end = mixer.audible(0, began + std::time::Duration::from_millis(1000));
        assert_eq!(end, 0.0);
        assert!(mixer.voices[0].fade.is_none());
    }

    #[test]
    fn a_gain_is_bounded() {
        assert_eq!(bounded_gain(9.0), 2.0);
        assert_eq!(bounded_gain(-1.0), 0.0);
        assert_eq!(bounded_gain(f32::NAN), 0.0);
    }

    #[test]
    fn an_absent_device_fails_what_plays_and_what_comes() {
        let audio = HostAudio::default();
        let mut mixer = audio.mixer();
        let playing = mixer.queue_play(
            AudioRequest {
                asset: "a.wav".to_owned(),
                ..AudioRequest::default()
            },
            VoiceSource::Asset,
        );
        apply(&mut mixer);
        mixer.fail_all_voices();
        assert_eq!(mixer.statuses[&playing], PlaybackStatus::Failed);
        let later = mixer.queue_play(
            AudioRequest {
                asset: "a.wav".to_owned(),
                ..AudioRequest::default()
            },
            VoiceSource::Asset,
        );
        assert!(apply(&mut mixer).is_empty());
        assert_eq!(mixer.statuses[&later], PlaybackStatus::Failed);
    }

    /// IBasicAudio's scale: 0 is unity, -2000 a tenth (20 dB), and -887, the title at 0.6 and master 1.0, is 0.36 of unity to 3 places.
    #[test]
    fn volume_scale() {
        assert_eq!(volume_to_gain(0), 1.0);
        assert!((volume_to_gain(-2000) - 0.1).abs() < 1e-6);
        assert_eq!(volume_to_gain(MUSIC_VOLUME_SILENT), 0.0);
        assert!((volume_to_gain(-887) - 0.36).abs() < 0.001);
    }

    /// The graph's calls as the exe makes them reach a voice: one voice at Run, with the put_Volume gain; EC_COMPLETE once it completed; a failed decode is RenderFile's failure.
    #[test]
    fn music_graph_over_a_voice() {
        let folder = std::env::temp_dir().join(format!("recomp_music_{}", std::process::id()));
        std::fs::create_dir_all(folder.join("Data/Music")).unwrap();
        std::fs::write(folder.join("Data/Music/track.wav"), wav(2000)).unwrap();
        std::fs::write(folder.join("Data/Music/bad.mp3"), b"nope").unwrap();
        let audio = HostAudio::default();
        let mut device = HostMusicDevice::new(audio.clone(), folder.clone());
        assert!(
            device.access(0, b"Data\\Music\\TRACK.wav"),
            "any case, as Windows"
        );
        assert!(!device.access(0, b"Data\\Music\\none.wav"));
        let graph = device.create_graph(0).object.unwrap();
        assert_eq!(
            device.render_file(0, graph, b"Data\\Music\\bad.mp3"),
            E_FAIL
        );
        assert_eq!(
            device.render_file(0, graph, b"Data\\Music\\track.wav"),
            S_OK
        );
        device.call(0, graph, MusicGraphCall::Run);
        device.call(0, graph, MusicGraphCall::PutVolume { volume: 0 });
        let voice = device.graph(graph).unwrap().voice;
        assert_ne!(voice, 0);
        apply(&mut audio.mixer());
        assert_eq!(audio.status(voice), PlaybackStatus::Playing);
        assert_eq!(audio.mixer().voices[0].request.bus, Bus::Music);
        assert_eq!(device.next_event(0, graph), None);
        audio.mixer().set_status(voice, PlaybackStatus::Completed);
        assert_eq!(device.next_event(0, graph), Some(1));
        assert_eq!(device.next_event(0, graph), None, "the one event");
        device.call(0, graph, MusicGraphCall::ReleaseGraph);
        assert!(device.graph(graph).is_none());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    fn data_file(relative: &str) -> Option<Arc<[u8]>> {
        let folder = std::env::var_os("OBLIVION_DATA_DIR")?;
        std::fs::read(std::path::Path::new(&folder).join(relative))
            .ok()
            .map(Into::into)
    }

    /// The title, whole: stereo, 44100 Hz, as `ffprobe` reports the file, for its 110.94 s. `media_real` compares it with FFmpeg's own decode, sample by sample.
    #[test]
    fn title_music_decodes_whole() {
        let Some(bytes) = data_file("Music/Special/tes4title.mp3") else {
            return;
        };
        let decoded = decode_all(bytes).unwrap();
        assert_eq!((decoded.channels, decoded.sample_rate), (2, 44100));
        let frames = decoded.samples.len() / 2;
        let seconds = frames as f64 / 44100.0;
        assert!((110.0..112.0).contains(&seconds), "{seconds}");
        assert!(decoded.samples.iter().any(|sample| sample.abs() > 0.1));
    }
}
