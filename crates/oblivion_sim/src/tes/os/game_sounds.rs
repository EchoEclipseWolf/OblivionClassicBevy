//! The game sounds a cell's references start when it attaches: 004CB590 TESObjectCELL_UpdateReferenceSounds, 004D9310 per reference, 006AE4B0 OSSoundGlobals' create, 006AC610 the instance with its WAV (006AABD0) and a folder's random file (006A9CD0), and 006AB540's start, whose 006B6E60 draws a looping sound's start position.
//! Evidence: `evidence/gameplay-run002-2026-11-07/` run-ae, every reference 004D9310 saw in the Imperial Prison's attach, each sound's form, path, flags, instance bytes and draw.
//! Host substitution: the port has no DirectSound. An instance's buffer stands as created whenever its WAV loads and DirectSound would take its format (PCM; mono for 3D), as the original's did under Wine; the buffer's volume, 3D, frequency, position and play calls are counted (`GameSoundBoundaries`).

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::os_globals::OsGlobalsData;
use crate::tes::form_components::TesSoundFile;
use crate::tes::forms_misc::TesSound;

/// IDA: 006B6DC0's 0x58-byte sound instance, the fields the port holds: +0x00 the flags, +0x08 the buffer's bytes (006AABD0's DSBUFFERDESC dwBufferBytes, the data chunk's size), +0x0C the ID, +0x1C the length in milliseconds, +0x2C and +0x30 the start and stop hours, +0x44 the static attenuation, +0x50 the buffer, by presence.
#[derive(Clone, Debug, PartialEq)]
pub struct GameSoundInstance {
    pub flags_00: u32,
    pub buffer_bytes_08: u32,
    pub id_0c: u32,
    pub milliseconds_1c: u32,
    pub start_hour_2c: f32,
    pub stop_hour_30: f32,
    pub static_attenuation_44: u16,
    pub buffer_50: bool,
    /// +0x4A the muted byte (006B7130), with +0x8C's attenuation 10000 while it's set.
    pub muted_4a: u8,
    /// IDA: N/A host substitution: the buffer's DSBSTATUS_PLAYING bit (006B6AF0's GetStatus). Play sets it and Stop clears it; a
    /// buffer that isn't looping never ends by itself here (the move's sounds all loop).
    pub playing: bool,
    /// The file the buffer was made from, lower case.
    pub path: String,
}

/// IDA: 006ABB00's NiTPointerMap<int, TESGameSound*> at OSSoundGlobals +0x300: 37 buckets (+0x04), hash `key % 37` (006A9060).
pub const GAME_SOUND_BUCKETS: u32 = 37;

/// IDA: N/A OSSoundGlobals +0x300, the instances by ID (006ACCA0), and the counts of the DirectSound calls the port doesn't make.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct GameSounds {
    pub instances_300: BTreeMap<u32, GameSoundInstance>,
    /// IDA: N/A the IDs in the order 00452570 inserted them, which with the hash gives the map's walk (`walk_order`).
    pub inserted_300: Vec<u32>,
    pub calls: BTreeMap<u32, u32>,
}

impl GameSounds {
    // IDA: N/A a helper, with no address of its own.
    fn count(&mut self, address: u32) {
        *self.calls.entry(address).or_default() += 1;
    }

    /// IDA: 00452600 NiTPointerMap_NextEntry's walk over +0x300: the buckets in index order (`id % 37`), each chain from its
    /// head; 00452570 SetAt puts a new key at its bucket's head, so within a bucket the newest comes first. Completion: 100%.
    #[must_use]
    pub fn walk_order(&self) -> Vec<u32> {
        let mut order = Vec::with_capacity(self.inserted_300.len());
        for bucket in 0..GAME_SOUND_BUCKETS {
            order.extend(
                self.inserted_300
                    .iter()
                    .rev()
                    .filter(|&&id| id % GAME_SOUND_BUCKETS == bucket),
            );
        }
        order
    }
}

