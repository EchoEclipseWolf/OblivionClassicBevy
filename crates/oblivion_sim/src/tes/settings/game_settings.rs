//! Phase 07 (D7): the GameSettingCollection 00B35468 the CRT initializers fill before WinMain: 2,304 settings (992
//! FloatSetting_ctor 0041BAE0, 1,312 SettingValue32_ctor 00419A70) registered in the BSTCaseInsensitiveStringMap at +0x10C
//! (00B35574, 0x191 buckets). Read-only until the Phase 12 GMST load.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;

use oblivion_core::error::GameError;

use super::game_settings_table::GAME_SETTING_DEFINITIONS;
use crate::tes::settings::SettingCollection;
use crate::tes::tes_file::{RecordReader, TesFileId, c_bytes};

// IDA: the fourcc 004A7D8D compares.
const DATA_FOURCC: u32 = 0x4154_4144;

/// IDA: 00B399D0's cinit, 009F6040, is slot 2206.
const CONTROL_SETTINGS_SLOT: u32 = 2206;
/// IDA: 00B399D0 the control-name settings, sUActnForward..sUActnGrab.
pub const CONTROL_SETTING_COUNT: usize = 29;

/// IDA: one CRT registration (generated table, cinit order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameSettingDefinition {
    /// The cinit slot index (00A283D0 + 4 * slot).
    pub slot: u32,
    /// The constructor call.
    pub call_site: u32,
    /// The static Setting; 0 is FormHeap-allocated.
    pub address: u32,
    /// `f` float, `i` integer, `s` string pointer.
    pub kind: u8,
    /// +4 key.
    pub name: Option<&'static str>,
    /// +0 raw value (float bits or integer).
    pub raw: u32,
    /// +0 string value for `s`.
    pub text: Option<&'static [u8]>,
}

/// IDA: Setting (8 bytes: +0 value, +4 key) as registered. `heap_name` is +4 once 004A7980 has moved the key to its block; `loads` is a host count of 0044E356's loads. Completion: 100%.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameSetting {
    pub definition: &'static GameSettingDefinition,
    pub raw: u32,
    pub text: Option<Vec<u8>>,
    pub heap_name: Option<Vec<u8>>,
    pub loads: u32,
}

impl GameSetting {
    // IDA: +4, the key.
    fn key(&self) -> Option<&[u8]> {
        self.heap_name
            .as_deref()
            .or_else(|| self.definition.name.map(str::as_bytes))
    }

    /// IDA: N/A the GameSetting view of 004A7980 (`setting_info_set_string`): the block's key goes to `heap_name`.
    pub fn set_string(&mut self, value: &[u8]) {
        let key = self.key().map(<[u8]>::to_vec);
        if let Some(key) =
            setting_info_set_string(&mut self.raw, &mut self.text, key.as_deref(), Some(value))
        {
            self.heap_name = Some(key);
        }
    }
}

/// IDA: 004A7980 SettingInfo_SetString(value): one FormHeap block holds {key copy, value}; +0 points at the value; with a key (a null key skips this), the block's first byte becomes 'S', a key starting 'S' is freed (its old block), and +4 points at the block. A null value clears +0 and leaves the key. Completion: 100%, with `text` as the value and the answer as the block's key (the caller stores it at +4).
pub fn setting_info_set_string(
    raw: &mut u32,
    text: &mut Option<Vec<u8>>,
    key: Option<&[u8]>,
    value: Option<&[u8]>,
) -> Option<Vec<u8>> {
    let Some(value) = value else {
        *raw = 0;
        *text = None;
        return None;
    };
    *text = Some(value.to_vec());
    let mut key = key?.to_vec();
    // 004A7A11 `mov byte [esi], 'S'`; an empty key's block holds only its NUL there.
    if let Some(first) = key.first_mut() {
        *first = b'S';
    } else {
        key.push(b'S');
    }
    Some(key)
}

/// IDA: 004A78D0 SettingInfo_GetType(key): 9 for a null key; by the first byte, through bytes 004A7954 over 004A792C, read from the exe: 'S' and 's' 6, 'a' 8, 'b' 0, 'c' 1, 'f' 5, 'h' 2, 'i' 3, 'r' 7, 'u' 4, anything else 9. Completion: 100%.
#[must_use]
pub fn setting_type(key: Option<&[u8]>) -> u32 {
    match key.and_then(<[u8]>::first).copied().unwrap_or(0) {
        b'S' | b's' => 6,
        b'a' => 8,
        b'b' => 0,
        b'c' => 1,
        b'f' => 5,
        b'h' => 2,
        b'i' => 3,
        b'r' => 7,
        b'u' => 4,
        _ => 9,
    }
}

