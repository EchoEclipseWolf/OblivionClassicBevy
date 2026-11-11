//! The setting collections the CRT initializers fill: INI, registry and BlendSettings.

pub mod blend_settings;
pub mod game_settings;
pub mod game_settings_table;
pub mod paths;
pub mod settings_table;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use bevy_ecs::prelude::*;
use oblivion_core::crt::{format_f, scan_f32, scan_u32_list, stricmp};
use oblivion_core::logging::{Level, emit};
use oblivion_core::profile;

use super::settings_table::STATIC_SETTINGS;
use crate::bs::archive::ArchiveSettings;

/// IDA: 004A78D0 the setting type from the first name character: anything else, or no name, is 9.
/// Completion: 100%.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SettingType {
    Boolean = 0,
    Character = 1,
    Hex = 2,
    Integer = 3,
    UnsignedInteger = 4,
    Floating = 5,
    String = 6,
    Rgb = 7,
    Rgba = 8,
    Unknown = 9,
}

impl SettingType {
    /// IDA: N/A the enum of 004A78D0's answer (`game_settings::setting_type`); anything outside 0..=8 is 9.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        match raw {
            0 => Self::Boolean,
            1 => Self::Character,
            2 => Self::Hex,
            3 => Self::Integer,
            4 => Self::UnsignedInteger,
            5 => Self::Floating,
            6 => Self::String,
            7 => Self::Rgb,
            8 => Self::Rgba,
            _ => Self::Unknown,
        }
    }
}

/// IDA: SettingInfo {u32 value +0; char* name +4}. Bool and char settings store into byte 0 only.
/// `original_address` is the static object's VA, the setting's identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingInfo {
    pub original_address: u32,
    pub name: String,
    pub raw: u32,
    pub text: Option<Vec<u8>>,
}