/// IDA: N/A the INI's bSoundEnabled (00B16178) and the archives' bInvalidateOlderFiles, which a game in play lends the grid (`TesGridServices::game_sounds`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameSoundSettings {
    pub sound_enabled_b16178: bool,
    pub invalidate_older_files: bool,
}

/// IDA: N/A what the sounds read that only the host holds: the load host's files (00431020, 00431690, 00431970) and the INI's bSoundEnabled (00B16178) and bInvalidateOlderFiles.
pub struct GameSoundServices<'a> {
    pub host: &'a mut dyn crate::tes::form_arms::DataLoadHost,
    pub sound_enabled_b16178: bool,
    pub invalidate_older_files: bool,
    pub draw: &'a mut dyn FnMut(&mut World) -> i32,
}

/// IDA: 006AE4B0(position, sound, id, new_id, three_d) on OSSoundGlobals: bSoundEnabled clear or no sound answers null. The sound's +0x3C bit 4 (looping) and bit 6 (2D: flag 1, else 3D 2) make the flags, with 0x2000 for `three_d`. A new ID is +0xB4 then incremented; a given one already in +0x300 answers null. 006AC610 with the path (TESSoundFile +0x28, or ""); a failure answers null. 006ACCA0 maps it; +0x2C and +0x30 from +0x43 and +0x42 times 0.09375, +0x44 from +0x40; the volume, 3D and frequency calls (counted); 006AB540(id, looping); FormHeap(4) the handle, which holds the ID. Completion: 100%.
pub(crate) fn create_sound(
    world: &mut World,
    _position: [f32; 3],
    sound: Option<Entity>,
    id: u32,
    new_id: bool,
    three_d: bool,
    services: &mut GameSoundServices<'_>,
) -> Result<Option<u32>, GameError> {
    let Some(sound) = sound else {
        return Ok(None);
    };
    if !services.sound_enabled_b16178 {
        return Ok(None);
    }
    let data = world.get::<TesSound>(sound).expect("a TESSound").clone();
    let mut flags = data.flags_3c & 0x10;
    flags |= if data.flags_3c & 0x40 != 0 { 1 } else { 2 };
    if three_d {
        flags |= 0x2000;
    }
    let id = if new_id {
        let mut os = world.resource_mut::<OsGlobalsData>();
        let sound_globals = os.sound_24.as_mut().expect("tested by the caller");
        let id = sound_globals.raw_b4;
        sound_globals.raw_b4 = id.wrapping_add(1);
        id
    } else {
        if world
            .get_resource::<GameSounds>()
            .is_some_and(|sounds| sounds.instances_300.contains_key(&id))
        {
            return Ok(None);
        }
        id
    };
    let path = world
        .get::<TesSoundFile>(sound)
        .and_then(|file| file.path_04.data_00.clone())
        .map(|bytes| {
            let text = bytes.split(|&byte| byte == 0).next().unwrap_or_default();
            String::from_utf8_lossy(text).into_owned()
        })
        .unwrap_or_default();
    let Some(mut instance) = create_instance(world, &path, flags, id, services)? else {
        return Ok(None);
    };
    instance.start_hour_2c = f32::from(data.word_42 >> 8) * 0.09375;
    instance.stop_hour_30 = f32::from(data.word_42 & 0xFF) * 0.09375;
    instance.static_attenuation_44 = data.static_attenuation_40;
    let mut sounds = world.get_resource_or_init::<GameSounds>();
    // 006AE59F 006ACCA0: a different instance under the ID would warn "AudioID Collision", stop and drop it.
    if sounds.instances_300.contains_key(&id) {
        return Err(GameError::unclassified(
            0x006ACCD0,
            "006ACCD0: an AudioID collision",
        ));
    }
    sounds.instances_300.insert(id, instance);
    sounds.inserted_300.push(id);
    // 006AE5EF, 006AE5FC SoundInstance_ApplyVolume; 006AE636 and 006AE657 the 3D distances and position, 006AE665 the listener's commit, for a 3D sound; 006AE6C5 the frequency.
    sounds.count(0x006B_6F20);
    sounds.count(0x006B_6F20);
    if data.flags_3c & 0x40 == 0 {
        sounds.count(0x006B_6C60);
        sounds.count(0x006B_6BE0);
        sounds.count(0x006A_E665);
    }
    sounds.count(0x006B_6B20);
    start_sound(world, id, flags & 0x10 != 0, services)?;
    Ok(Some(id))
}

