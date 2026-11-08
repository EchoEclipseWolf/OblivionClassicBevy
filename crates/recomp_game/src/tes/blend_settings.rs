//! BlendSettings: the version gate, the collection load and the three transfers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bevy_ecs::prelude::*;
use recomp_core::crt::scan_f32_pair;
use recomp_core::profile;

use super::settings::{SettingCollection, SettingCollections};

struct BlendTransfer {
    setting: u32,
    first: u32,
    second: u32,
}

// IDA: N/A a helper, with no address of its own.
const fn pair(setting: u32, first: u32, second: u32) -> BlendTransfer {
    BlendTransfer {
        setting,
        first,
        second,
    }
}

/// IDA: the sscanf sites of 0053A1B0 (HIT), 0053A460 (QUADHIT) and 0053A720 (DEFAULT) in call order.
const TRANSFERS: [BlendTransfer; 44] = [
    // 0053A1B0
    pair(0x00B11ABC, 0x00B2EC70, 0x00B2EC74), // sHead:HIT
    pair(0x00B11AB4, 0x00B2EC78, 0x00B2EC7C), // sBody:HIT
    pair(0x00B11AAC, 0x00B2EC80, 0x00B2EC84), // sSpine1:HIT
    pair(0x00B11AA4, 0x00B2EC88, 0x00B2EC8C), // sSpine2:HIT
    pair(0x00B11A9C, 0x00B2EC90, 0x00B2EC94), // sLUpperArm:HIT
    pair(0x00B11A94, 0x00B2EC98, 0x00B2EC9C), // sLForeArm:HIT
    pair(0x00B11A8C, 0x00B2ECA0, 0x00B2ECA4), // sLHand:HIT
    pair(0x00B11A84, 0x00B2ECC0, 0x00B2ECC4), // sRUpperArm:HIT
    pair(0x00B11A7C, 0x00B2ECC8, 0x00B2ECCC), // sRForeArm:HIT
    pair(0x00B11A74, 0x00B2ECD0, 0x00B2ECD4), // sRHand:HIT
    // 0053A460
    pair(0x00B11B2C, 0x00B2ED70, 0x00B2ED74), // sHead:QUADHIT
    pair(0x00B11B24, 0x00B2ED78, 0x00B2ED7C), // sBody:QUADHIT
    pair(0x00B11B1C, 0x00B2ED80, 0x00B2ED84), // sSpine1:QUADHIT
    pair(0x00B11B14, 0x00B2ED88, 0x00B2ED8C), // sSpine2:QUADHIT
    pair(0x00B11B0C, 0x00B2ED90, 0x00B2ED94), // sLUpperArm:QUADHIT
    pair(0x00B11B04, 0x00B2ED98, 0x00B2ED9C), // sLForeArm:QUADHIT
    pair(0x00B11AFC, 0x00B2EDA8, 0x00B2EDAC), // sLThigh:QUADHIT
    pair(0x00B11AF4, 0x00B2EDB0, 0x00B2EDB4), // sLCalf:QUADHIT
    pair(0x00B11AEC, 0x00B2EDC0, 0x00B2EDC4), // sRUpperArm:QUADHIT
    pair(0x00B11AE4, 0x00B2EDC8, 0x00B2EDCC), // sRForeArm:QUADHIT
    pair(0x00B11ADC, 0x00B2EDD8, 0x00B2EDDC), // sRThigh:QUADHIT
    pair(0x00B11AD4, 0x00B2EDE0, 0x00B2EDE4), // sRCalf:QUADHIT
    // 0053A720
    pair(0x00B11B34, 0x00B2E668, 0x00B2E66C), // sHead:DEFAULT
    pair(0x00B11B3C, 0x00B2E670, 0x00B2E674), // sBody:DEFAULT
    pair(0x00B11B44, 0x00B2E678, 0x00B2E67C), // sSpine1:DEFAULT
    pair(0x00B11B4C, 0x00B2E680, 0x00B2E684), // sSpine2:DEFAULT
    pair(0x00B11B54, 0x00B2E688, 0x00B2E68C), // sLUpperArm:DEFAULT
    pair(0x00B11B5C, 0x00B2E690, 0x00B2E694), // sLForeArm:DEFAULT
    pair(0x00B11B64, 0x00B2E698, 0x00B2E69C), // sLHand:DEFAULT
    pair(0x00B11B6C, 0x00B2E6A0, 0x00B2E6A4), // sLThigh:DEFAULT
    pair(0x00B11B74, 0x00B2E6A8, 0x00B2E6AC), // sLCalf:DEFAULT
    pair(0x00B11B7C, 0x00B2E6B0, 0x00B2E6B4), // sLFoot:DEFAULT
    pair(0x00B11B84, 0x00B2E6B8, 0x00B2E6BC), // sRUpperArm:DEFAULT
    pair(0x00B11B8C, 0x00B2E6C0, 0x00B2E6C4), // sRForeArm:DEFAULT
    pair(0x00B11B94, 0x00B2E6C8, 0x00B2E6CC), // sRHand:DEFAULT
    pair(0x00B11B9C, 0x00B2E6D0, 0x00B2E6D4), // sRThigh:DEFAULT
    pair(0x00B11BA4, 0x00B2E6D8, 0x00B2E6DC), // sRCalf:DEFAULT
    pair(0x00B11BAC, 0x00B2E6E0, 0x00B2E6E4), // sRFoot:DEFAULT
    pair(0x00B11BB4, 0x00B2E6E8, 0x00B2E6EC), // sTail:DEFAULT
    pair(0x00B11BBC, 0x00B2E6F0, 0x00B2E6F4), // sSideWeapon:DEFAULT
    pair(0x00B11BC4, 0x00B2E6F8, 0x00B2E6FC), // sShield:DEFAULT
    pair(0x00B11BCC, 0x00B2E700, 0x00B2E704), // sQuiver:DEFAULT
    pair(0x00B11BD4, 0x00B2E708, 0x00B2E70C), // sBackWeapon:DEFAULT
    pair(0x00B11BDC, 0x00B2E718, 0x00B2E71C), // sPonyTail:DEFAULT
];
const HIT_TRANSFERS: usize = 10;
const QUAD_HIT_TRANSFERS: usize = 12;