impl SettingInfo {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn new(original_address: u32, name: &str, raw: u32, text: Option<&[u8]>) -> Self {
        Self {
            original_address,
            name: name.to_owned(),
            raw,
            text: text.map(<[u8]>::to_vec),
        }
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn as_bool(&self) -> bool {
        self.raw & 0xFF != 0
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn as_int(&self) -> i32 {
        self.raw as i32
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn as_float(&self) -> f32 {
        f32::from_bits(self.raw)
    }

    /// IDA: N/A the SettingInfo view of 004A7980 (`game_settings::setting_info_set_string`): the block's key goes back
    /// to `name`.
    fn set_string(&mut self, value: Option<Vec<u8>>) {
        let key = self.name.as_bytes().to_vec();
        if let Some(key) = game_settings::setting_info_set_string(
            &mut self.raw,
            &mut self.text,
            Some(&key),
            value.as_deref(),
        ) {
            self.name = String::from_utf8(key).expect("004A7980: an ASCII setting name");
        }
    }
}

/// IDA: the three static collections: INISettingCollection 00B07BF0, RegSettingCollection 00B07D28
/// and the BlendSettings collection 00B11C40.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingCollectionKind {
    Ini,
    Registry,
    Blend,
}

/// IDA: one static SettingInfo registered by a CRT initializer.
#[derive(Clone, Copy, Debug)]
pub struct StaticSettingDefinition {
    pub address: u32,
    pub collection: SettingCollectionKind,
    pub name: &'static str,
    pub raw: u32,
    pub text: Option<&'static str>,
}

/// IDA: N/A a helper, with no address of its own.
#[must_use]
pub const fn static_setting_definitions() -> &'static [StaticSettingDefinition] {
    STATIC_SETTINGS
}

// IDA: N/A a helper, with no address of its own.
fn setting_section(name: &str) -> &str {
    name.split_once(':').map_or("MAIN", |(_, section)| section)
}

// IDA: N/A a helper, with no address of its own.
fn setting_key(name: &str) -> &str {
    name.split_once(':').map_or(name, |(key, _)| key)
}

/// IDA: SettingCollectionList: +0x04 path, +0x108 handle, +0x10C BSSimpleList<SettingInfo*>. The
/// list holds settings head first (reverse registration order).
#[derive(Debug, Default)]
pub struct SettingCollection {
    path: PathBuf,
    open: bool,
    settings: VecDeque<SettingInfo>,
    rejected: Vec<String>,
}

impl SettingCollection {
    /// IDA: 004040E0 AddSetting: a name already present (`_stricmp`) logs and is not added;
    /// otherwise PrependNonNull puts it at the head. Completion: 100%.
    pub fn add(&mut self, setting: SettingInfo) -> bool {
        let taken = self
            .settings
            .iter()
            .any(|existing| stricmp(existing.name.as_bytes(), setting.name.as_bytes()) == 0);
        if taken {
            emit(
                Level::Warning,
                &format!(
                    "Setting key '{}' already used in list. Setting keys must be unique.",
                    setting.name
                ),
            );
            self.rejected.push(setting.name);
            return false;
        }
        self.settings.push_front(setting);
        true
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn settings(&self) -> &VecDeque<SettingInfo> {
        &self.settings
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn rejected_duplicates(&self) -> &[String] {
        &self.rejected
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// IDA: +0x04 path copy (strcpy at 0040E6C6 / 0040E717).
    pub fn set_path(&mut self, path: impl Into<PathBuf>) {
        self.path = path.into();
    }

    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn find_address(&self, address: u32) -> Option<&SettingInfo> {
        self.settings
            .iter()
            .find(|setting| setting.original_address == address)
    }

    /// IDA: N/A the static SettingInfo at `address`, for the original's in-place writes.
    pub fn find_address_mut(&mut self, address: u32) -> Option<&mut SettingInfo> {
        self.settings
            .iter_mut()
            .find(|setting| setting.original_address == address)
    }

    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn find_name(&self, name: &str) -> Option<&SettingInfo> {
        self.settings
            .iter()
            .find(|setting| stricmp(setting.name.as_bytes(), name.as_bytes()) == 0)
    }

    /// IDA: 004A83B0 BeginStoreAccess (+0x108 = this; returns true). Completion: 100%.
    pub fn open_ini(&mut self) -> bool {
        self.open = true;
        true
    }

    /// IDA: 004A7CB0 EndStoreAccess (+0x108 = 0; returns true). Completion: 100%.
    pub fn close(&mut self) -> bool {
        self.open = false;
        true
    }

    /// IDA: 004A85A0 ReadAll: false without an open handle; otherwise every item from the head is
    /// read and the results ANDed (any non-float/rgb/rgba read returns false). Completion: 100%.
    pub fn read_all(&mut self) -> bool {
        if !self.open {
            return false;
        }
        let path = self.path.clone();
        let mut result = true;
        for setting in &mut self.settings {
            result = read_ini_setting(&path, setting) && result;
        }
        result
    }

    /// IDA: 004A8800, as a call outside ReadAll would make it.
    pub fn read_setting(&self, setting: &mut SettingInfo) -> bool {
        read_ini_setting(&self.path, setting)
    }
}

/// IDA: 004A8800 INISettingCollection read-one, per type. Defaults are the current values.
/// Completion: 98%: components a partial rgb/rgba sscanf leaves unwritten are 0 here (stack garbage
/// in the original).
fn read_ini_setting(path: &Path, setting: &mut SettingInfo) -> bool {
    const PROFILE_BUFFER: usize = 0x100;
    if setting.name.is_empty() {
        return false;
    }
    let section = setting_section(&setting.name).as_bytes().to_vec();
    let key = setting_key(&setting.name).as_bytes().to_vec();
    let profile_int = |fallback: i32| profile::read_int(path, &section, &key, fallback);
    let profile_string = |fallback: Option<&[u8]>| {
        profile::read_string(path, &section, &key, fallback, PROFILE_BUFFER)
    };
    let mut result = false;
    match SettingType::from_raw(game_settings::setting_type(Some(setting.name.as_bytes()))) {
        SettingType::Boolean => {
            // 004A8A6E: default = byte 0 != 0; store (result != 0) into byte 0.
            let value = profile_int(i32::from(setting.raw & 0xFF != 0));
            setting.raw = (setting.raw & !0xFF) | u32::from(value != 0);
        }
        SettingType::Character => {
            // 004A88C1: default = the full dword; store the low byte.
            let value = profile_int(setting.raw as i32);
            setting.raw = (setting.raw & !0xFF) | (value & 0xFF);
        }
        SettingType::Floating => {
            // 004A8903: sprintf "%f" default (6 decimals), then sscanf "%f" of a non-empty result:
            // a missing key re-parses the rounded default.
            let fallback = format_f(f64::from(setting.as_float()));
            let value = profile_string(Some(fallback.as_bytes()));
            if !value.is_empty()
                && let Some((parsed, _)) = scan_f32(&value)
            {
                setting.raw = parsed.to_bits();
                result = true;
            }
        }
        SettingType::String => {
            // 004A8889: default = current text; nSize 0x100.
            let value = profile_string(setting.text.as_deref());
            setting.set_string(Some(value));
        }
        SettingType::Rgb => {
            // 004A8979: default ""; packed even after a partial parse.
            let value = profile_string(Some(b""));
            if !value.is_empty() {
                let mut parts = [0u32; 3];
                let fields = scan_u32_list(&value, &mut parts);
                setting.raw = (((((parts[1] & 0xFF) | (parts[0] << 8)) << 8) | (parts[2] & 0xFF))
                    << 8)
                    | 0xFF;
                result = fields == 3;
            }
        }
        SettingType::Rgba => {
            // 004A89F6 -> 004A83C0(a1, u8 a2, u8 a3, u8 a4).
            let value = profile_string(Some(b""));
            if !value.is_empty() {
                let mut parts = [0u32; 4];
                let fields = scan_u32_list(&value, &mut parts);
                setting.raw = (parts[3] & 0xFF)
                    | (((parts[2] & 0xFF) | ((parts[0] << 8) | (parts[1] & 0xFF)) << 8) << 8);
                result = fields == 4;
            }
        }
        SettingType::Integer
        | SettingType::Hex
        | SettingType::UnsignedInteger
        | SettingType::Unknown => {
            // 004A8A9C and the integer case: dword default and store.
            setting.raw = profile_int(setting.raw as i32);
        }
    }
    result
}

/// IDA: 00B07D28 / 00B07BF0 / 00B11C40 after the CRT initializers ran.
///
/// Recorded host substitution: Linux has no registry, so the registry collection's open (004A8C90)
/// always fails, as it does on a machine without the key. `sBasePath` keeps its default ".\".
#[derive(Resource, Debug, Default)]
pub struct SettingCollections {
    pub registry: SettingCollection,
    pub ini: SettingCollection,
    pub blend: SettingCollection,
}

/// IDA: CRT initializers in table order calling 004040E0. Completion: 100%.
pub fn register_static_settings(collections: &mut SettingCollections) {
    for definition in STATIC_SETTINGS {
        let setting = SettingInfo::new(
            definition.address,
            definition.name,
            definition.raw,
            definition.text.map(str::as_bytes),
        );
        let collection = match definition.collection {
            SettingCollectionKind::Registry => &mut collections.registry,
            SettingCollectionKind::Ini => &mut collections.ini,
            SettingCollectionKind::Blend => &mut collections.blend,
        };
        collection.add(setting);
    }
}

/// IDA: B04428..B04460 the [Archive] settings, read from the INI collection. Completion: 100%.
#[must_use]
pub fn archive_settings_from(ini: &SettingCollection) -> ArchiveSettings {
    let mut settings = ArchiveSettings::default();
    if let Some(value) = ini.find_address(0x00B0_4428) {
        settings.use_archives = value.as_bool();
    }
    if let Some(value) = ini.find_address(0x00B0_4430) {
        settings.check_runtime_collisions = value.as_bool();
    }
    if let Some(value) = ini.find_address(0x00B0_4438) {
        settings.retain_directory_strings = value.as_int();
    }
    if let Some(value) = ini.find_address(0x00B0_4440) {
        settings.retain_filename_strings = value.as_int();
    }
    if let Some(value) = ini.find_address(0x00B0_4448) {
        settings.retain_filename_offsets = value.as_int();
    }
    if let Some(value) = ini.find_address(0x00B0_4450) {
        settings.invalidation_file = value.text.clone().unwrap_or_default();
    }
    if let Some(value) = ini.find_address(0x00B0_4458) {
        settings.archive_list = value.text.clone().unwrap_or_default();
    }
    if let Some(value) = ini.find_address(0x00B0_4460) {
        settings.invalidate_older_files = value.as_bool();
    }
    settings
}