/// IDA: N/A the _stricmp key (ASCII case folding).
fn fold(key: &str) -> Vec<u8> {
    key.bytes().map(|byte| byte.to_ascii_lowercase()).collect()
}

/// IDA: 00B35468 GameSettingCollection with its +0x10C map. 100% registration order and duplicate rule. The host index is a
/// sorted map on the folded key; nothing the port reads sees the original's 0x191 bucket order.
/// Completion: 100%.
#[derive(Debug, Clone, Resource)]
pub struct GameSettingCollection {
    /// +0x04, 004A805A's byte store.
    pub byte_04: u8,
    /// +0x108, the file 004A8040 holds while it loads.
    pub store_file_108: Option<TesFileId>,
    controls: [Option<&'static GameSettingDefinition>; CONTROL_SETTING_COUNT],
    settings: Vec<GameSetting>,
    index: BTreeMap<Vec<u8>, usize>,
    duplicate_warnings: Vec<String>,
}

impl Default for GameSettingCollection {
    /// IDA: 00B35468 after the CRT initializers. 100%.
    fn default() -> Self {
        Self::new(GAME_SETTING_DEFINITIONS)
    }
}

impl GameSettingCollection {
    /// IDA: 00419A70 and 0041BAE0 per definition, in order: NiTPointerMap_Lookup on the key; a hit warns "Setting key '%s'
    /// already used in map." and keeps the first; otherwise InsertOrAssign. A null key skips both. 100%.
    pub fn new(definitions: &'static [GameSettingDefinition]) -> Self {
        let mut collection = Self {
            byte_04: 0,
            store_file_108: None,
            controls: [None; CONTROL_SETTING_COUNT],
            settings: Vec::with_capacity(definitions.len()),
            index: BTreeMap::new(),
            duplicate_warnings: Vec::new(),
        };
        let mut control = 0;
        for definition in definitions {
            // 009F609A.. the B399D0 stores (the setting object, registered or not).
            if definition.slot == CONTROL_SETTINGS_SLOT && control < CONTROL_SETTING_COUNT {
                collection.controls[control] = Some(definition);
                control += 1;
            }
            // 00419AA5 / 0041BB15: a null key registers nothing.
            let Some(name) = definition.name else {
                continue;
            };
            let key = fold(name);
            if collection.index.contains_key(&key) {
                // 00419AC9 DebugMessage_Warning; the first setting stays mapped.
                collection
                    .duplicate_warnings
                    .push(format!("Setting key '{name}' already used in map.\n"));
                continue;
            }
            collection.index.insert(key, collection.settings.len());
            collection.settings.push(GameSetting {
                definition,
                raw: definition.raw,
                text: definition.text.map(<[u8]>::to_vec),
                heap_name: None,
                loads: 0,
            });
        }
        collection
    }

    /// IDA: NiTPointerMap_Lookup (BSTCaseInsensitiveStringMap: _stricmp keys). 100%.
    pub fn find(&self, key: &str) -> Option<&GameSetting> {
        self.index
            .get(&fold(key))
            .map(|&index| &self.settings[index])
    }

    /// IDA: the same lookup, for the Phase 12 GMST loader. 100%.
    pub fn find_mut(&mut self, key: &str) -> Option<&mut GameSetting> {
        self.index
            .get(&fold(key))
            .map(|&index| &mut self.settings[index])
    }

    /// IDA: 004A8040 vt+0x24 LoadSetting(file, key): +0x04 = 0, +0x108 = file, NiTPointerMap_Lookup(+0x10C, key); a hit runs vt+0x10 (below), whose byte is the result; then vt+0x18 (004A7CB0: +0x108 = 0). The key is the record's bytes: the map's keys are ASCII, and `_stricmp` folds only ASCII in the C locale, so the lowered lookup gives the original's hit.
    /// Completion: 100%.
    pub fn load_setting(
        &mut self,
        key: &[u8],
        file: &mut RecordReader<'_, '_>,
    ) -> Result<bool, GameError> {
        self.byte_04 = 0;
        self.store_file_108 = Some(file.id);
        let mut result = false;
        if let Some(&index) = self.index.get(&key.to_ascii_lowercase()) {
            result = load_setting_value(&mut self.settings[index], file)?;
        }
        self.store_file_108 = None;
        Ok(result)
    }

