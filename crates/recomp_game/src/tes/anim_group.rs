//! TESAnimGroup: the animation group a KF's sequence names, read from its text keys (`0051B490`), and the 43-row table at `00B102E0`.

use bevy_ecs::prelude::*;
use recomp_core::crt::{atol, stricmp, strnicmp};
use recomp_core::reader::StreamError;

use crate::bs::model_loader::ModelBuildServices;
use crate::gamebryo::controller_sequence::{ControllerSequence, interp_item_name};
use crate::gamebryo::extra_data::TextKeyExtraData;

/// IDA: one 0x24-byte row of 00B102E0: +0x00 the name, +0x04 (byte) more than one sequence may hold the group (`AnimSequenceMultiple`), +0x08 the section (00B10900 row 5's name: 0 lower body .. 6 upper body), +0x0C the kind (the column of 00B10900's key names, and 00472030's and 00476D10's switch), +0x10..+0x20 the keys' indexes into 00B10900 (-1 ends them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimGroupInfo {
    pub name: &'static [u8],
    pub multiple_04: bool,
    pub section_08: u32,
    pub kind_0c: u32,
    pub keys_10: [i32; 5],
}

const fn row(
    name: &'static [u8],
    multiple: bool,
    section: u32,
    kind: u32,
    keys: [i32; 5],
) -> AnimGroupInfo {
    AnimGroupInfo {
        name,
        multiple_04: multiple,
        section_08: section,
        kind_0c: kind,
        keys_10: keys,
    }
}

const TWO: [i32; 5] = [0, 1, -1, 0, 0];
const THREE: [i32; 5] = [0, 1, 2, -1, 0];

/// IDA: 00B102E0, the 43 groups (0x2B rows), dumped from the exe.
pub const ANIM_GROUPS: [AnimGroupInfo; 43] = [
    row(b"Idle", false, 0, 1, TWO),
    row(b"DynamicIdle", false, 0, 1, TWO),
    row(b"SpecialIdle", false, 4, 0, TWO),
    row(b"Forward", false, 0, 1, TWO),
    row(b"Backward", false, 0, 1, TWO),
    row(b"Left", false, 0, 1, TWO),
    row(b"Right", false, 0, 1, TWO),
    row(b"FastForward", false, 0, 1, TWO),
    row(b"FastBackward", false, 0, 1, TWO),
    row(b"FastLeft", false, 0, 1, TWO),
    row(b"FastRight", false, 0, 1, TWO),
    row(b"DodgeForward", false, 0, 0, TWO),
    row(b"DodgeBack", false, 0, 0, TWO),
    row(b"DodgeLeft", false, 0, 0, TWO),
    row(b"DodgeRight", false, 0, 0, TWO),
    row(b"TurnLeft", false, 0, 1, TWO),
    row(b"TurnRight", false, 0, 1, TWO),
    row(b"Equip", false, 3, 2, THREE),
    row(b"Unequip", false, 3, 3, THREE),
    row(b"AttackBow", false, 3, 7, [0, 1, 2, 3, 4]),
    row(b"AttackLeft", true, 3, 4, [0, 1, 2, 3, -1]),
    row(b"AttackRight", true, 3, 4, [0, 1, 2, 3, -1]),
    row(b"AttackPower", false, 3, 5, THREE),
    row(b"AttackForwardPower", false, 3, 5, THREE),
    row(b"AttackBackPower", false, 3, 5, THREE),
    row(b"AttackLeftPower", false, 3, 5, THREE),
    row(b"AttackRightPower", false, 3, 5, THREE),
    row(b"BlockIdle", false, 1, 1, TWO),
    row(b"BlockHit", true, 3, 0, TWO),
    row(b"BlockAttack", true, 3, 6, THREE),
    row(b"Recoil", true, 3, 0, TWO),
    row(b"Stagger", true, 0, 0, TWO),
    row(b"Death", true, 5, 0, TWO),
    row(b"TorchIdle", false, 2, 1, TWO),
    row(b"CastSelf", true, 1, 5, THREE),
    row(b"CastTouch", true, 1, 5, THREE),
    row(b"CastTarget", true, 1, 5, THREE),
    row(b"CastSelfAlt", true, 1, 5, THREE),
    row(b"CastTouchAlt", true, 1, 5, THREE),
    row(b"CastTargetAlt", true, 1, 5, THREE),
    row(b"JumpStart", false, 0, 0, TWO),
    row(b"JumpLoop", false, 0, 1, TWO),
    row(b"JumpLand", false, 0, 0, TWO),
];