/// IDA: 006AC610(&instance, path, flags, id): bSoundEnabled clear answers null. A null ID takes +0xB4 (not here). 006B6DC0 the instance. The path is kept when it starts "dat" or ".\data", otherwise "data\sound\" goes before it; lower case. A path with "voice" takes 006A9E00, a stop. Otherwise a path ending in '\' takes 006A9CD0's random file; 006AABD0 on it, then on the original path; both failing frees the instance (null, -1). +0x08 the buffer's bytes. The lip file arm (00B161C0) is a stop. Completion: 100% of a sound effect.
fn create_instance(
    world: &mut World,
    path: &str,
    flags: u32,
    id: u32,
    services: &mut GameSoundServices<'_>,
) -> Result<Option<GameSoundInstance>, GameError> {
    let lower = path.to_ascii_lowercase();
    let mut full = if lower.starts_with("dat") || lower.starts_with(".\\data") {
        lower.clone()
    } else {
        format!("data\\sound\\{lower}")
    };
    if full.contains("voice") {
        return Err(GameError::unclassified(
            0x006AC70E,
            "006AC70E: 006A9E00, a voice file",
        ));
    }
    if full.ends_with('\\') && !pick_folder_file(world, &mut full, services)? {
        return Ok(None);
    }
    let loaded = load_wave(&full, flags, services).or_else(|| load_wave(path, flags, services));
    let Some((bytes, milliseconds)) = loaded else {
        return Ok(None);
    };
    Ok(Some(GameSoundInstance {
        flags_00: flags,
        buffer_bytes_08: bytes,
        id_0c: id,
        milliseconds_1c: milliseconds,
        start_hour_2c: 0.0,
        stop_hour_30: 0.0,
        static_attenuation_44: 0,
        buffer_50: true,
        muted_4a: 0,
        playing: false,
        path: full,
    }))
}

/// IDA: 006A9CD0(folder) with bInvalidateOlderFiles set: 00431970 for "<folder>*.wav" (mask 8); none answers false; otherwise Random_Next(0), unsigned modulo their count, picks one, copied over `folder`. With the setting clear, 0042DDF0 decides, a stop. Completion: 100% of the set arm.
fn pick_folder_file(
    world: &mut World,
    folder: &mut String,
    services: &mut GameSoundServices<'_>,
) -> Result<bool, GameError> {
    if !services.invalidate_older_files {
        return Err(GameError::unclassified(
            0x006A9D16,
            "006A9D16: 0042DDF0 with bInvalidateOlderFiles clear",
        ));
    }
    let pattern = format!("{folder}*.wav");
    let files = services.host.find_matching_files(&pattern, folder, 8);
    if files.is_empty() {
        return Ok(false);
    }
    let random = (services.draw)(world) as u32;
    let chosen = (random % files.len() as u32) as usize;
    *folder = files[chosen].clone();
    Ok(true)
}