    /// IDA: a read of a static Setting's +0 value by address (e.g. 004A9A00 dword_B35464+0x12C): the registered setting's
    /// current raw value, else the definition's, since a duplicate key keeps its image value. 100%.
    pub fn raw_at(&self, address: u32) -> Option<u32> {
        if address == 0 {
            return None;
        }
        self.settings
            .iter()
            .find(|setting| setting.definition.address == address)
            .map(|setting| setting.raw)
            .or_else(|| {
                GAME_SETTING_DEFINITIONS
                    .iter()
                    .find(|definition| definition.address == address)
                    .map(|definition| definition.raw)
            })
    }

    /// IDA: N/A `raw_at` as a float's bits, as `fld d [address]` reads it.
    #[must_use]
    pub fn float_at(&self, address: u32) -> Option<f32> {
        self.raw_at(address).map(f32::from_bits)
    }

    /// IDA: N/A `raw_at` as a signed dword.
    #[must_use]
    pub fn int_at(&self, address: u32) -> Option<i32> {
        self.raw_at(address).map(|raw| raw as i32)
    }

    /// IDA: N/A `find`'s +0 value as a float's bits (SettingInfo_FloatValueOrZero's read, without the zero).
    #[must_use]
    pub fn float_named(&self, name: &str) -> Option<f32> {
        self.find(name).map(|setting| f32::from_bits(setting.raw))
    }

    /// IDA: N/A `find`'s +0 value as a signed dword.
    #[must_use]
    pub fn int_named(&self, name: &str) -> Option<i32> {
        self.find(name).map(|setting| setting.raw as i32)
    }

    /// IDA: a store through a static Setting's address (e.g. 004D568D 00B35C14). 100%.
    pub fn setting_at_mut(&mut self, address: u32) -> Option<&mut GameSetting> {
        if address == 0 {
            return None;
        }
        self.settings
            .iter_mut()
            .find(|setting| setting.definition.address == address)
    }

    /// IDA: the same read of a static string Setting: the char* +0 value, else the definition's text. `None` is no static
    /// string Setting there or a null char* value. 100%.
    pub fn text_at(&self, address: u32) -> Option<Vec<u8>> {
        if address == 0 {
            return None;
        }
        if let Some(setting) = self.settings.iter().find(|setting| {
            setting.definition.address == address && setting.definition.kind == b's'
        }) {
            return setting.text.clone();
        }
        GAME_SETTING_DEFINITIONS
            .iter()
            .find(|definition| definition.address == address && definition.kind == b's')
            .and_then(|definition| definition.text)
            .map(<[u8]>::to_vec)
    }

    /// IDA: N/A an accessor.
    pub fn len(&self) -> usize {
        self.settings.len()
    }

    /// IDA: N/A an accessor.
    pub fn is_empty(&self) -> bool {
        self.settings.is_empty()
    }

    /// IDA: N/A an accessor.
    pub fn duplicate_warnings(&self) -> &[String] {
        &self.duplicate_warnings
    }

    /// IDA: 00B399D0 the 29 settings the cinit 009F6040 (slot 2206) allocates and stores in order.
    pub fn control_settings(
        &self,
    ) -> &[Option<&'static GameSettingDefinition>; CONTROL_SETTING_COUNT] {
        &self.controls
    }
}

/// IDA: N/A a static float GameSetting by address, as the callers that answer 0.0 read it: zero bits with no static setting
/// there. The collection must exist (it panics without one, as those callers did).
pub(crate) fn float_at_or_zero(world: &World, address: u32) -> f32 {
    f32::from_bits(
        world
            .resource::<GameSettingCollection>()
            .raw_at(address)
            .unwrap_or(0),
    )
}

/// IDA: N/A as `float_at_or_zero`, a signed dword: 0 with no static setting there.
pub(crate) fn int_at_or_zero(world: &World, address: u32) -> i32 {
    world
        .resource::<GameSettingCollection>()
        .raw_at(address)
        .unwrap_or(0) as i32
}

/// IDA: N/A a static string GameSetting's text by address, as the menus' prompts and captions read it (the exe's operands
/// are the Setting's address; the Phase 12 load may have replaced the text): empty with no collection, no static string
/// Setting there or a null value.
pub(crate) fn text_at_or_empty(world: &World, address: u32) -> Vec<u8> {
    world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.text_at(address))
        .unwrap_or_default()
}