/// IDA: N/A where the bare "BlendSettings.ini" (00B11A20) at 0053AC86 resolves. Windows resolves a
/// bare name against the Windows directory, and UAC file virtualization makes a
/// `LocalAppData\VirtualStore\Windows` copy win when present. The host reads that location
/// explicitly. Recorded substitution: the Windows directory is a Wine prefix's here.
#[must_use]
pub fn blend_settings_version_file(windows_directory: &Path, local_app_data: &Path) -> PathBuf {
    let virtualized = local_app_data.join("VirtualStore/Windows/BlendSettings.ini");
    if virtualized.exists() {
        virtualized
    } else {
        windows_directory.join("BlendSettings.ini")
    }
}

/// IDA: the destination globals of the always-run transfers: the float globals by original address
/// (88 pair floats, 00B2EC54..60, 00B2FD20..2C) and the 32-float tables 00B2EE68 and 00B2EEE8
/// (.data image values: 25 x 1.0 / 0.25, then 7 x -1.0, before the >= 0 replacement).
#[derive(Resource, Debug)]
pub struct BlendSettingsState {
    pub values: BTreeMap<u32, f32>,
    pub get_up_times: [f32; 32],
    pub knock_down_times: [f32; 32],
}

impl Default for BlendSettingsState {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        let mut get_up_times = [-1.0; 32];
        let mut knock_down_times = [-1.0; 32];
        get_up_times[..25].fill(1.0);
        knock_down_times[..25].fill(0.25);
        Self {
            values: BTreeMap::new(),
            get_up_times,
            knock_down_times,
        }
    }
}

impl BlendSettingsState {
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn value(&self, address: u32) -> f32 {
        self.values.get(&address).copied().unwrap_or(0.0)
    }
}

// IDA: N/A a helper, with no address of its own.
fn float_setting(blend: &SettingCollection, address: u32) -> f32 {
    blend
        .find_address(address)
        .map_or(0.0, super::settings::SettingInfo::as_float)
}