/// IDA: 006AABD0(path, flags): DirectSound absent answers 0. 00431020 finds the file (else "data\sound\" + path), mmioOpen through 006AAAA0's BSFile proc; RIFF, a 'fmt ' chunk of 16 bytes or more whose tag is 1 (PCM), then 'data', whose size is the buffer's dwBufferBytes. DirectSound's CreateSoundBuffer refuses a 3D (flag 2) buffer of more than one channel ("Attempting to play %i channel sound ... in 3D"). Answers the bytes and the length in milliseconds: bytes / (bits / 8) / (rate / 1000), unsigned. Completion: 100% of the parse; DirectSound's other refusals aren't modelled.
fn load_wave(path: &str, flags: u32, services: &mut GameSoundServices<'_>) -> Option<(u32, u32)> {
    let path = if services.host.find_file(path, 0, -1) != 0 {
        path.to_owned()
    } else {
        let prefixed = format!("data\\sound\\{path}");
        if services.host.find_file(&prefixed, 0, -1) == 0 {
            return None;
        }
        prefixed
    };
    let bytes = services.host.read_data_file(&path)?;
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().expect("four")))
    };
    if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut at = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    let mut data_size = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = word(at + 4)? as usize;
        let body = at + 8;
        if id == b"fmt " && format.is_none() {
            if size < 16 {
                return None;
            }
            let half = |offset: usize| {
                u16::from_le_bytes([bytes[body + offset], bytes[body + offset + 1]])
            };
            format = Some((half(0), half(2), word(body + 4)?, half(14)));
        } else if id == b"data" && format.is_some() {
            data_size = Some(size as u32);
            break;
        }
        at = body + size + (size & 1);
    }
    let (tag, channels, rate, bits) = format?;
    if tag != 1 {
        return None;
    }
    let data_size = data_size?;
    if flags & 2 != 0 && channels > 1 {
        return None;
    }
    let block = u32::from(bits >> 3).max(1);
    let per_millisecond = (rate / 1000).max(1);
    Some((data_size, data_size / block / per_millisecond))
}

/// IDA: 006AB540(id, looping): the instance in +0x300, bSoundEnabled set, 006B7050's time and weather test true (no weather mask; start and stop hours closer than 0.09375 pass, others a stop here). The volume (counted); +0xA4 paused or not, the arm continues while +0xA5 is clear or the instance flagged 0x20; a 3D instance is muted beyond its distance (counted). Neither looping nor flagged 0x10: 006B6E60(false); otherwise 006B6E60(true). Completion: 100% with +0xA5 clear.
fn start_sound(
    world: &mut World,
    id: u32,
    looping: bool,
    services: &mut GameSoundServices<'_>,
) -> Result<(), GameError> {
    let Some(instance) = world
        .get_resource::<GameSounds>()
        .and_then(|sounds| sounds.instances_300.get(&id))
        .cloned()
    else {
        return Ok(());
    };
    if !services.sound_enabled_b16178 {
        return Ok(());
    }
    if (instance.stop_hour_30 - instance.start_hour_2c).abs() >= 0.09375 {
        return Err(GameError::unclassified(
            0x006B7073,
            "006B7073: a sound with hours, against the sky's",
        ));
    }
    let byte_a5 = world
        .resource::<OsGlobalsData>()
        .sound_24
        .as_ref()
        .expect("tested by the caller")
        .byte_a5;
    if byte_a5 != 0 && instance.flags_00 & 0x20 == 0 {
        return Err(GameError::unclassified(
            0x006AB5A8,
            "006AB5A8: 006B6B90's frequency arm",
        ));
    }
    {
        let mut sounds = world.resource_mut::<GameSounds>();
        sounds.count(0x006B_6F20);
        if instance.flags_00 & 2 != 0 {
            sounds.count(0x006B_7130);
        }
    }
    set_playback(
        world,
        id,
        instance.flags_00 & 0x10 != 0 || looping,
        &mut *services.draw,
    );
    Ok(())
}