/// IDA: N/A a float GameSetting by name (SettingInfo_FloatValueOrZero): 0.0 with no collection or no such setting.
pub(crate) fn float_named_or_zero(world: &World, name: &str) -> f32 {
    world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.float_named(name))
        .unwrap_or(0.0)
}

// IDA: 004A7D60 vt+0x10 LoadSettingValue(setting): 0044FEA0 (result ignored); a chunk other than DATA returns 0. By 004A78D0 of the key: 3 and 5 read 4 bytes into +0 (004510E0); 6 reads the chunk whole (alloca, size 0), runs 004A7980, and returns the read's result; others return 0. The +0x108 null test is `load_setting`'s, which always sets it. Completion: 100%.
fn load_setting_value(
    setting: &mut GameSetting,
    file: &mut RecordReader<'_, '_>,
) -> Result<bool, GameError> {
    file.advance_chunk()?;
    if file.chunk_type()? != DATA_FOURCC {
        return Ok(false);
    }
    match setting_type(setting.key()) {
        3 | 5 => {
            let (raw, read) = file.read_dword(setting.raw)?;
            setting.raw = raw;
            setting.loads += 1;
            Ok(read)
        }
        6 => {
            let length = file.chunk_size()? as usize;
            let mut buffer = vec![0_u8; length + 1];
            let read = file.read_chunk(&mut buffer, 0)?;
            setting.set_string(c_bytes(&buffer));
            setting.loads += 1;
            Ok(read)
        }
        _ => Ok(false),
    }
}

/// IDA: 00404E30/00403C00 value-or-zero getters: a null SettingInfo returns 0.0 and stores 0.0 into 00B35464; the
/// settings here always exist. Completion: 100%.
pub(crate) fn setting_float(ini: &SettingCollection, address: u32) -> f32 {
    ini.find_address(address)
        .map_or(0.0, |setting| setting.as_float())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The C++ phase07 verify_game_settings, except its font token check, which ports with fonts.
    #[test]
    fn counts_lookup_and_control_table() {
        assert_eq!(GAME_SETTING_DEFINITIONS.len(), 2304);
        assert_eq!(
            GAME_SETTING_DEFINITIONS
                .iter()
                .filter(|d| d.kind == b'f')
                .count(),
            992
        );
        let collection = GameSettingCollection::default();
        assert_eq!(collection.len(), 2304);
        assert!(collection.duplicate_warnings().is_empty());
        let setting = collection.find("SMAGICEFFECTITEMFOR").unwrap();
        assert_eq!(setting.text.as_deref(), Some(&b"for"[..]));
        let controls = collection.control_settings();
        assert_eq!(controls[0].unwrap().name, Some("sUActnForward"));
        assert_eq!(controls[28].unwrap().name, Some("sUActnGrab"));
    }

    #[test]
    fn reads_by_address() {
        let collection = GameSettingCollection::default();
        assert_eq!(collection.raw_at(0x00B3B7C8), Some(0x43FA0000));
        assert_eq!(collection.text_at(0x00B33460), Some(b"for".to_vec()));
        assert_eq!(collection.text_at(0x00B3B7C8), None);
        assert_eq!(collection.raw_at(0), None);
        assert_eq!(collection.raw_at(4), None);
    }

    #[test]
    fn a_duplicate_key_warns_and_keeps_the_first() {
        static DEFINITIONS: [GameSettingDefinition; 3] = [
            GameSettingDefinition {
                slot: 1,
                call_site: 1,
                address: 8,
                kind: b'i',
                name: Some("iA"),
                raw: 1,
                text: None,
            },
            GameSettingDefinition {
                slot: 2,
                call_site: 2,
                address: 16,
                kind: b'i',
                name: Some("IA"),
                raw: 2,
                text: None,
            },
            GameSettingDefinition {
                slot: 3,
                call_site: 3,
                address: 24,
                kind: b'i',
                name: None,
                raw: 3,
                text: None,
            },
        ];
        let collection = GameSettingCollection::new(&DEFINITIONS);
        assert_eq!(collection.len(), 1);
        assert_eq!(collection.find("ia").unwrap().raw, 1);
        assert_eq!(
            collection.duplicate_warnings(),
            ["Setting key 'IA' already used in map.\n"]
        );
    }
}