// IDA: N/A a helper, with no address of its own.
fn parse_pair(blend: &SettingCollection, transfer: &BlendTransfer, state: &mut BlendSettingsState) {
    let (mut first, mut second) = (1.0, 1.0);
    if let Some(text) = blend
        .find_address(transfer.setting)
        .and_then(|setting| setting.text.as_ref())
    {
        scan_f32_pair(text, &mut first, &mut second);
    }
    state.values.insert(transfer.first, first);
    state.values.insert(transfer.second, second);
}

/// IDA: 0053A1B0, 0053A460, 0053A720, in that order. Each string setting is parsed with
/// sscanf("%f, %f") into a pair preset to 1.0 / 1.0. 0053A3F4..0053A451 copies fMinVelGain and
/// fMinHeirGain and replaces every table entry >= 0 (fcom/jp: negative and NaN entries stay) with
/// fGetUpTime / fKnockDownTime. 0053AC14..0053AC53 copies fPassOutForce, fPassOutTime and calls
/// 008A3180(fHighTrans, fHighRot, fLowTrans, fLowRot). Completion: 100% order.
pub fn transfer_blend_settings(blend: &SettingCollection, state: &mut BlendSettingsState) {
    let (hit, rest) = TRANSFERS.split_at(HIT_TRANSFERS);
    for transfer in hit {
        parse_pair(blend, transfer, state);
    }
    state
        .values
        .insert(0x00B2_EC58, float_setting(blend, 0x00B1_1A34)); // fMinVelGain
    state
        .values
        .insert(0x00B2_EC54, float_setting(blend, 0x00B1_1A3C)); // fMinHeirGain
    let get_up = float_setting(blend, 0x00B1_1A24); // fGetUpTime
    let knock_down = float_setting(blend, 0x00B1_1A2C); // fKnockDownTime
    for entry in &mut state.get_up_times {
        if *entry >= 0.0 {
            *entry = get_up;
        }
    }
    for entry in &mut state.knock_down_times {
        if *entry >= 0.0 {
            *entry = knock_down;
        }
    }
    let (quad_hit, default) = rest.split_at(QUAD_HIT_TRANSFERS);
    for transfer in quad_hit.iter().chain(default) {
        parse_pair(blend, transfer, state);
    }
    state
        .values
        .insert(0x00B2_EC5C, float_setting(blend, 0x00B1_1C0C)); // fPassOutForce
    state
        .values
        .insert(0x00B2_EC60, float_setting(blend, 0x00B1_1C04)); // fPassOutTime
    // 008A3180(fHighTrans, fHighRot, fLowTrans, fLowRot)
    state
        .values
        .insert(0x00B2_FD20, float_setting(blend, 0x00B1_1BE4));
    state
        .values
        .insert(0x00B2_FD24, float_setting(blend, 0x00B1_1BEC));
    state
        .values
        .insert(0x00B2_FD28, float_setting(blend, 0x00B1_1BF4));
    state
        .values
        .insert(0x00B2_FD2C, float_setting(blend, 0x00B1_1BFC));
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlendSettingsLoad {
    pub version: u32,
    pub version_gate_passed: bool,
    pub collection_opened: bool,
    pub collection_read_all: bool,
}

/// IDA: 0053AC60 BlendSettings_Initialize: VERSION from the bare name, compared unsigned >= 14; on
/// success the 00B11C40 path becomes 00B3F280 + "BlendSettings.ini" and vt+0x14(0), vt+0x20 and
/// vt+0x18 run; the three transfers always run. Completion: 100% order.
pub fn initialize_blend_settings(
    collections: &mut SettingCollections,
    my_games_directory: &Path,
    version_file: &Path,
    state: &mut BlendSettingsState,
) -> BlendSettingsLoad {
    let mut result = BlendSettingsLoad {
        version: profile::read_int(version_file, b"DEFAULT", b"VERSION", 0),
        ..BlendSettingsLoad::default()
    };
    result.version_gate_passed = result.version >= 14;
    if result.version_gate_passed {
        collections
            .blend
            .set_path(my_games_directory.join("BlendSettings.ini"));
        result.collection_opened = collections.blend.open_ini();
        if result.collection_opened {
            result.collection_read_all = collections.blend.read_all();
            collections.blend.close();
        }
    }
    transfer_blend_settings(&collections.blend, state);
    result
}