/// IDA: 00B10900 rows 0..4, the key names by key index and kind (8 columns); an empty string ends a kind's keys.
pub const KEY_NAMES: [[&[u8]; 8]; 5] = [
    [b"Start"; 8],
    [
        b"End", b"End", b"Attach", b"Detach", b"Hit", b"Hit", b"Attack", b"Attach",
    ],
    [b"", b"", b"End", b"End", b"a:", b"End", b"End", b"Hold"],
    [b"", b"", b"", b"", b"End", b"", b"", b"Release"],
    [b"", b"", b"", b"", b"", b"", b"", b"End"],
];

/// IDA: 00B102B8, the movement prefixes of a KF's file name (index 0 none).
const MOVE_PREFIXES: [&[u8]; 4] = [b"", b"Sneak", b"Swim", b"Fly"];
/// IDA: 00B102C8, the weapon prefixes after it (index 0 none).
const HAND_PREFIXES: [&[u8]; 6] = [b"", b"HandToHand", b"OneHand", b"TwoHand", b"Staff", b"Bow"];
/// IDA: 00B102C8's name by index (00472030's warning), empty past the table.
#[must_use]
pub fn hand_prefix(index: usize) -> &'static [u8] {
    HAND_PREFIXES.get(index).copied().unwrap_or_default()
}

/// IDA: 00B102B8's name by index (00472030's warning), empty past the table.
#[must_use]
pub fn movement_prefix(index: usize) -> &'static [u8] {
    MOVE_PREFIXES.get(index).copied().unwrap_or_default()
}

/// IDA: 00B109C0, an "Enum:" note's sixteen names.
const ENUM_NAMES: [&[u8]; 16] = [
    b"Left",
    b"Right",
    b"BackLeft",
    b"BackRight",
    b"Idle",
    b"Aware",
    b"Attack",
    b"Hit",
    b"Death",
    b"Equip",
    b"Unequip",
    b"Face",
    b"StopSounds",
    b"HitShader",
    b"RiderSoft",
    b"RiderLoud",
];

/// IDA: 0051ACF0's 16-byte entry (+0x28): +0x00 the time, +0x04 an Enum's index or a Sound's volume (`ftol` of 250 x the second value; -6 unset), +0x08 an Enum's key index | its line's offset << 8 or a Sound's float (1.0 unset; raw bits), +0x0C the TESSound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimGroupNote {
    pub time_00: u32,
    pub value_04: i32,
    pub value_08: u32,
    pub sound_0c: Option<Entity>,
}

/// IDA: 0051AB30 TESAnimGroup (0x2C, a NiRefObject, vtable A541C0 by 0051AB30): +0x08 the group (u16: the group's row in the low byte, the weapon prefix's index in bits 8..11 and the movement's in 12..15), +0x0C the action count and +0x10 their times (0051AA10: the kind's key count, zeroed), +0x14 the accumulated movement per second (00472030 by 004BEF50), +0x20 an "m:" note's byte, +0x21 a "Blend:" note's value, +0x24 / +0x28 the Sound: and Enum: notes. Its refcount isn't kept: the KFModel and every BSAnimGroupSequence share it.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct TesAnimGroup {
    pub group_08: u16,
    pub times_10: Vec<f32>,
    pub movement_14: [f32; 3],
    pub move_byte_20: u8,
    pub blend_21: u8,
    pub notes_28: Vec<AnimGroupNote>,
}

impl TesAnimGroup {
    /// IDA: 0051AC70, +0x08's low byte: the group's row.
    #[must_use]
    pub const fn group_type(&self) -> u8 {
        self.group_08 as u8
    }

    // IDA: 0051AB30(group): the movement zeroed, +0x20 / +0x21 zero, 0051AA10(group): +0x08, the count of 00B10900's nonempty names in the row's kind column (up to 5) into +0x0C and a zeroed array, +0x24 / +0x28 empty.
    fn new(group: u16) -> Self {
        let kind = ANIM_GROUPS[usize::from(group as u8)].kind_0c as usize;
        let count = KEY_NAMES
            .iter()
            .take_while(|names| !names[kind].is_empty())
            .count();
        Self {
            group_08: group,
            times_10: vec![0.0; count],
            movement_14: [0.0; 3],
            move_byte_20: 0,
            blend_21: 0,
            notes_28: Vec::new(),
        }
    }