/// IDA: 006B6E60 SoundInstance_SetPlaybackEnabled(enabled): no buffer (+0x50) answers E_FAIL. The play flags are 4 with +0x00 bit 2, else 8. Enabled: +0x00 |= 0x10; with +0x08, Random_Next(0) unsigned modulo it is the buffer's SetCurrentPosition (vt+0x34); then Play (vt+0x30) with the flags | 1, looping. Disabled: bit 4 cleared, then Play with the flags alone, once (not Stop). A refused Play (E_INVALIDARG) retries with 1 or 0, the same here. Completion: 100%; the buffer's calls are counted, and `playing` stands for the buffer's status.
fn set_playback(
    world: &mut World,
    id: u32,
    enabled: bool,
    draw: &mut dyn FnMut(&mut World) -> i32,
) {
    let instance = world.resource::<GameSounds>().instances_300[&id].clone();
    if !instance.buffer_50 {
        return;
    }
    if enabled {
        world
            .resource_mut::<GameSounds>()
            .instances_300
            .get_mut(&id)
            .expect("present")
            .flags_00 |= 0x10;
        if instance.buffer_bytes_08 != 0 {
            let _position = draw(world) as u32 % instance.buffer_bytes_08;
            world.resource_mut::<GameSounds>().count(0x006B_6EB4);
        }
        world.resource_mut::<GameSounds>().count(0x006B_6EC1);
    } else {
        world
            .resource_mut::<GameSounds>()
            .instances_300
            .get_mut(&id)
            .expect("present")
            .flags_00 &= !0x10;
        world.resource_mut::<GameSounds>().count(0x006B_6EE4);
    }
    world
        .resource_mut::<GameSounds>()
        .instances_300
        .get_mut(&id)
        .expect("present")
        .playing = true;
}

/// IDA: 006B7130 SoundInstance_SetMuted(muted): a change of +0x4A (1 muted, with +0x8C = 10000; 0 with +0x8C = 0) applies the
/// volume twice (006B6F20, counted); no change returns. Completion: 100%.
fn set_muted(world: &mut World, id: u32, muted: bool) {
    let mut sounds = world.resource_mut::<GameSounds>();
    let instance = sounds.instances_300.get_mut(&id).expect("present");
    if (instance.muted_4a != 0) == muted {
        return;
    }
    instance.muted_4a = u8::from(muted);
    sounds.count(0x006B_6F20);
    sounds.count(0x006B_6F20);
}

/// IDA: 006B6AA0 SoundInstance_StopBuffer: the buffer's Stop (vt+0x48), E_FAIL with none. Completion: 100%; Stop is counted.
fn stop_buffer(world: &mut World, id: u32) {
    let mut sounds = world.resource_mut::<GameSounds>();
    let instance = sounds.instances_300.get_mut(&id).expect("present");
    if !instance.buffer_50 {
        return;
    }
    instance.playing = false;
    sounds.count(0x006B_6AA0);
}

// IDA: N/A OSSoundGlobals +0xA4, the shared byte the movie thread reads.
fn set_paused_a4(world: &World, paused: bool) {
    let Some(sound) = world
        .get_resource::<OsGlobalsData>()
        .and_then(|os| os.sound_24.as_ref())
    else {
        return;
    };
    if paused {
        sound.game_sounds_paused_a4.pause(true);
    } else {
        sound.game_sounds_paused_a4.resume(true);
    }
}

// IDA: N/A the +0x300 walk's IDs, taken before the loop as 00452600 visits them.
fn walk(world: &World) -> Vec<u32> {
    world
        .get_resource::<GameSounds>()
        .map(GameSounds::walk_order)
        .unwrap_or_default()
}

/// IDA: 006A9AA0 OSSoundGlobals_ResumeFlag10Sounds, 00440AF0's close: over the +0x300 walk, each instance with +0x00 bit 4
/// loses 0x200, then a 2D one (bit 0) is unmuted (006B7130(0)) and any other gets 006B6E60(1), which draws. No
/// bSoundEnabled test. Completion: 100%.
pub fn resume_flag10_sounds(world: &mut World, draw: &mut dyn FnMut(&mut World) -> i32) {
    for id in walk(world) {
        let flags = world.resource::<GameSounds>().instances_300[&id].flags_00;
        if flags & 0x10 == 0 {
            continue;
        }
        let flags = flags & !0x200;
        world
            .resource_mut::<GameSounds>()
            .instances_300
            .get_mut(&id)
            .expect("present")
            .flags_00 = flags;
        if flags & 1 != 0 {
            set_muted(world, id, false);
        } else {
            set_playback(world, id, true, draw);
        }
    }
}