    // IDA: 0051ACF0(count): a grown array keeps the old entries and fills the new ones with { 0.0, -6, 1.0, null }.
    fn resize_notes(&mut self, count: usize) {
        self.notes_28.resize(
            count,
            AnimGroupNote {
                time_00: 0,
                value_04: -6,
                value_08: 1.0_f32.to_bits(),
                sound_0c: None,
            },
        );
    }
}

/// IDA: 0051AE20 TESAnimGroup_GetActionTime(action): a negative action warns "Invalid anim group action (action %d too small) in %s (%04X)." and answers 0; one past +0x0C warns "... too big %d max ..." and answers 0; a null array 0; a NaN time warns "Time %d in group %04X is QNAN" and answers 0. Completion: 100%.
pub fn action_time(group: &TesAnimGroup, action: i32, warning: &mut dyn FnMut(Vec<u8>)) -> f32 {
    let name =
        String::from_utf8_lossy(ANIM_GROUPS[usize::from(group.group_type())].name).into_owned();
    if action < 0 {
        warning(
            format!(
                "Invalid anim group action (action {action} too small) in {name} ({:04X}).",
                group.group_08
            )
            .into_bytes(),
        );
        return 0.0;
    }
    let Some(&time) = group.times_10.get(action as usize) else {
        warning(
            format!(
                "Invalid anim group action (action {action} too big {} max) in {name} ({:04x}).",
                group.times_10.len(),
                group.group_08
            )
            .into_bytes(),
        );
        return 0.0;
    };
    if time.is_nan() {
        warning(format!("Time {action} in group {:04X} is QNAN", group.group_08).into_bytes());
        return 0.0;
    }
    time
}

// IDA: N/A the CRT `atof` (0098xxxx) on a byte string: leading white space, a sign, digits, a fraction and an exponent; nothing parsed is 0.
fn c_atof(text: &[u8]) -> f64 {
    let mut at = text
        .iter()
        .position(|&byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C))
        .unwrap_or(text.len());
    let start = at;
    if matches!(text.get(at), Some(b'+' | b'-')) {
        at += 1;
    }
    while text.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
    }
    if text.get(at) == Some(&b'.') {
        at += 1;
        while text.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
    }
    let mantissa = at;
    if matches!(text.get(at), Some(b'e' | b'E')) {
        let mut exponent = at + 1;
        if matches!(text.get(exponent), Some(b'+' | b'-')) {
            exponent += 1;
        }
        if text.get(exponent).is_some_and(u8::is_ascii_digit) {
            while text.get(exponent).is_some_and(u8::is_ascii_digit) {
                exponent += 1;
            }
            at = exponent;
        }
    }
    let end = if at > mantissa { at } else { mantissa };
    std::str::from_utf8(&text[start..end])
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0)
}

// IDA: 0051B9D1's frame for a "Bad note string" warning: the time times 30.0 stored as a float, then `fistp` under the default (nearest) rounding.
fn note_frame(time: f32) -> i32 {
    ((f64::from(time) * 30.0) as f32).round_ties_even() as i32
}

// IDA: N/A a C string's bytes from `at`, to its NUL or the buffer's end.
fn c_str(text: &[u8], at: usize) -> &[u8] {
    let rest = text.get(at..).unwrap_or_default();
    let end = rest
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(rest.len());
    &rest[..end]
}

// IDA: 0051B6F1 / 0051B969's skip: from the first '\n' at or after `at`, past every '\r' and '\n'; the next line's start, or None at the text's end (or with no '\n').
fn next_line(text: &[u8], at: usize) -> Option<usize> {
    let line = c_str(text, at);
    let newline = line.iter().position(|&byte| byte == b'\n')?;
    let mut next = at + newline;
    while let Some(&byte) = text.get(next) {
        if byte == 0 {
            return None;
        }
        if byte != b'\r' && byte != b'\n' {
            return Some(next);
        }
        next += 1;
    }
    None
}

/// IDA: 0051B490 TESAnimGroup_CreateFromSequence(sequence, path) (__cdecl), under g_disableWarningMessages = 1 (restored after). First the accum root's priority copied to its "<accum> NonAccum" item (each item's name by 006C66B0, both by _stricmp; only when both are found). Then the file name after the last '\': a movement prefix (00B102B8, `_strnicmp`) and, after it, a weapon prefix (00B102C8). Then each text key (+0x20), line by line: an empty key text warns "AnimGroup empty note key at time %.2f in sequence '%s' in model '%s'."; a line starting '\r' moves past its line ends; "m:" sets +0x20 once (its third byte); "Blend:" +0x21 (atol); "Sound:" / "Enum:" add a note (`AnimGroupNote`; an unknown sound or enum warns "Bad note string"); any other line is matched against the group's next key name (the sequence's name, cut at '_' by 00436DA0, by _stricmp over 00B102E0, the row kept from the first match): a match makes the group on the first key (SpecialIdle's id 2 alone, any other row | the prefixes << 8), stores the key's time (above -1.0) and counts the key, with the loop warnings by the kind ("should be a looping animation" for kind 1 and a non-looping +0x24, "should NOT be ..." for the rest); no row warns "AnimGroup unable to find sequence" and ends the keys. Last, the stage time against the start ("End frame is less than or equal to Start frame"), and "Animation group note problem. See %s file." when anything warned. Answers the new group's entity, or None when no key started one. Completion: 100%; a null line after a '\r' (the CRT's invalid-parameter path) and a sixth key of AttackBow (a read past its row) are stops.
/// # Errors
/// As above.
pub fn create_from_sequence(
    world: &mut World,
    sequence: Entity,
    path: &[u8],
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, StreamError> {
    let saved = *services.disable_warning_messages_b06b18;
    *services.disable_warning_messages_b06b18 = 1;
    let result = create_from_sequence_body(world, sequence, path, services);
    *services.disable_warning_messages_b06b18 = saved;
    let (group, problem) = result?;
    if problem {
        // 0051BDF3: 0040C850 Startup_GetWarningsLogPath, "Warnings.txt" beside the exe.
        (services.warning)(
            b"Animation group note problem. See Warnings.txt file.".to_vec(),
            saved != 0,
        );
    }
    Ok(group.map(|group| world.spawn(group).id()))
}

#[expect(
    clippy::too_many_lines,
    reason = "0051B490 is one function of 0x99B bytes"
)]
fn create_from_sequence_body(
    world: &mut World,
    sequence: Entity,
    path: &[u8],
    services: &mut ModelBuildServices<'_>,
) -> Result<(Option<TesAnimGroup>, bool), StreamError> {
    let mut warn = |text: String| (services.warning)(text.into_bytes(), true);
    let state = world
        .get::<ControllerSequence>(sequence)
        .expect("a NiControllerSequence")
        .clone();
    let file = String::from_utf8_lossy(path).into_owned();
    let sequence_name = state.name_08.clone().unwrap_or_default();
    let sequence_text = String::from_utf8_lossy(&sequence_name).into_owned();
    let mut problem = false;
    // 0051B4DB: sprintf("%s NonAccum") of +0x5C; the CRT prints a null as "(null)".
    let accum = state.accum_root_name_5c.clone();
    let mut non_accum = accum.clone().unwrap_or_else(|| b"(null)".to_vec());
    non_accum.extend_from_slice(b" NonAccum");
    let count = state.array_size_0c as usize;
    let (mut accum_index, mut non_accum_index) = (None, None);
    for index in 0..count {
        let name = interp_item_name(world, sequence, index)?;
        // _stricmp on a null side is the CRT's invalid-parameter path, which answers nonzero.
        if let (Some(accum), Some(name)) = (accum.as_deref(), name.as_deref()) {
            if stricmp(accum, name) == 0 {
                accum_index = Some(index);
            } else if stricmp(&non_accum, name) == 0 {
                non_accum_index = Some(index);
            }
        } else if let Some(name) = name.as_deref()
            && stricmp(&non_accum, name) == 0
        {
            non_accum_index = Some(index);
        }
        if let (Some(from), Some(to)) = (accum_index, non_accum_index) {
            let priority = state
                .items_14
                .get(from)
                .map_or(0xFF, |item| item.priority_0d);
            if to < count {
                world
                    .get_mut::<ControllerSequence>(sequence)
                    .expect("a NiControllerSequence")
                    .items_14[to]
                    .priority_0d = priority;
            }
            break;
        }
    }
    let (mut movement, mut hand) = (0_u16, 0_u16);
    if let Some(slash) = path.iter().rposition(|&byte| byte == b'\\') {
        let mut rest = &path[slash + 1..];
        for (index, prefix) in MOVE_PREFIXES.iter().enumerate().skip(1) {
            if strnicmp(rest, prefix, prefix.len()) == 0 {
                movement = index as u16;
                rest = &rest[prefix.len()..];
                break;
            }
        }
        for (index, prefix) in HAND_PREFIXES.iter().enumerate().skip(1) {
            if strnicmp(rest, prefix, prefix.len()) == 0 {
                hand = index as u16;
                break;
            }
        }
    }
    let keys = state
        .text_keys_20
        .and_then(|keys| world.get::<TextKeyExtraData>(keys))
        .map(|keys| keys.keys.clone())
        .unwrap_or_default();
    let mut group: Option<TesAnimGroup> = None;
    let mut cached_row: Option<usize> = None;
    let mut matched = 0_usize;
    let mut key_index = 0;
    while key_index < keys.len() {
        let key = &keys[key_index];
        let time = f32::from_bits(key.time_bits);
        let Some(text) = key.text.clone() else {
            warn(format!(
                "AnimGroup empty note key at time {time:.2} in sequence '{sequence_text}' in model '{file}'."
            ));
            problem = true;
            key_index += 1;
            continue;
        };
        let mut line = 0_usize;
        'lines: loop {
            if c_str(&text, line).is_empty() {
                break 'lines;
            }
            if text[line] == b'\r' {
                match next_line(&text, line) {
                    Some(next) => line = next,
                    None => {
                        return Err(StreamError(
                            "0051B6FD: a key's text past its last line end (a null _strnicmp)",
                        ));
                    }
                }
            }
            let here = c_str(&text, line);
            if strnicmp(here, b"m:", 2) == 0 {
                let group = group
                    .as_mut()
                    .ok_or(StreamError("0051B732: an \"m:\" note before the group"))?;
                if group.move_byte_20 == 0 {
                    group.move_byte_20 = text.get(line + 2).copied().unwrap_or(0);
                }
            } else if strnicmp(here, b"Blend:", 6) == 0 {
                let group = group
                    .as_mut()
                    .ok_or(StreamError("0051B76B: a \"Blend:\" note before the group"))?;
                let from = if here.get(6) == Some(&b' ') { 7 } else { 6 };
                group.blend_21 = atol(&here[from.min(here.len())..]) as u8;
            } else if strnicmp(here, b"Sound:", 6) == 0 || strnicmp(here, b"Enum:", 5) == 0 {
                let is_enum = strnicmp(here, b"Enum:", 5) == 0;
                if let Some(group) = group.as_mut() {
                    // The line up to its '\r', copied from +6 (Enum:) or +7 (Sound:).
                    let end = here
                        .iter()
                        .position(|&byte| byte == b'\r')
                        .unwrap_or(here.len());
                    let from = if is_enum { 6 } else { 7 };
                    let mut copy = here[from.min(end)..end].to_vec();
                    let mut second: Option<Vec<u8>> = None;
                    let mut third: Option<Vec<u8>> = None;
                    if let Some(split) = copy
                        .iter()
                        .position(|&byte| byte == b',')
                        .or_else(|| copy.iter().position(|&byte| byte == b' '))
                    {
                        let mut rest = copy[split + 1..].to_vec();
                        copy.truncate(split);
                        let spaces = rest.iter().take_while(|&&byte| byte == b' ').count();
                        rest.drain(..spaces);
                        if let Some(split) = rest
                            .iter()
                            .position(|&byte| byte == b',')
                            .or_else(|| rest.iter().position(|&byte| byte == b' '))
                        {
                            third = Some(rest[split + 1..].to_vec());
                            rest.truncate(split);
                        }
                        second = Some(rest);
                    }
                    if is_enum {
                        match ENUM_NAMES.iter().position(|name| stricmp(name, &copy) == 0) {
                            Some(index) => {
                                let at = group.notes_28.len();
                                group.resize_notes(at + 1);
                                group.notes_28[at].time_00 = key.time_bits;
                                group.notes_28[at].value_04 = index as i32;
                                group.notes_28[at].value_08 =
                                    (key_index as u32) | ((line as u32) << 8);
                            }
                            None => {
                                warn(format!(
                                    "Bad note string \"{}\" frame {} in \"{file}\".",
                                    String::from_utf8_lossy(here),
                                    note_frame(time)
                                ));
                                problem = true;
                            }
                        }
                    } else {
                        // 00447490: g_formsByEditorID, a TESSound (type 10) only.
                        let sound = (!copy.is_empty())
                            .then(|| {
                                world
                                    .resource::<super::data_handler::EditorIdMap>()
                                    .lookup(&copy)
                            })
                            .flatten()
                            .filter(|&form| super::forms_init::form_type(world, form) == 10);
                        match sound {
                            Some(sound) => {
                                let at = group.notes_28.len();
                                group.resize_notes(at + 1);
                                group.notes_28[at].time_00 = key.time_bits;
                                group.notes_28[at].sound_0c = Some(sound);
                                if let Some(second) = &second {
                                    // 0051BD00: the float times 250.0, truncated (0x0C00), and only its low byte stored.
                                    let volume =
                                        (f64::from(c_atof(second) as f32) * 250.0).trunc() as i32;
                                    let note = &mut group.notes_28[at];
                                    note.value_04 = (note.value_04 & !0xFF) | (volume & 0xFF);
                                }
                                if let Some(third) = &third {
                                    group.notes_28[at].value_08 = (c_atof(third) as f32).to_bits();
                                }
                            }
                            None => {
                                warn(format!(
                                    "Bad note string \"{}\" frame {} in \"{file}\".",
                                    String::from_utf8_lossy(here),
                                    note_frame(time)
                                ));
                                problem = true;
                            }
                        }
                    }
                }
            } else {
                // 0051B9A8: the group's row, from the cached one or by _stricmp from row 0.
                let found = match cached_row {
                    Some(row) => {
                        (stricmp(&sequence_name, ANIM_GROUPS[row].name) == 0).then_some(row)
                    }
                    None => ANIM_GROUPS
                        .iter()
                        .position(|info| stricmp(&sequence_name, info.name) == 0),
                };
                if found.is_none() && cached_row.is_none() {
                    warn(format!(
                        "AnimGroup unable to find sequence '{sequence_text}' in model '{file}'."
                    ));
                    problem = true;
                    key_index = keys.len();
                    break 'lines;
                }
                if let Some(row) = found {
                    cached_row = Some(row);
                    let info = ANIM_GROUPS[row];
                    if matched >= 5 {
                        return Err(StreamError(
                            "0051B9F2: a sixth key read past its row (00B102F0)",
                        ));
                    }
                    let key_name_index = info.keys_10[matched];
                    if key_name_index != -1 {
                        let name = KEY_NAMES[key_name_index as usize][info.kind_0c as usize];
                        if strnicmp(here, name, name.len()) == 0 {
                            let group = group.get_or_insert_with(|| {
                                TesAnimGroup::new(if row == 2 {
                                    2
                                } else {
                                    row as u16 + ((hand + 16 * movement) << 8)
                                })
                            });
                            if time > -1.0
                                && let Some(slot) = group.times_10.get_mut(key_name_index as usize)
                            {
                                *slot = time;
                            }
                            if row != 2 {
                                let looping = state.cycle_type_24 == 0;
                                if info.kind_0c == 1 && !looping {
                                    warn(format!("'{file}' should be a looping animation."));
                                    problem = true;
                                } else if info.kind_0c != 1 && looping {
                                    warn(format!("'{file}' should NOT be a looping animation."));
                                    problem = true;
                                }
                            }
                            matched += 1;
                        }
                    }
                }
            }
            match next_line(&text, line) {
                Some(next) => line = next,
                None => break 'lines,
            }
        }
        key_index += 1;
    }
    if let Some(group) = &group {
        let stage = match ANIM_GROUPS[usize::from(group.group_type())].kind_0c {
            2 | 3 | 5 => 2,
            4 => 3,
            7 => 4,
            _ => 1,
        };
        let end = action_time(group, stage, &mut |text| (services.warning)(text, true));
        let start = action_time(group, 0, &mut |text| (services.warning)(text, true));
        if start >= end {
            let name = String::from_utf8_lossy(ANIM_GROUPS[usize::from(group.group_type())].name)
                .into_owned();
            (services.warning)(
                format!("{name}: End frame is less than or equal to Start frame in \"{file}\".")
                    .into_bytes(),
                true,
            );
            problem = true;
        }
    }
    Ok((group, problem))
}