/// IDA: 006A9B40 OSSoundGlobals_PauseGameSounds: +0xA4 = 1; with bSoundEnabled, over the +0x300 walk each instance whose buffer
/// plays (006B6AF0) and isn't flagged 0x20 gains 0x200, then is muted (006B7130(1)) when playback is enabled (bit 4) and 2D
/// (bit 0), else stopped (006B6AA0). Completion: 100%.
pub fn pause_game_sounds(world: &mut World, sound_enabled: bool) {
    set_paused_a4(world, true);
    if !sound_enabled {
        return;
    }
    for id in walk(world) {
        let instance = world.resource::<GameSounds>().instances_300[&id].clone();
        if !(instance.buffer_50 && instance.playing) || instance.flags_00 & 0x20 != 0 {
            continue;
        }
        let flags = instance.flags_00 | 0x200;
        world
            .resource_mut::<GameSounds>()
            .instances_300
            .get_mut(&id)
            .expect("present")
            .flags_00 = flags;
        if flags & 0x10 != 0 && flags & 1 != 0 {
            set_muted(world, id, true);
        } else {
            stop_buffer(world, id);
        }
    }
}

/// IDA: 006A9C00 OSSoundGlobals_ResumeGameSounds: +0xA4 = 0; with bSoundEnabled, over the +0x300 walk each instance with 0x200
/// whose buffer isn't playing, or that is 2D (bit 0), loses 0x200; then with playback enabled (bit 4) a 2D one is unmuted and
/// 006B6E60(1) runs (a draw), otherwise 006B6E60(0); then 006B6F20 the volume (counted). Completion: 100%.
pub fn resume_game_sounds(
    world: &mut World,
    sound_enabled: bool,
    draw: &mut dyn FnMut(&mut World) -> i32,
) {
    set_paused_a4(world, false);
    if !sound_enabled {
        return;
    }
    for id in walk(world) {
        let instance = world.resource::<GameSounds>().instances_300[&id].clone();
        if instance.flags_00 & 0x200 == 0
            || (instance.buffer_50 && instance.playing && instance.flags_00 & 1 == 0)
        {
            continue;
        }
        let flags = instance.flags_00 ^ 0x200;
        world
            .resource_mut::<GameSounds>()
            .instances_300
            .get_mut(&id)
            .expect("present")
            .flags_00 = flags;
        if flags & 0x10 != 0 {
            if flags & 1 != 0 {
                set_muted(world, id, false);
            }
            set_playback(world, id, true, draw);
        } else {
            set_playback(world, id, false, draw);
        }
        world.resource_mut::<GameSounds>().count(0x006B_6F20);
    }
}

/// IDA: N/A the main thread's draw for the walks above: the host's CRT (`ScriptRandom`), as 006A9D86's.
/// # Errors
/// No host CRT.
pub fn host_draw(world: &World) -> Result<impl FnMut(&mut World) -> i32 + use<>, GameError> {
    let random = world
        .get_non_send::<crate::tes::form_arms::ScriptRandom>()
        .ok_or(GameError::unclassified(
            0x006B6E8B,
            "006B6E8B: Random_Next with no host CRT",
        ))?
        .0
        .clone();
    Ok(move |world: &mut World| random(world))
}

/// IDA: 006A9C00 OSSoundGlobals_ResumeGameSounds as a caller with no draw of its own reaches it (005ADC76): `resume_game_sounds`
/// with the host's CRT. Only an instance marked 0x200 draws, so with none marked (every startup close) no CRT is needed.
/// Completion: 100%.
/// # Errors
/// A marked instance with no host CRT.
pub fn resume_game_sounds_checked(world: &mut World, sound_enabled: bool) -> Result<(), GameError> {
    let marked = world.get_resource::<GameSounds>().is_some_and(|sounds| {
        sounds
            .instances_300
            .values()
            .any(|i| i.flags_00 & 0x200 != 0)
    });
    if marked && sound_enabled {
        let mut draw = host_draw(world)?;
        resume_game_sounds(world, sound_enabled, &mut draw);
    } else {
        resume_game_sounds(world, sound_enabled, &mut |_| 0);
    }
    Ok(())
}
