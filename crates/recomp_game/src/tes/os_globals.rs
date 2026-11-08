//! WinMain 0040E812..0040E831: the OS globals. DirectInput and DirectSound are data here: device calls go through
//! `OsGlobalsPlatform` by original call site, and every result that can fail stays scriptable. Runtime evidence:
//! P04-RUN-001. Host substitutions: the host never calls Win32, COM, DirectInput or DirectSound, and handles are host
//! values, never compared with evidence.

use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bevy_ecs::prelude::*;
use recomp_core::crt::strtol_hex;
use recomp_core::profile;
use recomp_core::x87::ftol64_low_f64;

use super::settings::{SettingCollection, SettingCollections};

/// IDA: N/A lock a mutex shared with the movie thread. A panic on the other side has already failed its test, and the
/// original has no such state, so a poisoned lock still answers.
pub fn lock_shared<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// IDA: N/A state the main thread and the MoviePlayer thread both reach: the role of the C++ `os_input_mutex` host lock.
/// The original reads and writes these globals unsynchronized (recorded). A clone names the same state; `==` compares
/// the contents. Lock order: input, then TimeInfo. Nothing takes the input while it holds TimeInfo, and nobody holds
/// TimeInfo for longer than one read or one update.
#[derive(Default)]
pub struct Shared<T>(Arc<Mutex<T>>);

impl<T> Shared<T> {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn new(value: T) -> Self {
        Self(Arc::new(Mutex::new(value)))
    }

    /// IDA: N/A an accessor.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        lock_shared(&self.0)
    }
}

impl<T> Clone for Shared<T> {
    /// IDA: N/A Rust's `Clone`.
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: PartialEq> PartialEq for Shared<T> {
    /// IDA: N/A Rust's `PartialEq`.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || *self.lock() == *other.lock()
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Shared<T> {
    /// IDA: N/A Rust's `Debug`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.lock().fmt(formatter)
    }
}

/// IDA: N/A g_osGlobals +0x20 for both threads: 004046A0 and 00403520 run on the MoviePlayer thread inside 00410390.
pub type SharedOsInput = Shared<OsInputGlobals>;

/// IDA: 006A9B40 / 006A9C00's byte `+0xA4` of OSSoundGlobals, in a small shared atomic: the MoviePlayer thread's 00410A70
/// pauses and resumes it while the main thread owns the rest of the object. A clone names the same byte. Completion: 100%
/// of the byte.
#[derive(Clone, Debug, Default)]
pub struct GameSoundPause(Arc<AtomicU8>);

impl GameSoundPause {
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn load(&self) -> u8 {
        self.0.load(Ordering::Acquire)
    }

    /// IDA: 006A9B40 OSSoundGlobals_PauseGameSounds (Phase 06, D4): 006A9B46 byte +0xA4 = 1 before the bSoundEnabled test,
    /// then 006A9B62..006A9BE8 walk the +0x300 map (no host instances). Completion: 100% of the reachable behaviour.
    /// TODO: [ ] Sound phases: the map walk over live TESGameSound instances.
    pub fn pause(&self, _sound_enabled: bool) {
        self.0.store(1, Ordering::Release);
    }

    /// IDA: 006A9C00 OSSoundGlobals_ResumeGameSounds (Phase 06, D4): 006A9C06 byte +0xA4 = 0 before the bSoundEnabled test,
    /// then 006A9C22..006A9CBA re-enable the marked sounds (no host instances). Completion: 100% of the reachable behaviour.
    /// TODO: [ ] Sound phases: the map walk over live TESGameSound instances.
    pub fn resume(&self, _sound_enabled: bool) {
        self.0.store(0, Ordering::Release);
    }
}

impl PartialEq for GameSoundPause {
    /// IDA: N/A Rust's `PartialEq`.
    fn eq(&self, other: &Self) -> bool {
        self.load() == other.load()
    }
}

/// IDA: 0040E812 FormHeap_Allocate sizes and the x86 field offsets used below.
pub mod layout {
    pub const GLOBALS_SIZE: u32 = 0x28;
    pub const INPUT_SIZE: u32 = 0x1BD8;
    pub const SOUND_SIZE: u32 = 0x328;
    pub const GLOBALS_INPUT: u32 = 0x20;
    pub const GLOBALS_SOUND: u32 = 0x24;
    pub const INPUT_BINDINGS: u32 = 0x1B7E;
    pub const BINDING_COUNT: usize = 29;
    pub const MAX_JOYSTICKS: u32 = 8;
}

pub const S_OK: u32 = 0;
pub const S_FALSE: u32 = 1;
pub const E_FAIL: u32 = 0x8000_4005;
pub const E_NOINTERFACE: u32 = 0x8000_4002;
pub const E_OUTOFMEMORY: u32 = 0x8007_000E;
pub const RPC_E_CHANGED_MODE: u32 = 0x8001_0106;
pub const DIERR_NOTACQUIRED: u32 = 0x8007_000C;
pub const DSERR_INVALIDPARAM: u32 = 0x8007_0057;
pub const DSERR_NODRIVER: u32 = 0x8878_0078;
pub const DSERR_OUTOFMEMORY: u32 = 0x8007_000E;

pub const DIEDFL_ATTACHEDONLY: u32 = 1;
pub const DI8DEVTYPE_JOYSTICK: u8 = 0x14;
pub const DI8DEVTYPE_GAMEPAD: u8 = 0x15;
pub const DISCL_EXCLUSIVE: u32 = 1;
pub const DISCL_NONEXCLUSIVE: u32 = 2;
pub const DISCL_FOREGROUND: u32 = 4;
pub const DISCL_BACKGROUND: u32 = 8;
pub const DIDFT_ALL: u32 = 0;
pub const DIPH_DEVICE: u32 = 0;
pub const DIPROP_BUFFERSIZE: u32 = 1;
pub const DIPROP_RANGE: u32 = 4;
pub const DIPROP_DEADZONE: u32 = 5;
pub const DIK_ESCAPE: u8 = 0x01;
pub const DIK_GRAVE: u8 = 0x29;
pub const DIK_SYSRQ: u8 = 0xB7;
pub const DSSCL_PRIORITY: u32 = 2;
/// DSBCAPS_PRIMARYBUFFER | DSBCAPS_CTRL3D | DSBCAPS_CTRLVOLUME.
pub const PRIMARY_BUFFER_FLAGS: u32 = 0x91;
pub const DS3D_IMMEDIATE: u32 = 0;
/// IDA: sizeof(DIDEVCAPS), sizeof(DSBUFFERDESC), sizeof(DSCAPS), the x86 DIDEVICEOBJECTDATA.
pub const DIDEVCAPS_SIZE: u32 = 0x2C;
pub const DSBUFFERDESC_SIZE: u32 = 0x24;
pub const DSCAPS_SIZE: u32 = 0x60;
pub const DIDEVICEOBJECTDATA_SIZE: u32 = 20;
pub const DIJOYSTATE_SIZE: u32 = 0x50;
pub const DIMOUSESTATE2_SIZE: u32 = 0x14;

/// IDA: 00B33EA0 is TimeInfo 00B33E90 +0x10; P04-RUN-001 read 0 when the sound constructor copied it (no frame yet).
pub const PHASE04_TIME_INFO_TICK: u32 = 0;

/// IDA: N/A a host window handle. Never compared with evidence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HostWindow(pub u32);

/// IDA: N/A a host process instance. The value of the evidence run's `hInstance` is 00400000.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HostInstance(pub u32);

/// IDA: N/A a host IDirectInputDevice8A. Never compared with evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InputDevice(pub u32);

/// A Win32 GUID in its original field layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl Guid {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub const fn new(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        Self {
            data1,
            data2,
            data3,
            data4,
        }
    }

    /// IDA: N/A a helper, with no address of its own.
    /// The 16 bytes as they sit in memory.
    #[must_use]
    pub fn to_bytes(self) -> [u8; 16] {
        let mut bytes = [0; 16];
        bytes[0..4].copy_from_slice(&self.data1.to_le_bytes());
        bytes[4..6].copy_from_slice(&self.data2.to_le_bytes());
        bytes[6..8].copy_from_slice(&self.data3.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.data4);
        bytes
    }
}

const DINPUT_TAIL: [u8; 8] = [0xBF, 0xC7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00];
pub const GUID_SYS_MOUSE: Guid = Guid::new(0x6F1D_2B60, 0xD5A0, 0x11CF, DINPUT_TAIL);
pub const GUID_SYS_KEYBOARD: Guid = Guid::new(0x6F1D_2B61, 0xD5A0, 0x11CF, DINPUT_TAIL);
pub const GUID_X_AXIS: Guid = Guid::new(0xA36D_02E0, 0xC9F3, 0x11CF, DINPUT_TAIL);
pub const GUID_Y_AXIS: Guid = Guid::new(0xA36D_02E1, 0xC9F3, 0x11CF, DINPUT_TAIL);
pub const GUID_Z_AXIS: Guid = Guid::new(0xA36D_02E2, 0xC9F3, 0x11CF, DINPUT_TAIL);
pub const GUID_RX_AXIS: Guid = Guid::new(0xA36D_02F4, 0xC9F3, 0x11CF, DINPUT_TAIL);
pub const GUID_RY_AXIS: Guid = Guid::new(0xA36D_02F5, 0xC9F3, 0x11CF, DINPUT_TAIL);
pub const GUID_RZ_AXIS: Guid = Guid::new(0xA36D_02E3, 0xC9F3, 0x11CF, DINPUT_TAIL);
/// IDA: A78E84 (6ABE9D's second argument).
pub const IID_DIRECT_SOUND_3D_LISTENER: Guid = Guid::new(
    0x279A_FA84,
    0x4981,
    0x11CE,
    [0xA5, 0x21, 0x00, 0x20, 0xAF, 0x0B, 0xE5, 0x60],
);

/// IDA: the three DIDATAFORMAT globals, by original address (evidence only, never read).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataFormat(pub u32);
pub const C_DF_DI_KEYBOARD: DataFormat = DataFormat(0x00A7_8E5C);
pub const C_DF_DI_MOUSE2: DataFormat = DataFormat(0x00A7_8A4C);
pub const C_DF_DI_JOYSTICK: DataFormat = DataFormat(0x00A7_8C54);

/// IDA: DIDEVCAPS (0x2C).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiDevCaps {
    pub size: u32,
    pub flags: u32,
    pub dev_type: u32,
    pub axes: u32,
    pub buttons: u32,
    pub povs: u32,
    pub ff_sample_period: u32,
    pub ff_min_time_resolution: u32,
    pub firmware_revision: u32,
    pub hardware_revision: u32,
    pub ff_driver_version: u32,
}

/// IDA: DIJOYSTATE (0x50).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiJoyState {
    pub axes: [i32; 6],
    pub sliders: [i32; 2],
    pub pov: [u32; 4],
    pub buttons: [u8; 32],
}

/// IDA: DIMOUSESTATE2 (0x14).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiMouseState2 {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub buttons: [u8; 8],
}

/// IDA: DIDEVICEINSTANCEA (0x244), which 00403DD0 copies whole. The name fields stay at their fixed 260 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInstance {
    pub size: u32,
    pub guid_instance: Guid,
    pub guid_product: Guid,
    pub dev_type: u32,
    pub instance_name: [u8; 260],
    pub product_name: [u8; 260],
    pub guid_ff_driver: Guid,
    pub usage_page: u16,
    pub usage: u16,
}

impl Default for DeviceInstance {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            size: 0,
            guid_instance: Guid::default(),
            guid_product: Guid::default(),
            dev_type: 0,
            instance_name: [0; 260],
            product_name: [0; 260],
            guid_ff_driver: Guid::default(),
            usage_page: 0,
            usage: 0,
        }
    }
}

/// IDA: the two DIDEVICEOBJECTINSTANCEA fields 004035D0 reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceObjectInstance {
    pub guid_type: Guid,
    pub object_type: u32,
}

/// IDA: DSCAPS (0x60). `+0x34` is the field 006ABF2A tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DsCaps {
    pub size: u32,
    pub words_04: [u32; 12],
    pub max_hw_3d_streaming_buffers_34: u32,
    pub words_38: [u32; 10],
}

/// IDA: the DIPROPDWORD and DIPROPRANGE the original passes, all with `DIPROPHEADER {size, 0x10, 0, DIPH_DEVICE}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceProperty {
    /// `{20, 16, 0, DIPH_DEVICE, data}`.
    Dword { property: u32, data: u32 },
    /// `{24, 16, 0, DIPH_DEVICE, min, max}`.
    Range { property: u32, min: i32, max: i32 },
}

/// IDA: N/A one call the code issued through the platform, with the values it passed. Float arguments stay raw bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OsCall {
    Allocate {
        bytes: u32,
    },
    GetCurrentThreadId,
    GetCurrentThread,
    DuplicateHandle {
        source_process: u32,
        source_handle: u32,
        target_process: u32,
        access: u32,
        inherit: u32,
        options: u32,
    },
    /// 00747830: RaiseException(0x406D1388, 0, 4, {0x1000, name, id, 0}).
    NameThread {
        thread_id: u32,
        name: &'static str,
    },
    GetActiveWindow,
    DirectInput8Create {
        instance: HostInstance,
        version: u32,
    },
    EnumDevices {
        dev_type: u32,
        flags: u32,
    },
    CreateDevice {
        guid: Guid,
    },
    SetCooperativeLevel {
        device: InputDevice,
        window: HostWindow,
        level: u32,
    },
    SetDataFormat {
        device: InputDevice,
        format: DataFormat,
    },
    SetProperty {
        device: InputDevice,
        property: DeviceProperty,
    },
    GetCapabilities {
        device: InputDevice,
    },
    EnumObjects {
        device: InputDevice,
        flags: u32,
    },
    Acquire {
        device: InputDevice,
    },
    Poll {
        device: InputDevice,
    },
    GetDeviceState {
        device: InputDevice,
        size: u32,
    },
    GetDeviceData {
        device: InputDevice,
        element_size: u32,
        count: u32,
        flags: u32,
    },
    SwapMouseButton {
        swap: u32,
    },
    GetDoubleClickTime,
    DirectSoundCreate8,
    SoundSetCooperativeLevel {
        window: HostWindow,
        level: u32,
    },
    SoundGetCaps {
        size: u32,
    },
    CreateSoundBuffer {
        size: u32,
        flags: u32,
    },
    QueryListener {
        iid: Guid,
    },
    ListenerSetPosition {
        x_bits: u32,
        y_bits: u32,
        z_bits: u32,
        apply: u32,
    },
    ListenerSetRolloffFactor {
        rolloff_bits: u32,
        apply: u32,
    },
    CoInitialize,
}

/// IDA: N/A one trace row: the original call site and the call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OsTraceEntry {
    pub site: u32,
    pub call: OsCall,
}

/// IDA: N/A a pointer the call writes: the HRESULT, and whether the object came back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreatedInterface<T> {
    pub result: u32,
    pub object: Option<T>,
}

/// IDA: N/A the host seam in place of the APIs the OS globals call. The original calls them directly; a test scripts
/// them to reach each failure branch, and `recomp_host` will answer them from Bevy input and audio. `site` is the
/// original call site. Phase 06's movie thread polls through the same seam.
pub trait OsGlobalsPlatform {
    /// IDA: 00401F00 FormHeap_Allocate: false is a null result.
    fn allocate(&mut self, site: u32, bytes: u32) -> bool;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn get_current_thread_id(&mut self, site: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn get_current_thread(&mut self, site: u32) -> u32;
    /// IDA: see the words here, which name the addresses this relates to.
    /// Returns (BOOL, the target handle the call wrote).
    #[allow(clippy::too_many_arguments)] // 00404A62 DuplicateHandle's own seven arguments.
    fn duplicate_handle(
        &mut self,
        site: u32,
        source_process: u32,
        source_handle: u32,
        target_process: u32,
        access: u32,
        inherit: u32,
        options: u32,
    ) -> (u32, u32);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn name_thread(&mut self, site: u32, thread_id: u32, name: &'static str);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn get_active_window(&mut self, site: u32) -> HostWindow;

    /// IDA: N/A no address of its own; its comments cite 00403DD0.
    fn direct_input8_create(
        &mut self,
        site: u32,
        instance: HostInstance,
        version: u32,
    ) -> CreatedInterface<()>;
    /// IDA: see the words here, which name the addresses this relates to.
    /// The attached devices in DirectInput's order: the caller runs 00403DD0 on each.
    fn enum_devices(&mut self, site: u32, dev_type: u32, flags: u32) -> Vec<DeviceInstance>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn create_device(&mut self, site: u32, guid: Guid) -> CreatedInterface<InputDevice>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn set_cooperative_level(
        &mut self,
        site: u32,
        device: InputDevice,
        window: HostWindow,
        level: u32,
    ) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn set_data_format(&mut self, site: u32, device: InputDevice, format: DataFormat) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn set_property(&mut self, site: u32, device: InputDevice, property: DeviceProperty) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn get_capabilities(&mut self, site: u32, device: InputDevice, caps: &mut DiDevCaps) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn enum_objects(
        &mut self,
        site: u32,
        device: InputDevice,
        flags: u32,
    ) -> Vec<DeviceObjectInstance>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn acquire(&mut self, site: u32, device: InputDevice) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn poll(&mut self, site: u32, device: InputDevice) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn get_keyboard_state(&mut self, site: u32, device: InputDevice, state: &mut [u8; 256]) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn get_mouse_state(&mut self, site: u32, device: InputDevice, state: &mut DiMouseState2)
    -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn get_joystick_state(&mut self, site: u32, device: InputDevice, state: &mut DiJoyState)
    -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// GetDeviceData(element_size, NULL, &count, flags).
    fn get_device_data(
        &mut self,
        site: u32,
        device: InputDevice,
        element_size: u32,
        count: u32,
        flags: u32,
    ) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn swap_mouse_button(&mut self, site: u32, swap: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn get_double_click_time(&mut self, site: u32) -> u32;

    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn direct_sound_create8(&mut self, site: u32) -> CreatedInterface<()>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn sound_set_cooperative_level(&mut self, site: u32, window: HostWindow, level: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn sound_get_caps(&mut self, site: u32, caps: &mut DsCaps) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn create_sound_buffer(&mut self, site: u32, size: u32, flags: u32) -> CreatedInterface<()>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn query_listener(&mut self, site: u32, iid: Guid) -> CreatedInterface<()>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn listener_set_position(&mut self, site: u32, x: f32, y: f32, z: f32, apply: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn listener_set_rolloff_factor(&mut self, site: u32, rolloff: f32, apply: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn co_initialize(&mut self, site: u32) -> u32;
}

/// IDA: N/A the C++ `throw`s: where the original goes on to fault, the host stops and names the site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OsGlobalsError(pub String);

impl std::fmt::Display for OsGlobalsError {
    /// IDA: N/A Rust's `Display`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for OsGlobalsError {}

/// IDA: B02BB8 lpKeyName, the 29 [Controls] keys in table order.
pub const CONTROL_BINDING_KEYS: [&str; 29] = [
    "Forward",
    "Back",
    "SlideLeft",
    "SlideRight",
    "Use",
    "Activate",
    "Block",
    "Cast",
    "Ready Item",
    "Crouch/Sneak",
    "Run",
    "Always Run",
    "Auto Move",
    "Jump",
    "Toggle POV",
    "Menu Mode",
    "Rest",
    "Quick Menu",
    "Quick1",
    "Quick2",
    "Quick3",
    "Quick4",
    "Quick5",
    "Quick6",
    "Quick7",
    "Quick8",
    "QuickSave",
    "QuickLoad",
    "Grab",
];

// IDA: setting addresses the constructors read (`settings_table.rs`).
const USE_JOYSTICK: u32 = 0x00B0_2C2C; // bUse Joystick:Controls
const BACKGROUND_KEYBOARD: u32 = 0x00B0_2C34; // bBackground Keyboard:Controls
const BACKGROUND_MOUSE: u32 = 0x00B0_2C3C; // bBackground Mouse:Controls
const CONTROLS_VERSION: u32 = 0x00B0_2C4C; // fVersion:Controls
const MASTER_VOLUME: u32 = 0x00B1_6190; // fDefaultMasterVolume:Audio
const FOOT_VOLUME: u32 = 0x00B1_6198; // fDefaultFootVolume:Audio
const MUSIC_VOLUME: u32 = 0x00B1_61A0; // fDefaultMusicVolume:Audio
const EFFECTS_VOLUME: u32 = 0x00B1_61A8; // fDefaultEffectsVolume:Audio
const VOICE_VOLUME: u32 = 0x00B1_61B0; // fDefaultVoiceVolume:Audio
const UNDERWATER_DELTA: u32 = 0x00B1_61B8; // fUnderwaterFrequencyDelta:Audio
const ROLLOFF_FACTOR: u32 = 0x00B1_61D8; // fDSoundRolloffFactor:Audio

/// IDA: A2FB10 (double)1.8f, the 004045F4 comparison constant.
const CONTROLS_VERSION_VALUE: f64 = 1.799_999_952_316_284_2;
/// IDA: A2FAAC 0.1f dead zone and A2FA98 10000.0 scale (00403F16, 004030FF).
const JOYSTICK_DEAD_ZONE: f32 = 0.1;
const DEAD_ZONE_SCALE: f64 = 10000.0;
/// IDA: A31C80 10.0f rolloff ceiling.
const ROLLOFF_CEILING: f32 = 10.0;
/// IDA: 006ABCEA raw dword stored at +0xB4.
const SOUND_0B4: u32 = 0x3B9A_CA00;
/// IDA: 006ABD0B/006ABD45 NiTPointerMap bucket count.
const SOUND_MAP_BUCKETS: u32 = 37;

// IDA: N/A a helper, with no address of its own.
fn setting_bool(ini: &SettingCollection, address: u32, fallback: bool) -> bool {
    ini.find_address(address)
        .map_or(fallback, |setting| setting.raw & 0xFF == 1)
}

/// IDA: 00404E30/00403C00 value-or-zero getters: a null SettingInfo returns 0.0 and stores 0.0 into 00B35464; the
/// settings here always exist. Completion: 100%.
fn setting_float(ini: &SettingCollection, address: u32) -> f32 {
    ini.find_address(address)
        .map_or(0.0, |setting| setting.as_float())
}

/// IDA: 004046DE/00404753/004047EC: `test eax, eax; jz` then `cmp eax, 1; jz` continue to Poll/GetDeviceState; any
/// other HRESULT, including every error, takes the zeroing branch. Completion: 100%.
const fn acquired_for_poll(result: u32) -> bool {
    result == S_OK || result == S_FALSE
}

// IDA: N/A a helper, with no address of its own.
const fn high_bit(byte: u8) -> bool {
    byte & 0x80 != 0
}

/// IDA: 004035D0 pair at +0x18B0 + n*8. `axes` bits 1/2/4/8/0x10/0x20 X/Y/Z/Rx/Ry/Rz; `buttons` 1 << (dwType >> 8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JoystickObjects {
    pub axes: u32,
    pub buttons: u32,
}

/// IDA: +0x30 + 0xA0 * i: the current then the previous DIJOYSTATE.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JoystickStatePair {
    pub current: DiJoyState,
    pub previous: DiJoyState,
}

/// IDA: 00403160's result: `called` means input was non-null, `device` that +0x28 was. The original ignores `result`
/// (P05-RUN-001: 0x8007000C DIERR_NOTACQUIRED).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputFlushResult {
    pub called: bool,
    pub device: bool,
    pub result: u32,
}

/// IDA: 00404150 OSInputGlobals_Initialize (0x1BD8 bytes). Phase 06 row 06-B-1: 004046A0 OSInputGlobals_PollDevices
/// (with its 00403C30 tail) and the control queries the MoviePlayer thread uses.
/// Completion: 100% of the reached calls, arguments and stores, as data; unwritten heap bytes are zero (recorded).
/// TODO: [ ] Menu input phase (D3, carried): an exclusive mouse (cooperative level 5, bBackground Mouse=0) is not
///       acquired, so the host menu keeps its window-message pointer; the poll takes the failed-Acquire branch.
///       P06-RUN-001's configuration (bBackground Mouse=1, level 6) acquires.
/// TODO: [ ] Teardown (carried): the original releases the devices at shutdown.
/// Phase 06: the movie thread shares this state through `SharedOsInput`, in `OsGlobalsData::input_20`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OsInputGlobals {
    pub flags_0000: u32,
    pub direct_input_0004: bool,
    pub joysticks_0008: [Option<InputDevice>; 8],
    pub keyboard_0028: Option<InputDevice>,
    pub mouse_002c: Option<InputDevice>,
    pub joystick_states_0030: [JoystickStatePair; 8],
    pub joystick_instances_0530: Vec<DeviceInstance>,
    pub joystick_caps_1750: [DiDevCaps; 8],
    pub joystick_objects_18b0: [JoystickObjects; 8],
    pub joystick_count_18f0: u32,
    pub keyboard_state_18f4: [u8; 256],
    pub previous_keyboard_state_19f4: [u8; 256],
    pub mouse_caps_1af4: DiDevCaps,
    pub mouse_state_1b20: DiMouseState2,
    pub previous_mouse_state_1b34: DiMouseState2,
    pub swap_mouse_buttons_1b48: u32,
    pub double_click_time_1b4c: u32,
    /// Heap bytes in the original until the first poll (+0x1B50).
    pub double_clicks_1b50: [u8; 8],
    pub last_click_times_1b58: [u32; 8],
    /// Never written (+0x1B78, heap bytes).
    pub unwritten_1b78: u32,
    pub zeroed_1b7c: u8,
    pub zeroed_1b7d: u8,
    /// +0x1B7E..+0x1BD7: the three tables, then three bytes the third table's defaults reach into.
    pub bindings_1b7e: [u8; 90],
    /// IDA: N/A host record of 00404150's mouse cooperative level (5 exclusive, 6 non-exclusive).
    pub mouse_exclusive: bool,
    /// IDA: N/A the D3 policy switch, off by default; fixtures enable it.
    pub acquire_exclusive_mouse: bool,
}

impl OsInputGlobals {
    pub const JOYSTICK_FLAG: u32 = 1;
    pub const MOUSE_FLAG: u32 = 2;
    pub const KEYBOARD_FLAG: u32 = 4;
    pub const BACKGROUND_MOUSE_FLAG: u32 = 8;

    /// IDA: InputKeyQuery (004032D0/004031E0/00402FC0); 4 is mouse-only.
    pub const QUERY_HELD: i32 = 0;
    pub const QUERY_PRESSED: i32 = 1;
    pub const QUERY_RELEASED: i32 = 2;
    pub const QUERY_CHANGED: i32 = 3;
    pub const QUERY_DOUBLE_CLICK: i32 = 4;

    /// IDA: N/A every field as the zeroed host allocation.
    fn zeroed() -> Self {
        Self {
            flags_0000: 0,
            direct_input_0004: false,
            joysticks_0008: [None; 8],
            keyboard_0028: None,
            mouse_002c: None,
            joystick_states_0030: [JoystickStatePair::default(); 8],
            joystick_instances_0530: vec![DeviceInstance::default(); 8],
            joystick_caps_1750: [DiDevCaps::default(); 8],
            joystick_objects_18b0: [JoystickObjects::default(); 8],
            joystick_count_18f0: 0,
            keyboard_state_18f4: [0; 256],
            previous_keyboard_state_19f4: [0; 256],
            mouse_caps_1af4: DiDevCaps::default(),
            mouse_state_1b20: DiMouseState2::default(),
            previous_mouse_state_1b34: DiMouseState2::default(),
            swap_mouse_buttons_1b48: 0,
            double_click_time_1b4c: 0,
            double_clicks_1b50: [0; 8],
            last_click_times_1b58: [0; 8],
            unwritten_1b78: 0,
            zeroed_1b7c: 0,
            zeroed_1b7d: 0,
            bindings_1b7e: [0; 90],
            mouse_exclusive: false,
            acquire_exclusive_mouse: false,
        }
    }

    // IDA: 00404150 OSInputGlobals_Initialize(this, hInstance). Order: +0x18F0 = 0, +0 = 0, DirectInput8Create (result
    // unchecked); bUse Joystick == 1: flags |= 1, EnumDevices(0, 00403DD0, this, DIEDFL_ATTACHEDONLY), no joystick
    // clears bit 0; +0x28 = 0; keyboard CreateDevice(GUID_SysKeyboard): on success flags |= 4, +0x28 = device,
    // SetCooperativeLevel(GetActiveWindow(), bBackground Keyboard == 1 ? 0x16 : 0x15), SetDataFormat(c_dfDIKeyboard),
    // SetProperty(DIPROP_BUFFERSIZE, {20, 16, 0, DIPH_DEVICE, 32}); +0x2C = 0; SwapMouseButton(FALSE) stored and passed
    // back; GetDoubleClickTime; zero +0x1B58..+0x1B74; DIDEVCAPS at +0x1AF4 zeroed with dwSize 44; bBackground Mouse ==
    // 1: flags |= 8; mouse CreateDevice(GUID_SysMouse): on success flags |= 2, +0x2C = device, GetCapabilities,
    // SetCooperativeLevel(GetActiveWindow(), bit 8 ? 6 : 5), SetDataFormat(c_dfDIMouse2); zero +0x30 (0x500), +0x18F4
    // (0x200), +0x1B20..+0x1B44, +0x1B7C, +0x1B7D; 00403960(this, 3). The device HRESULTs after CreateDevice are ignored.
    // Completion: 100%; a failed DirectInput8Create leaves a null interface that the original then reads through (it
    // faults at 0040418B): host stop, the recorded error contract.
    /// # Errors
    /// The original faults: DirectInput8Create returned no interface.
    pub fn new(
        instance: HostInstance,
        settings: &SettingCollections,
        platform: &mut dyn OsGlobalsPlatform,
    ) -> Result<Box<Self>, OsGlobalsError> {
        let ini = &settings.ini;
        let mut this = Box::new(Self::zeroed());
        this.joystick_count_18f0 = 0;
        this.flags_0000 = 0;
        let created = platform.direct_input8_create(0x0040_4177, instance, 0x0800);
        if created.object.is_none() {
            // 0040418B/004041B3 read through the null interface: the original faults.
            return Err(OsGlobalsError(format!(
                "DirectInput8Create failed (HRESULT {}): the original dereferences the null interface at 0040418B",
                created.result
            )));
        }
        this.direct_input_0004 = true;
        if setting_bool(ini, USE_JOYSTICK, true) {
            this.flags_0000 |= Self::JOYSTICK_FLAG;
            // 0040419A: the result is ignored. The callback's FALSE ends the enumeration.
            for device in platform.enum_devices(0x0040_419A, 0, DIEDFL_ATTACHEDONLY) {
                if !this.enum_joystick(&device, platform) {
                    break;
                }
            }
            if this.joystick_count_18f0 == 0 {
                this.flags_0000 &= !Self::JOYSTICK_FLAG;
            }
        }
        this.keyboard_0028 = None;
        let keyboard = platform.create_device(0x0040_41BE, GUID_SYS_KEYBOARD);
        if keyboard.result == S_OK
            && let Some(device) = keyboard.object
        {
            this.flags_0000 |= Self::KEYBOARD_FLAG;
            this.keyboard_0028 = Some(device);
            let level = if setting_bool(ini, BACKGROUND_KEYBOARD, false) {
                0x16
            } else {
                0x15
            };
            let window = platform.get_active_window(0x0040_41F3);
            platform.set_cooperative_level(0x0040_41FC, device, window, level);
            platform.set_data_format(0x0040_420C, device, C_DF_DI_KEYBOARD);
            platform.set_property(
                0x0040_4240,
                device,
                DeviceProperty::Dword {
                    property: DIPROP_BUFFERSIZE,
                    data: 32,
                },
            );
        }
        this.mouse_002c = None;
        this.swap_mouse_buttons_1b48 = platform.swap_mouse_button(0x0040_424E, 0);
        platform.swap_mouse_button(0x0040_4257, this.swap_mouse_buttons_1b48);
        this.double_click_time_1b4c = platform.get_double_click_time(0x0040_4259);
        this.last_click_times_1b58 = [0; 8];
        this.mouse_caps_1af4 = DiDevCaps {
            size: DIDEVCAPS_SIZE,
            ..DiDevCaps::default()
        };
        if setting_bool(ini, BACKGROUND_MOUSE, false) {
            this.flags_0000 |= Self::BACKGROUND_MOUSE_FLAG;
        }
        let mouse = platform.create_device(0x0040_42D0, GUID_SYS_MOUSE);
        if mouse.result == S_OK
            && let Some(device) = mouse.object
        {
            this.flags_0000 |= Self::MOUSE_FLAG;
            this.mouse_002c = Some(device);
            platform.get_capabilities(0x0040_42E7, device, &mut this.mouse_caps_1af4);
            let level = if this.flags_0000 & Self::BACKGROUND_MOUSE_FLAG != 0 {
                6
            } else {
                5
            };
            this.mouse_exclusive = level == DISCL_EXCLUSIVE | DISCL_FOREGROUND;
            let window = platform.get_active_window(0x0040_42FC);
            platform.set_cooperative_level(0x0040_4305, device, window, level);
            platform.set_data_format(0x0040_4315, device, C_DF_DI_MOUSE2);
        }
        this.joystick_states_0030 = [JoystickStatePair::default(); 8];
        this.keyboard_state_18f4 = [0; 256];
        this.previous_keyboard_state_19f4 = [0; 256];
        this.mouse_state_1b20 = DiMouseState2::default();
        this.previous_mouse_state_1b34 = DiMouseState2::default();
        this.zeroed_1b7c = 0;
        this.zeroed_1b7d = 0;
        this.reset_bindings(3);
        Ok(this)
    }

    // IDA: 00403960(this, which): 0..2 resets one table to 0xFF, 3 all three, then writes the defaults (keyboard for
    // 0/3, mouse for 1/3, joystick for 2/3 only when flag bit 0 is set). The reset loop runs i = 0..28 first.
    // Completion: 100%, including the joystick store at +0x1BD5 one byte past the 29-entry table.
    //
    // The joystick block follows the disassembly (00403AB5..00403B38, table at +0x1BB8), not the C++ port: the C++ wrote
    // every joystick default 4 entries too low (+5 for +0x1BC1, and so on), and lost the +0x1BD5 store its own header
    // names. No test or row covered it; no run attached a joystick.
    pub fn reset_bindings(&mut self, which: i32) {
        const KEYBOARD: usize = 0;
        const MOUSE: usize = layout::BINDING_COUNT;
        const JOYSTICK: usize = 2 * layout::BINDING_COUNT;
        let bindings = &mut self.bindings_1b7e;
        for i in 0..layout::BINDING_COUNT {
            if (0..=2).contains(&which) {
                bindings[which as usize * layout::BINDING_COUNT + i] = 0xFF;
            } else if which == 3 {
                bindings[KEYBOARD + i] = 0xFF;
                bindings[MOUSE + i] = 0xFF;
                bindings[JOYSTICK + i] = 0xFF;
            }
        }
        if which == 0 || which == 3 {
            // 004039BD..00403A78 in store order (DIK codes).
            bindings[KEYBOARD] = 0x11; // W
            bindings[KEYBOARD + 1] = 0x1F; // S
            bindings[KEYBOARD + 2] = 0x1E; // A
            bindings[KEYBOARD + 3] = 0x20; // D
            bindings[KEYBOARD + 5] = 0x39; // Space
            bindings[KEYBOARD + 6] = 0x38; // Left Alt
            bindings[KEYBOARD + 7] = 0x2E; // C
            bindings[KEYBOARD + 8] = 0x21; // F
            bindings[KEYBOARD + 9] = 0x1D; // Left Ctrl
            bindings[KEYBOARD + 10] = 0x2A; // Left Shift
            bindings[KEYBOARD + 11] = 0x3A; // Caps Lock
            bindings[KEYBOARD + 12] = 0x10; // Q
            bindings[KEYBOARD + 13] = 0x12; // E
            bindings[KEYBOARD + 14] = 0x13; // R
            bindings[KEYBOARD + 16] = 0x14; // T
            bindings[KEYBOARD + 15] = 0x0F; // Tab
            bindings[KEYBOARD + 17] = 0x3B; // F1
            bindings[KEYBOARD + 18] = 0x02; // 1
            bindings[KEYBOARD + 19] = 0x03;
            bindings[KEYBOARD + 20] = 0x04;
            bindings[KEYBOARD + 21] = 0x05;
            bindings[KEYBOARD + 22] = 0x06;
            bindings[KEYBOARD + 23] = 0x07;
            bindings[KEYBOARD + 24] = 0x08;
            bindings[KEYBOARD + 25] = 0x09;
            bindings[KEYBOARD + 26] = 0x3F; // F5
            bindings[KEYBOARD + 27] = 0x43; // F9
            bindings[KEYBOARD + 28] = 0x2C; // Z
        }
        if which == 1 || which == 3 {
            bindings[MOUSE + 4] = 0;
            bindings[MOUSE + 6] = 1;
            bindings[MOUSE + 14] = 2;
        }
        if (which == 2 || which == 3) && self.flags_0000 & Self::JOYSTICK_FLAG != 0 {
            // 00403AB5..00403B38 in store order.
            bindings[JOYSTICK + 9] = 0; // +0x1BC1
            bindings[JOYSTICK + 13] = 1; // +0x1BC5
            bindings[JOYSTICK + 8] = 2; // +0x1BC0
            bindings[JOYSTICK + 5] = 3; // +0x1BBD
            bindings[JOYSTICK + 15] = 4; // +0x1BC7
            bindings[JOYSTICK + 14] = 5; // +0x1BC6
            bindings[JOYSTICK + 7] = 6; // +0x1BBF
            bindings[JOYSTICK + 28] = 7; // +0x1BD4
            bindings[JOYSTICK + 6] = 8; // +0x1BBE
            bindings[JOYSTICK + 4] = 9; // +0x1BBC
            bindings[JOYSTICK + 16] = 10; // +0x1BC8
            bindings[JOYSTICK + 29] = 11; // +0x1BD5, past the table
            bindings[JOYSTICK + 18] = 32; // +0x1BCA
            bindings[JOYSTICK + 19] = 33;
            bindings[JOYSTICK + 20] = 34;
            bindings[JOYSTICK + 21] = 35;
            bindings[JOYSTICK + 22] = 36;
            bindings[JOYSTICK + 23] = 37;
            bindings[JOYSTICK + 24] = 38;
            bindings[JOYSTICK + 25] = 39; // +0x1BD1
        }
    }

    /// IDA: N/A index 0 keyboard (+0x1B7E), 1 mouse (+0x1B9B), 2 joystick (+0x1BB8).
    #[must_use]
    pub const fn binding(&self, table: usize, index: usize) -> u8 {
        self.bindings_1b7e[table * layout::BINDING_COUNT + index]
    }

    /// IDA: N/A see `binding`.
    pub const fn set_binding(&mut self, table: usize, index: usize, value: u8) {
        self.bindings_1b7e[table * layout::BINDING_COUNT + index] = value;
    }

    // IDA: 00403DD0 EnumDevices callback. Only DI8DEVTYPE_JOYSTICK (0x14) and DI8DEVTYPE_GAMEPAD (0x15) continue: copy
    // the 0x244-byte instance to +0x530 + n*0x244, CreateDevice(guidInstance) (unchecked), SetCooperativeLevel(
    // GetActiveWindow(), DISCL_BACKGROUND | DISCL_NONEXCLUSIVE), SetDataFormat(c_dfDIJoystick), DIDEVCAPS +0x1750 +
    // n*0x2C zeroed with dwSize 44 then GetCapabilities, objects +0x18B0 + n*8 zeroed, EnumObjects(004035D0, that pair,
    // DIDFT_ALL), +8 + n*4 = device, SetProperty(DIPROP_RANGE, {24, 16, 0, DIPH_DEVICE, -100, 100}), 004030F0(n, 0.1f),
    // ++n; returns n != 8. Completion: 100% static (no joystick was attached in P04-RUN-001). The original calls through
    // a null device where CreateDevice failed; the C++ port skips those calls and this keeps that.
    fn enum_joystick(
        &mut self,
        device: &DeviceInstance,
        platform: &mut dyn OsGlobalsPlatform,
    ) -> bool {
        let dev_type = (device.dev_type & 0xFF) as u8;
        if dev_type != DI8DEVTYPE_JOYSTICK && dev_type != DI8DEVTYPE_GAMEPAD {
            return true; // DIENUM_CONTINUE
        }
        let index = self.joystick_count_18f0 as usize;
        if self.joystick_count_18f0 >= layout::MAX_JOYSTICKS {
            return false; // DIENUM_STOP
        }
        self.joystick_instances_0530[index] = device.clone();
        let joystick = platform
            .create_device(0x0040_3E26, device.guid_instance)
            .object;
        if let Some(joystick) = joystick {
            let window = platform.get_active_window(0x0040_3E33);
            platform.set_cooperative_level(
                0x0040_3E3D,
                joystick,
                window,
                DISCL_BACKGROUND | DISCL_NONEXCLUSIVE,
            );
            platform.set_data_format(0x0040_3E4A, joystick, C_DF_DI_JOYSTICK);
        }
        self.joystick_caps_1750[index] = DiDevCaps {
            size: DIDEVCAPS_SIZE,
            ..DiDevCaps::default()
        };
        if let Some(joystick) = joystick {
            platform.get_capabilities(0x0040_3E93, joystick, &mut self.joystick_caps_1750[index]);
        }
        self.joystick_objects_18b0[index] = JoystickObjects::default();
        if let Some(joystick) = joystick {
            for object in platform.enum_objects(0x0040_3EC9, joystick, DIDFT_ALL) {
                if !enum_joystick_object(&object, &mut self.joystick_objects_18b0[index]) {
                    break;
                }
            }
        }
        self.joysticks_0008[index] = joystick;
        if let Some(joystick) = joystick {
            platform.set_property(
                0x0040_3F14,
                joystick,
                DeviceProperty::Range {
                    property: DIPROP_RANGE,
                    min: -100,
                    max: 100,
                },
            );
        }
        self.set_joystick_dead_zone(self.joystick_count_18f0, JOYSTICK_DEAD_ZONE, platform);
        self.joystick_count_18f0 += 1;
        self.joystick_count_18f0 != layout::MAX_JOYSTICKS
    }

    /// IDA: 004030F0(index, fraction): SetProperty(DIPROP_DEADZONE, {20, 16, 0, DIPH_DEVICE, fraction * 10000.0}), fmul
    /// at 53-bit precision (before CreateDevice), then `fistp qword` under RC=chop, keeping the low dword. Completion:
    /// 100%.
    pub fn set_joystick_dead_zone(
        &self,
        index: u32,
        fraction: f32,
        platform: &mut dyn OsGlobalsPlatform,
    ) {
        let data = ftol64_low_f64(f64::from(fraction) * DEAD_ZONE_SCALE);
        if let Some(joystick) = self.joysticks_0008[index as usize] {
            platform.set_property(
                0x0040_3150,
                joystick,
                DeviceProperty::Dword {
                    property: DIPROP_DEADZONE,
                    data,
                },
            );
        }
    }

    /// IDA: 004046A0 OSInputGlobals_PollDevices (__fastcall). `time_info_10` is TimeInfo+0x10 (00B33EA0), read by the
    /// 00403C30 tail for each new mouse press. Completion: 100% of the calls, copies, store order and the S_OK/S_FALSE
    /// test, with the D3 exclusive-mouse substitution in the type's TODO.
    pub fn poll_devices(&mut self, time_info_10: u32, platform: &mut dyn OsGlobalsPlatform) {
        // 004046A7..0040472D: `cmp [+0x18F0], 0; jle` and `jl` signed loop over the joysticks; the state copy precedes
        // Acquire.
        let count = self.joystick_count_18f0 as i32;
        let mut index = 0;
        while index < count && index < 8 {
            let slot = index as usize;
            let pair = &mut self.joystick_states_0030[slot];
            pair.previous = pair.current;
            // The original calls through a null device here (it faults at 004046DC): host stop.
            let device = self.joysticks_0008[slot]
                .expect("004046DC: the original calls Acquire through a null joystick");
            if acquired_for_poll(platform.acquire(0x0040_46DC, device)) {
                platform.poll(0x0040_4702, device);
                platform.get_joystick_state(0x0040_470F, device, &mut pair.current);
            } else {
                pair.current = DiJoyState::default();
            }
            index += 1;
        }
        // 0040472F..00404788: keyboard +0x28.
        if let Some(keyboard) = self.keyboard_0028 {
            self.previous_keyboard_state_19f4 = self.keyboard_state_18f4;
            if acquired_for_poll(platform.acquire(0x0040_4751, keyboard)) {
                platform.poll(0x0040_4777, keyboard);
                platform.get_keyboard_state(0x0040_4788, keyboard, &mut self.keyboard_state_18f4);
            } else {
                self.keyboard_state_18f4 = [0; 256];
            }
        }
        // 0040478A..0040482A: mouse +0x2C; the five dwords +0x1B20..+0x1B30 move to +0x1B34..+0x1B44, then
        // +0x1B50/+0x1B54 are cleared.
        let Some(mouse) = self.mouse_002c else {
            return;
        };
        self.previous_mouse_state_1b34 = self.mouse_state_1b20;
        self.double_clicks_1b50 = [0; 8];
        // D3 (recorded substitution): an exclusive mouse is not acquired; the original's failed-Acquire branch runs.
        let skip_acquire = self.mouse_exclusive && !self.acquire_exclusive_mouse;
        if !skip_acquire && acquired_for_poll(platform.acquire(0x0040_47EA, mouse)) {
            platform.poll(0x0040_481C, mouse);
            platform.get_mouse_state(0x0040_482A, mouse, &mut self.mouse_state_1b20);
        } else {
            self.mouse_state_1b20 = DiMouseState2::default();
        }
        // 0040480E/00404835: jmp 00403C30.
        self.update_double_clicks(time_info_10);
    }

    /// IDA: 00403C30 (tail of 004046A0): per button 0..7 a new press either sets the double-click byte and clears the
    /// time (gap <= +0x1B4C, unsigned) or stores TimeInfo+0x10. Completion: 100%.
    fn update_double_clicks(&mut self, time_info_10: u32) {
        for button in 0..8 {
            if !high_bit(self.mouse_state_1b20.buttons[button])
                || high_bit(self.previous_mouse_state_1b34.buttons[button])
            {
                continue;
            }
            // 00403C54..00403C76: edi = now - last; `cmp edi, [+0x1B4C]; ja`.
            if time_info_10.wrapping_sub(self.last_click_times_1b58[button])
                > self.double_click_time_1b4c
            {
                self.last_click_times_1b58[button] = time_info_10;
            } else {
                self.double_clicks_1b50[button] = 1;
                self.last_click_times_1b58[button] = 0;
            }
        }
    }

    /// IDA: 00403520 OSInputGlobals_QueryControlState: controls 0..28 sum the three bound schemes; 29/30/31 are Escape,
    /// grave and SysRq just-pressed. Completion: 100%, with one recorded host value: a negative control reads object
    /// bytes before +0x1B7E in the original; the callers pass 0..31 and the host returns 0.
    #[must_use]
    pub fn query_control_state(&self, control: i32, query: i32) -> i32 {
        if control < 0 {
            return 0;
        }
        // 00403524 `cmp eax, 1Dh; jge`: controls below 29 (signed).
        if control < 29 {
            let mut total = 0;
            for scheme in 0..3 {
                // 00403533: +0x1B7E + control + 29 * scheme.
                let keycode =
                    self.bindings_1b7e[(control + scheme * layout::BINDING_COUNT as i32) as usize];
                if keycode != 0xFF {
                    total += self.query_input_state(scheme, keycode, query);
                }
            }
            return total;
        }
        // 0040356B..004035BC: fixed scan codes, just pressed; `query` unused.
        let just_pressed = |scan: u8| {
            i32::from(
                !high_bit(self.previous_keyboard_state_19f4[scan as usize])
                    && high_bit(self.keyboard_state_18f4[scan as usize]),
            )
        };
        match control {
            29 => just_pressed(DIK_ESCAPE),
            30 => just_pressed(DIK_GRAVE),
            31 => just_pressed(DIK_SYSRQ),
            _ => 0,
        }
    }

    /// IDA: 00403490 OSInputGlobals_QueryInputState: scheme 0 keyboard, 1 mouse, 0xFF none, 2.. joystick (buttons <
    /// 0x20, else a POV direction). Completion: 100%.
    #[must_use]
    pub fn query_input_state(&self, scheme: i32, keycode: u8, query: i32) -> i32 {
        match scheme {
            0 => return self.query_scan_code_state(keycode, query),
            1 => return self.query_mouse_button_state(keycode, query),
            0xFF => return 0,
            _ => {}
        }
        // 004034A7 `lea eax, [edx-2]; cmp eax, [+0x18F0]; jge`.
        let joystick = scheme - 2;
        if joystick >= self.joystick_count_18f0 as i32 {
            return 0;
        }
        if keycode < 0x20 {
            return self.query_joystick_button_state(joystick, keycode, query);
        }
        i32::from(self.joystick_pov_direction(joystick, 0) == i32::from(keycode))
    }

    /// IDA: 004032D0 OSInputGlobals_QueryScanCodeState. 004032D7 unsigned switch over 0..3. Completion: 100%.
    #[must_use]
    pub fn query_scan_code_state(&self, scan_code: u8, query: i32) -> i32 {
        let now = high_bit(self.keyboard_state_18f4[scan_code as usize]);
        let before = high_bit(self.previous_keyboard_state_19f4[scan_code as usize]);
        query_transition(now, before, query)
    }

    /// IDA: 004031E0 OSInputGlobals_QueryMouseButtonState: 8/9 wheel up/down, 0/1 swapped with +0x1B48, queries 0..4 (4
    /// = the double-click byte). Completion: 100% of buttons 0..9.
    /// TODO: [ ] Buttons 10..255 read object bytes past the 8 button bytes in the original (up to past the allocation);
    ///       the host returns 0.
    #[must_use]
    pub fn query_mouse_button_state(&self, button: u8, query: i32) -> i32 {
        // 004031E7..004031F3: the wheel pseudo-buttons read lZ (+0x1B28) signed.
        if button == 8 {
            return i32::from(self.mouse_state_1b20.z > 0);
        }
        if button == 9 {
            return i32::from(self.mouse_state_1b20.z < 0);
        }
        // 004031F9..00403219: swapped primary buttons.
        let mut index = u32::from(button);
        if index == 0 && self.swap_mouse_buttons_1b48 != 0 {
            index = 1;
        } else if index == 1 && self.swap_mouse_buttons_1b48 != 0 {
            index = 0;
        }
        if index >= 8 {
            return 0; // host: past the button bytes (see the TODO)
        }
        let index = index as usize;
        let now = high_bit(self.mouse_state_1b20.buttons[index]);
        let before = high_bit(self.previous_mouse_state_1b34.buttons[index]);
        // 00403220 unsigned switch over 0..4; 0040327C `cmp edx, 7; ja`.
        if query == Self::QUERY_DOUBLE_CLICK {
            return i32::from(self.double_clicks_1b50[index] != 0);
        }
        query_transition(now, before, query)
    }

    /// IDA: 00402FC0 OSInputGlobals_QueryJoystickButtonState. 00402FC8: +0x30 + 0xA0 * joystick; rgbButtons at +0x30 of
    /// each DIJOYSTATE. Completion: 100%.
    #[must_use]
    pub fn query_joystick_button_state(&self, joystick: i32, button: u8, query: i32) -> i32 {
        let pair = &self.joystick_states_0030[joystick as usize];
        let now = high_bit(pair.current.buttons[button as usize]);
        let before = high_bit(pair.previous.buttons[button as usize]);
        query_transition(now, before, query)
    }

    /// IDA: 00403070 OSInputGlobals_GetJoystickPovDirection: -1 when the low word is 0xFFFF (0040307E `cmp cx, 0FFFFh`),
    /// else (int)pov / 4500 + 0x20 (0040308F: imul by 0x7482296B, sar 11, + sign: signed, truncating). Completion: 100%.
    #[must_use]
    pub fn joystick_pov_direction(&self, joystick: i32, pov: i32) -> i32 {
        let value = self.joystick_states_0030[joystick as usize].current.pov[pov as usize];
        if value & 0xFFFF == 0xFFFF {
            return -1;
        }
        value as i32 / 4500 + 0x20
    }
}

/// IDA: the shared switch of 004032D0/004031E0/00402FC0 over queries 0..3; anything else is false.
const fn query_transition(now: bool, before: bool, query: i32) -> i32 {
    let result = match query {
        OsInputGlobals::QUERY_HELD => now,
        OsInputGlobals::QUERY_PRESSED => !before && now,
        OsInputGlobals::QUERY_RELEASED => !now && before,
        OsInputGlobals::QUERY_CHANGED => now != before,
        _ => false,
    };
    result as i32
}

/// IDA: 004035D0: guidType equal to GUID_XAxis/YAxis/ZAxis/RxAxis/RyAxis/RzAxis sets bits 1/2/4/8/0x10/0x20 of the first
/// dword; DIDFT_BUTTON types (dwType & 0xC) set 1 << ((dwType >> 8) & 31) in the second. Returns 1. Completion: 100%.
fn enum_joystick_object(object: &DeviceObjectInstance, objects: &mut JoystickObjects) -> bool {
    let guid = object.guid_type;
    if guid == GUID_X_AXIS {
        objects.axes |= 1;
    }
    if guid == GUID_Y_AXIS {
        objects.axes |= 2;
    }
    if guid == GUID_Z_AXIS {
        objects.axes |= 4;
    }
    if guid == GUID_RX_AXIS {
        objects.axes |= 8;
    }
    if guid == GUID_RY_AXIS {
        objects.axes |= 0x10;
    }
    if guid == GUID_RZ_AXIS {
        objects.axes |= 0x20;
    }
    if object.object_type & 0xC != 0 {
        objects.buttons |= 1 << ((object.object_type >> 8) & 31);
    }
    true
}

/// IDA: 00403160 OSInputGlobals_DiscardBufferedEvents after WinMain's 0040EAB1..0040EABF null test: when the keyboard at
/// +0x28 is non-null, 00403183 GetDeviceData(20, NULL, &count = -1 (INFINITE), 0) flushes its buffer; the HRESULT is
/// ignored. The C++ port has this in `tes/shader_system` (Phase 05 row 05-F); it is here because the window procedure
/// and the WinMain replay call it. Completion: 100%.
pub fn discard_buffered_input_events(
    input: Option<&OsInputGlobals>,
    platform: &mut dyn OsGlobalsPlatform,
) -> InputFlushResult {
    let Some(input) = input else {
        return InputFlushResult::default();
    };
    let mut result = InputFlushResult {
        called: true,
        ..InputFlushResult::default()
    };
    let Some(keyboard) = input.keyboard_0028 else {
        return result;
    };
    result.device = true;
    result.result = platform.get_device_data(
        0x0040_3183,
        keyboard,
        DIDEVICEOBJECTDATA_SIZE,
        0xFFFF_FFFF,
        0,
    );
    result
}

/// IDA: 006ABC80 OSSoundGlobals_Constructor (0x328 bytes). `co_initialized` and `co_initialize_result` are host records.
/// The COM objects are presence flags.
/// Completion: 100% of the reached calls and stores, with the two recorded substitutions: the 006ABE5F GetCaps buffer is
/// zeroed (see `new`) and the unwritten heap bytes are zero.
/// TODO: [ ] Sound phases (carried, D2): the TESGameSound/NiAVObject maps, message lists and DirectSound playback. The
///       Bevy audio replacement stays a recorded host substitution until then.
#[derive(Clone, Debug, PartialEq)]
pub struct OsSoundGlobals {
    pub direct_sound_08: bool,
    pub primary_buffer_0c: bool,
    pub caps_10: DsCaps,
    /// +0x70 IGraphBuilder, a host value (`os_sound_music`). Left set after a release, as the original leaves it.
    pub music_graph_70: Option<super::os_sound_music::MusicGraph>,
    /// +0x74 IBasicAudio, by presence.
    pub music_audio_74: bool,
    pub listener_78: bool,
    /// +0x80..+0x88 the listener's position (006A8E10), +0x8C..+0x94 its front and +0x98..+0xA0 its top (006A8E40). The constructor doesn't store them.
    pub listener_position_80: [f32; 3],
    pub listener_front_8c: [f32; 3],
    pub listener_top_98: [f32; 3],
    pub game_sounds_paused_a4: GameSoundPause,
    pub byte_a5: u8,
    pub byte_a6: u8,
    pub flags_ac: u32,
    pub current_music_type_b0: u16,
    pub raw_b4: u32,
    pub master_volume_b8: f32,
    pub foot_volume_bc: f32,
    pub voice_volume_c0: f32,
    pub effects_volume_c4: f32,
    /// +0xC8, +0xCC, +0xD0.
    pub time_c8: [u32; 3],
    pub music_flags_dc: u32,
    /// +0x1E4, the path of the current track, the original's bytes.
    pub music_path_1e4: Vec<u8>,
    /// +0x2E8, the tick a music request began.
    pub music_start_tick_2e8: u32,
    /// +0x2EC, that tick + 1700.
    pub music_fade_end_2ec: u32,
    pub music_volume_2f0: f32,
    pub unk_2f4: f32,
    pub music_volume2_2f8: f32,
    /// +0x2FC, the requested type, a word.
    pub music_next_type_2fc: u16,
    /// +0x300 and +0x304, 37 buckets each.
    pub map_buckets_300: u32,
    pub unk_324: u32,
    /// 00B3C214 points at this object from 006ABD88. Nothing reads it yet.
    pub audio_manager_b3c214: bool,
    pub co_initialized: bool,
    pub co_initialize_result: u32,
    /// 00B3C0EC: set by 005B5B2F and 005B5FB3, read by 006AA1E9. A byte of the MainMenu's that only sound reads here.
    pub main_menu_music_b3c0ec: u8,
    /// IDA: N/A counts each time the music code reached an arm not ported (`os_sound_music`), so that it's visible rather than silent.
    pub music_unported_arms: u32,
}

impl OsSoundGlobals {
    pub const CAPS_FLAG: u32 = 1;
    pub const HARDWARE_3D_STREAMING_FLAG: u32 = 4;

    // IDA: 006ABC80. Order: +0/+4 = 0, memset 0x328, +0xCC = TimeInfo tick, byte +0xA4 = 0, +0xB4 = 0x3B9ACA00,
    // NiTPointerMap<int, TESGameSound*>(37) at +0x300, NiTPointerMap<int, NiPointer<NiAVObject>>(37) at +0x304, NiTList
    // <uint> at +0x320, 00B3C214 = this, NiTPointerList<SoundMessage*> at +0x308, byte +0xA6 = 0,
    // fUnderwaterFrequencyDelta = 0.0f; DirectSoundCreate8(NULL, +8, NULL) != DS_OK -> return; SetCooperativeLevel(
    // window, DSSCL_PRIORITY) < 0 -> return; +0x10 = 0x60, +0x70 = 0, GetCaps(+0x10) < 0 -> return; +0xAC |= 1;
    // DSBUFFERDESC zeroed with dwSize 36; GetCaps(stack buffer) (result ignored); dwFlags 0x91; CreateSoundBuffer < 0 ->
    // return; QueryInterface(IID_IDirectSound3DListener, +0x78) < 0 -> return; SetPosition(FLT_MAX, FLT_MAX, FLT_MAX,
    // DS3D_IMMEDIATE); rolloff > 10.0 -> 10.0, rolloff < 0.0 -> 1.0 (both written back to the setting, NaN kept);
    // SetRolloffFactor(rolloff, DS3D_IMMEDIATE); byte +0xA5 = 0, +0x74 = 0; stack word +0x34 != 0 -> +0xAC |= 4; +0x324 =
    // 0; +0xD0 = tick; word +0xB0 = 0; CoInitialize(NULL); +0xDC = 0; master +0xB8, music +0x2F8 and +0x2F0, effects
    // +0xC4, foot +0xBC, voice +0xC0, +0x2F4 = 0.0f, +0xC8 = tick. Completion: 100% with the substitutions on the type.
    //
    // Recorded substitution: the original passes an uninitialized stack buffer at 006ABE5F, so dwSize is stack contents
    // and GetCaps returns DSERR_INVALIDPARAM in practice, leaving the stack bytes. In P04-RUN-001 its +0x34 was 04C89800,
    // non-zero, so the original's flags ended 5; the host passes a zeroed DSCAPS, so they end 1 unless the seam writes
    // the buffer. A seam can reach the `|= 4` branch.
    #[must_use]
    pub fn new(
        window: HostWindow,
        settings: &mut SettingCollections,
        platform: &mut dyn OsGlobalsPlatform,
    ) -> Box<Self> {
        let ini = &mut settings.ini;
        let mut this = Box::new(Self {
            direct_sound_08: false,
            primary_buffer_0c: false,
            caps_10: DsCaps::default(),
            music_graph_70: None,
            music_audio_74: false,
            listener_78: false,
            listener_position_80: [0.0; 3],
            listener_front_8c: [0.0; 3],
            listener_top_98: [0.0; 3],
            game_sounds_paused_a4: GameSoundPause::default(),
            byte_a5: 0,
            byte_a6: 0,
            flags_ac: 0,
            current_music_type_b0: 0,
            raw_b4: 0,
            master_volume_b8: 0.0,
            foot_volume_bc: 0.0,
            voice_volume_c0: 0.0,
            effects_volume_c4: 0.0,
            time_c8: [0; 3],
            music_flags_dc: 0,
            music_path_1e4: Vec::new(),
            music_start_tick_2e8: 0,
            music_fade_end_2ec: 0,
            music_volume_2f0: 0.0,
            unk_2f4: 0.0,
            music_volume2_2f8: 0.0,
            music_next_type_2fc: 0,
            map_buckets_300: 0,
            unk_324: 0,
            audio_manager_b3c214: false,
            co_initialized: false,
            co_initialize_result: E_FAIL,
            main_menu_music_b3c0ec: 0,
            music_unported_arms: 0,
        });
        this.time_c8[1] = PHASE04_TIME_INFO_TICK;
        this.game_sounds_paused_a4.resume(false);
        this.raw_b4 = SOUND_0B4;
        this.map_buckets_300 = SOUND_MAP_BUCKETS;
        this.audio_manager_b3c214 = true;
        this.byte_a6 = 0;
        if let Some(delta) = ini.find_address_mut(UNDERWATER_DELTA) {
            delta.raw = 0.0f32.to_bits();
        }
        let created = platform.direct_sound_create8(0x006A_BDC7);
        if created.result != S_OK || created.object.is_none() {
            return this;
        }
        this.direct_sound_08 = true;
        if failed(platform.sound_set_cooperative_level(0x006A_BDDF, window, DSSCL_PRIORITY)) {
            return this;
        }
        this.caps_10 = DsCaps {
            size: DSCAPS_SIZE,
            ..DsCaps::default()
        };
        if failed(platform.sound_get_caps(0x006A_BDFE, &mut this.caps_10)) {
            return this;
        }
        this.flags_ac |= Self::CAPS_FLAG;
        let mut stack_caps = DsCaps::default();
        platform.sound_get_caps(0x006A_BE5F, &mut stack_caps);
        let buffer =
            platform.create_sound_buffer(0x006A_BE81, DSBUFFERDESC_SIZE, PRIMARY_BUFFER_FLAGS);
        if failed(buffer.result) {
            return this;
        }
        this.primary_buffer_0c = buffer.object.is_some();
        let listener = platform.query_listener(0x006A_BE9D, IID_DIRECT_SOUND_3D_LISTENER);
        if failed(listener.result) {
            return this;
        }
        this.listener_78 = listener.object.is_some();
        // A32048 FLT_MAX listener position.
        platform.listener_set_position(0x006A_BEC4, f32::MAX, f32::MAX, f32::MAX, DS3D_IMMEDIATE);
        if let Some(rolloff) = ini.find_address_mut(ROLLOFF_FACTOR) {
            let value = rolloff.as_float();
            if ROLLOFF_CEILING < value {
                rolloff.raw = ROLLOFF_CEILING.to_bits();
            }
            if rolloff.as_float() < 0.0 {
                rolloff.raw = 1.0f32.to_bits();
            }
        }
        platform.listener_set_rolloff_factor(
            0x006A_BF21,
            setting_float(ini, ROLLOFF_FACTOR),
            DS3D_IMMEDIATE,
        );
        this.byte_a5 = 0;
        if stack_caps.max_hw_3d_streaming_buffers_34 != 0 {
            this.flags_ac |= Self::HARDWARE_3D_STREAMING_FLAG;
        }
        this.unk_324 = 0;
        this.time_c8[2] = PHASE04_TIME_INFO_TICK;
        this.current_music_type_b0 = 0;
        this.co_initialize_result = platform.co_initialize(0x006A_BF52);
        this.co_initialized = true;
        this.music_flags_dc = 0;
        this.master_volume_b8 = setting_float(ini, MASTER_VOLUME);
        this.music_volume2_2f8 = setting_float(ini, MUSIC_VOLUME);
        this.music_volume_2f0 = this.music_volume2_2f8;
        this.effects_volume_c4 = setting_float(ini, EFFECTS_VOLUME);
        this.foot_volume_bc = setting_float(ini, FOOT_VOLUME);
        this.voice_volume_c0 = setting_float(ini, VOICE_VOLUME);
        this.unk_2f4 = 0.0;
        this.time_c8[0] = PHASE04_TIME_INFO_TICK;
        this
    }

    /// IDA: 006A9B40 OSSoundGlobals_PauseGameSounds (Phase 06, D4): 006A9B46 byte +0xA4 = 1 before the bSoundEnabled test
    /// (00B16178), then a walk of the +0x300 NiTPointerMap<int, TESGameSound*> that marks (0x200) and mutes or stops each
    /// playing sound. The host keeps no TESGameSound instances (Phase 04 substitution), so the walk visits nothing;
    /// P06-RUN-001 read an empty map at every intro. Completion: 100% of the reachable behaviour.
    /// TODO: [ ] Sound phases (carried): the per-instance mute/stop/Play and volume calls (006B6AA0, 006B6AF0, 006B6E60,
    ///       006B6F20, 006B7120, 006B7130) once the game-sound map is ported.
    pub fn pause_game_sounds(&self, sound_enabled: bool) {
        self.game_sounds_paused_a4.pause(sound_enabled);
    }

    /// IDA: 006A9C00 OSSoundGlobals_ResumeGameSounds (Phase 06, D4): 006A9C06 byte +0xA4 = 0 before the bSoundEnabled
    /// test, then the walk re-enables the marked sounds. Completion: 100% of the reachable behaviour; see
    /// `pause_game_sounds`.
    pub fn resume_game_sounds(&self, sound_enabled: bool) {
        self.game_sounds_paused_a4.resume(sound_enabled);
    }

    /// IDA: +0x300 map count (no host instances). Completion: 100% of the host's empty map.
    #[must_use]
    pub const fn game_sound_count(&self) -> u32 {
        0
    }
}

/// IDA: `test eax, eax; jl`: the FAILED() the sound constructor applies.
const fn failed(result: u32) -> bool {
    (result as i32) < 0
}

/// IDA: 00404A00 OSGlobalsData_Constructor (0x28 bytes). 00B33398 g_osGlobals: the resource exists while it is non-null.
/// Completion: 100% of the fields.
/// TODO: [ ] Teardown (carried): 0040F598 clears g_osGlobals at WinMain exit.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct OsGlobalsData {
    pub quit_game_00: bool,
    pub exit_to_main_menu_01: bool,
    /// +0x02 windowActive (Phase 06, D6): written 1 by WinMain 0040EAD7 and WM_ACTIVATE activation, 0 by deactivation;
    /// no reader in the executable (static survey and the P06-RUN-001 watchpoint). Write-only.
    pub window_active_02: u8,
    pub unk_03: u8,
    pub unk_04: u8,
    pub window_08: HostWindow,
    pub process_instance_0c: HostInstance,
    pub main_thread_id_10: u32,
    /// Stays 0: the 00404A62 duplicate fails.
    pub main_thread_handle_14: u32,
    pub unk_18: u32,
    /// Never written (heap bytes).
    pub unk_1c: u32,
    pub input_20: Option<SharedOsInput>,
    pub sound_24: Option<Box<OsSoundGlobals>>,
    /// IDA: N/A host record of 0040E862's result (BOOL).
    pub duplicate_handle_result: u32,
}

/// IDA: 00404540 OSInputGlobals_LoadControlBindings result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlBindingLoad {
    pub collection_opened: bool,
    pub version_matched: bool,
    pub keys_read: u32,
    /// True when a key was read while `input` was null: the original then writes through a null pointer (+0x1B7E) and
    /// faults; the host reports it.
    pub null_input_write: bool,
}

/// IDA: 00404540: strcpy(path, 00B3F280) + strcat("Oblivion.ini") into a char[260], copy it into the INI collection +0x04,
/// vt+0x14(0) (004045D0), vt+0x10 (004045EC) re-reads fVersion:Controls (00B02C4C) and only an exact 1.8f continues; 29
/// GetPrivateProfileStringA("Controls", key, NULL, buf, 64, path) reads (0040462B) parsed with strtol(buf, &end, 16)
/// (0040463D) into keyboard = bits 16..23, mouse = 8..15, joystick = 0..7; vt+0x18. Completion: 100%. 00B3F280 always ends
/// in a backslash; the host directory path does not, so it is a path append.
pub fn load_control_bindings(
    mut input: Option<&mut OsInputGlobals>,
    ini: &mut SettingCollection,
    my_games_root: &Path,
) -> ControlBindingLoad {
    let mut result = ControlBindingLoad::default();
    ini.set_path(my_games_root.join("Oblivion.ini"));
    result.collection_opened = ini.open_ini();
    if !result.collection_opened {
        return result;
    }
    if let Some(mut version) = ini.find_address(CONTROLS_VERSION).cloned() {
        ini.read_setting(&mut version);
        if let Some(slot) = ini.find_address_mut(CONTROLS_VERSION) {
            *slot = version;
        }
    }
    let value = ini
        .find_address(CONTROLS_VERSION)
        .map_or(0.0, |version| f64::from(version.as_float()));
    result.version_matched = value == CONTROLS_VERSION_VALUE;
    if result.version_matched {
        let path = ini.path().to_path_buf();
        for (index, key) in CONTROL_BINDING_KEYS.iter().enumerate() {
            let text = profile::read_string(&path, b"Controls", key.as_bytes(), None, 64);
            if text.is_empty() {
                continue;
            }
            let bits = strtol_hex(&text) as u32;
            result.keys_read += 1;
            let Some(input) = input.as_deref_mut() else {
                result.null_input_write = true;
                continue;
            };
            input.set_binding(0, index, (bits >> 16) as u8);
            input.set_binding(1, index, (bits >> 8) as u8);
            input.set_binding(2, index, bits as u8);
        }
    }
    ini.close();
    result
}

// IDA: 00404A00 (40E826 call): flags, window, instance, thread identity, DuplicateHandle(NULL, GetCurrentThread(), NULL,
// &h, 0, FALSE, 0), "Main", FormHeap(0x1BD8) input, 00404540 bindings without a null check, FormHeap(0x328) sound.
// Completion: 100% of the order; `Ok(None)` on the 0x28 allocation failure (0040E82D stores 0 into g_osGlobals).
// `my_games_root` is 00B3F280.
/// # Errors
/// A site where the original faults: see `OsInputGlobals::new`, and 0040464F for bindings through a null input.
pub fn construct_os_globals(
    window: HostWindow,
    instance: HostInstance,
    settings: &mut SettingCollections,
    my_games_root: &Path,
    platform: &mut dyn OsGlobalsPlatform,
) -> Result<Option<OsGlobalsData>, OsGlobalsError> {
    // 0040E816 FormHeap_Allocate(0x28); null -> 0040E82D eax = 0.
    if !platform.allocate(0x0040_E816, layout::GLOBALS_SIZE) {
        return Ok(None);
    }
    let mut globals = OsGlobalsData {
        // 00404A30..00404A53.
        quit_game_00: false,
        exit_to_main_menu_01: false,
        window_active_02: 0,
        unk_03: 0,
        unk_04: 0,
        unk_18: 0,
        window_08: window,
        process_instance_0c: instance,
        main_thread_id_10: platform.get_current_thread_id(0x0040_4A47),
        main_thread_handle_14: 0,
        unk_1c: 0,
        input_20: None,
        sound_24: None,
        duplicate_handle_result: 1,
    };
    // 00404A62: null source/target process handles make the call fail.
    let current_thread = platform.get_current_thread(0x0040_4A55);
    let (duplicated, handle) =
        platform.duplicate_handle(0x0040_4A62, 0, current_thread, 0, 0, 0, 0);
    globals.duplicate_handle_result = duplicated;
    globals.main_thread_handle_14 = handle;
    // 00404A71 -> 00747830 -> 0074788A RaiseException, the MSVC thread-naming convention, inside __try/__except.
    platform.name_thread(0x0040_4A71, globals.main_thread_id_10, "Main");
    // 00404A7B FormHeap_Allocate(0x1BD8); null -> input 0.
    if platform.allocate(0x0040_4A7B, layout::INPUT_SIZE) {
        globals.input_20 = Some(Shared::new(*OsInputGlobals::new(
            instance, settings, platform,
        )?));
    }
    // 00404AA8: called without a null check.
    let mut input = globals.input_20.as_ref().map(Shared::lock);
    let bindings = load_control_bindings(input.as_deref_mut(), &mut settings.ini, my_games_root);
    drop(input);
    if bindings.null_input_write {
        return Err(OsGlobalsError(
            "OSInputGlobals allocation failed and [Controls] had keys: the original writes through a null pointer at 0040464F"
                .to_owned(),
        ));
    }
    // 00404AB2 FormHeap_Allocate(0x328); null -> sound 0.
    if platform.allocate(0x0040_4AB2, layout::SOUND_SIZE) {
        globals.sound_24 = Some(OsSoundGlobals::new(window, settings, platform));
    }
    Ok(Some(globals))
}

/// IDA: N/A which sound globals 00B3C0F0 names: `OsGlobals` is g_osGlobals +0x24; `Other` is any other non-null object a
/// caller passes, by a host identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundSource {
    OsGlobals,
    Other(u32),
}

/// IDA: Phase 13 (13d): 00B3C0F0 g_soundSourceGlobal (source_118), the sound globals 006AF850 selects (non-owning; null
/// until 00406360 004068F5). Completion: 100% of the slot.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SoundSourceGlobal(pub Option<SoundSource>);

/// IDA: 006AF850 Sound_SetGlobalSource(sound) (__cdecl): a nonnull argument (006AF856) becomes 00B3C0F0; null takes
/// g_osGlobals +0x24 (006AF85E..006AF866; no null test on g_osGlobals: host stop). Completion: 100%.
/// # Errors
/// The original faults: a null argument with a null g_osGlobals.
pub fn set_global_sound_source(
    slot: &mut SoundSourceGlobal,
    sound: Option<SoundSource>,
    os_globals: Option<&OsGlobalsData>,
) -> Result<(), OsGlobalsError> {
    if sound.is_some() {
        slot.0 = sound;
        return Ok(());
    }
    let Some(os_globals) = os_globals else {
        return Err(OsGlobalsError(
            "006AF85E: 006AF850(0) with a null g_osGlobals".to_owned(),
        ));
    };
    slot.0 = os_globals.sound_24.as_ref().map(|_| SoundSource::OsGlobals);
    Ok(())
}

/// IDA: N/A the seam's script, for tests here and in later phases: every result, and the ordered trace of every call.
/// Defaults are the success path of P04-RUN-001.
#[derive(Debug)]
pub struct ScriptedOsPlatform {
    pub trace: Vec<OsTraceEntry>,
    /// Sizes whose FormHeap request returns null.
    pub failed_allocations: Vec<u32>,
    pub thread_id: u32,
    pub active_window: HostWindow,
    pub direct_input: CreatedInterface<()>,
    pub devices: Vec<DeviceInstance>,
    /// Created by GUID; a GUID not listed fails with `E_FAIL` and no object.
    pub creatable: Vec<(Guid, InputDevice)>,
    pub capabilities: DiDevCaps,
    pub objects: Vec<DeviceObjectInstance>,
    pub acquire_result: u32,
    /// Consumed one per GetDeviceState; when empty the buffer stays as it was.
    pub keyboard_states: std::collections::VecDeque<[u8; 256]>,
    pub mouse_states: std::collections::VecDeque<DiMouseState2>,
    pub joystick_states: std::collections::VecDeque<DiJoyState>,
    pub device_data_result: u32,
    pub swap_mouse_result: u32,
    pub double_click_time: u32,
    pub direct_sound: CreatedInterface<()>,
    pub sound_cooperative_level_result: u32,
    pub sound_caps_result: u32,
    /// What the second GetCaps (006ABE5F) writes into its buffer, like the original's stack contents.
    pub stack_caps: Option<DsCaps>,
    pub sound_buffer_result: u32,
    pub listener_result: u32,
    pub co_initialize_result: u32,
}

impl ScriptedOsPlatform {
    pub const KEYBOARD: InputDevice = InputDevice(1);
    pub const MOUSE: InputDevice = InputDevice(2);
}

impl Default for ScriptedOsPlatform {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            trace: Vec::new(),
            failed_allocations: Vec::new(),
            thread_id: 0x14E28,
            active_window: HostWindow(0x0871_1BAC),
            direct_input: CreatedInterface {
                result: S_OK,
                object: Some(()),
            },
            devices: Vec::new(),
            creatable: vec![
                (GUID_SYS_KEYBOARD, Self::KEYBOARD),
                (GUID_SYS_MOUSE, Self::MOUSE),
            ],
            capabilities: DiDevCaps {
                size: DIDEVCAPS_SIZE,
                ..DiDevCaps::default()
            },
            objects: Vec::new(),
            acquire_result: S_OK,
            keyboard_states: std::collections::VecDeque::new(),
            mouse_states: std::collections::VecDeque::new(),
            joystick_states: std::collections::VecDeque::new(),
            device_data_result: DIERR_NOTACQUIRED,
            swap_mouse_result: 0,
            double_click_time: 0x1F4,
            direct_sound: CreatedInterface {
                result: S_OK,
                object: Some(()),
            },
            sound_cooperative_level_result: S_OK,
            sound_caps_result: S_OK,
            stack_caps: None,
            sound_buffer_result: S_OK,
            listener_result: S_OK,
            co_initialize_result: S_OK,
        }
    }
}

/// IDA: N/A the `[Controls]` part of the C++ Phase 06 `InputFixture`: bUse Joystick 1 and the given bBackground Mouse, as
/// raw bytes 00404150 reads. `devices` lists any joystick the platform reports.
#[must_use]
pub fn scripted_input_settings(background_mouse: bool) -> SettingCollections {
    let mut settings = SettingCollections::default();
    super::settings::register_static_settings(&mut settings);
    if let Some(setting) = settings.ini.find_address_mut(USE_JOYSTICK) {
        setting.raw = 1;
    }
    if let Some(setting) = settings.ini.find_address_mut(BACKGROUND_MOUSE) {
        setting.raw = u32::from(background_mouse);
    }
    settings
}

impl ScriptedOsPlatform {
    pub const JOYSTICK: InputDevice = InputDevice(3);

    /// IDA: N/A the C++ `FakeDirectInput`: the keyboard, and optionally the mouse and one joystick reported as
    /// DI8DEVTYPE_JOYSTICK, whose instance GUID has `data1` 0.
    #[must_use]
    pub fn with_devices(with_mouse: bool, with_joystick: bool) -> Self {
        let mut platform = Self {
            creatable: vec![(GUID_SYS_KEYBOARD, Self::KEYBOARD)],
            ..Self::default()
        };
        if with_mouse {
            platform.creatable.push((GUID_SYS_MOUSE, Self::MOUSE));
        }
        if with_joystick {
            let guid = Guid::new(0, 0, 0, [0; 8]);
            platform.creatable.push((guid, Self::JOYSTICK));
            platform.devices.push(DeviceInstance {
                size: 0x244,
                guid_instance: guid,
                dev_type: u32::from(DI8DEVTYPE_JOYSTICK),
                ..DeviceInstance::default()
            });
        }
        platform
    }

    // IDA: N/A a helper, with no address of its own.
    fn record(&mut self, site: u32, call: OsCall) {
        self.trace.push(OsTraceEntry { site, call });
    }
}

impl OsGlobalsPlatform for ScriptedOsPlatform {
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn allocate(&mut self, site: u32, bytes: u32) -> bool {
        self.record(site, OsCall::Allocate { bytes });
        !self.failed_allocations.contains(&bytes)
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_current_thread_id(&mut self, site: u32) -> u32 {
        self.record(site, OsCall::GetCurrentThreadId);
        self.thread_id
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_current_thread(&mut self, site: u32) -> u32 {
        self.record(site, OsCall::GetCurrentThread);
        0xFFFF_FFFE
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn duplicate_handle(
        &mut self,
        site: u32,
        source_process: u32,
        source_handle: u32,
        target_process: u32,
        access: u32,
        inherit: u32,
        options: u32,
    ) -> (u32, u32) {
        self.record(
            site,
            OsCall::DuplicateHandle {
                source_process,
                source_handle,
                target_process,
                access,
                inherit,
                options,
            },
        );
        (0, 0)
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn name_thread(&mut self, site: u32, thread_id: u32, name: &'static str) {
        self.record(site, OsCall::NameThread { thread_id, name });
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_active_window(&mut self, site: u32) -> HostWindow {
        self.record(site, OsCall::GetActiveWindow);
        self.active_window
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn direct_input8_create(
        &mut self,
        site: u32,
        instance: HostInstance,
        version: u32,
    ) -> CreatedInterface<()> {
        self.record(site, OsCall::DirectInput8Create { instance, version });
        self.direct_input
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn enum_devices(&mut self, site: u32, dev_type: u32, flags: u32) -> Vec<DeviceInstance> {
        self.record(site, OsCall::EnumDevices { dev_type, flags });
        self.devices.clone()
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn create_device(&mut self, site: u32, guid: Guid) -> CreatedInterface<InputDevice> {
        self.record(site, OsCall::CreateDevice { guid });
        match self.creatable.iter().find(|(known, _)| *known == guid) {
            Some(&(_, device)) => CreatedInterface {
                result: S_OK,
                object: Some(device),
            },
            None => CreatedInterface {
                result: E_FAIL,
                object: None,
            },
        }
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn set_cooperative_level(
        &mut self,
        site: u32,
        device: InputDevice,
        window: HostWindow,
        level: u32,
    ) -> u32 {
        self.record(
            site,
            OsCall::SetCooperativeLevel {
                device,
                window,
                level,
            },
        );
        S_OK
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn set_data_format(&mut self, site: u32, device: InputDevice, format: DataFormat) -> u32 {
        self.record(site, OsCall::SetDataFormat { device, format });
        S_OK
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn set_property(&mut self, site: u32, device: InputDevice, property: DeviceProperty) -> u32 {
        self.record(site, OsCall::SetProperty { device, property });
        S_OK
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_capabilities(&mut self, site: u32, device: InputDevice, caps: &mut DiDevCaps) -> u32 {
        self.record(site, OsCall::GetCapabilities { device });
        *caps = self.capabilities;
        S_OK
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn enum_objects(
        &mut self,
        site: u32,
        device: InputDevice,
        flags: u32,
    ) -> Vec<DeviceObjectInstance> {
        self.record(site, OsCall::EnumObjects { device, flags });
        self.objects.clone()
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn acquire(&mut self, site: u32, device: InputDevice) -> u32 {
        self.record(site, OsCall::Acquire { device });
        self.acquire_result
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn poll(&mut self, site: u32, device: InputDevice) -> u32 {
        self.record(site, OsCall::Poll { device });
        S_OK
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_keyboard_state(&mut self, site: u32, device: InputDevice, state: &mut [u8; 256]) -> u32 {
        self.record(
            site,
            OsCall::GetDeviceState {
                device,
                size: 0x100,
            },
        );
        if let Some(next) = self.keyboard_states.pop_front() {
            *state = next;
        }
        S_OK
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_mouse_state(
        &mut self,
        site: u32,
        device: InputDevice,
        state: &mut DiMouseState2,
    ) -> u32 {
        self.record(
            site,
            OsCall::GetDeviceState {
                device,
                size: DIMOUSESTATE2_SIZE,
            },
        );
        if let Some(next) = self.mouse_states.pop_front() {
            *state = next;
        }
        S_OK
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_joystick_state(
        &mut self,
        site: u32,
        device: InputDevice,
        state: &mut DiJoyState,
    ) -> u32 {
        self.record(
            site,
            OsCall::GetDeviceState {
                device,
                size: DIJOYSTATE_SIZE,
            },
        );
        if let Some(next) = self.joystick_states.pop_front() {
            *state = next;
        }
        S_OK
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_device_data(
        &mut self,
        site: u32,
        device: InputDevice,
        element_size: u32,
        count: u32,
        flags: u32,
    ) -> u32 {
        self.record(
            site,
            OsCall::GetDeviceData {
                device,
                element_size,
                count,
                flags,
            },
        );
        self.device_data_result
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn swap_mouse_button(&mut self, site: u32, swap: u32) -> u32 {
        self.record(site, OsCall::SwapMouseButton { swap });
        self.swap_mouse_result
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn get_double_click_time(&mut self, site: u32) -> u32 {
        self.record(site, OsCall::GetDoubleClickTime);
        self.double_click_time
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn direct_sound_create8(&mut self, site: u32) -> CreatedInterface<()> {
        self.record(site, OsCall::DirectSoundCreate8);
        self.direct_sound
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn sound_set_cooperative_level(&mut self, site: u32, window: HostWindow, level: u32) -> u32 {
        self.record(site, OsCall::SoundSetCooperativeLevel { window, level });
        self.sound_cooperative_level_result
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn sound_get_caps(&mut self, site: u32, caps: &mut DsCaps) -> u32 {
        self.record(site, OsCall::SoundGetCaps { size: caps.size });
        if self.sound_caps_result != S_OK {
            return self.sound_caps_result;
        }
        if caps.size == DSCAPS_SIZE {
            return S_OK;
        }
        // A wrong dwSize fails; what it leaves is the buffer's contents, which the script stands in for.
        if let Some(stack) = self.stack_caps {
            *caps = stack;
        }
        DSERR_INVALIDPARAM
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn create_sound_buffer(&mut self, site: u32, size: u32, flags: u32) -> CreatedInterface<()> {
        self.record(site, OsCall::CreateSoundBuffer { size, flags });
        CreatedInterface {
            result: self.sound_buffer_result,
            object: (!failed(self.sound_buffer_result)).then_some(()),
        }
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn query_listener(&mut self, site: u32, iid: Guid) -> CreatedInterface<()> {
        self.record(site, OsCall::QueryListener { iid });
        CreatedInterface {
            result: self.listener_result,
            object: (!failed(self.listener_result)).then_some(()),
        }
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn listener_set_position(&mut self, site: u32, x: f32, y: f32, z: f32, apply: u32) -> u32 {
        self.record(
            site,
            OsCall::ListenerSetPosition {
                x_bits: x.to_bits(),
                y_bits: y.to_bits(),
                z_bits: z.to_bits(),
                apply,
            },
        );
        S_OK
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn listener_set_rolloff_factor(&mut self, site: u32, rolloff: f32, apply: u32) -> u32 {
        self.record(
            site,
            OsCall::ListenerSetRolloffFactor {
                rolloff_bits: rolloff.to_bits(),
                apply,
            },
        );
        S_OK
    }
    /// IDA: N/A `ScriptedOsPlatform`'s side of `OsGlobalsPlatform`.
    fn co_initialize(&mut self, site: u32) -> u32 {
        self.record(site, OsCall::CoInitialize);
        self.co_initialize_result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tes::settings::register_static_settings;
    use recomp_core::fixtures;
    use std::path::PathBuf;

    const INSTANCE: HostInstance = HostInstance(0x0040_0000);
    const WINDOW: HostWindow = HostWindow(0x0871_1BAC);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("recomp_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn registered() -> SettingCollections {
        let mut settings = SettingCollections::default();
        register_static_settings(&mut settings);
        settings
    }

    /// `verify_os_globals`: 00404A00/00404540 error contract and bindings (04-C).
    #[test]
    fn verify_os_globals() {
        let mut settings = registered();
        let mut platform = ScriptedOsPlatform {
            failed_allocations: vec![layout::GLOBALS_SIZE],
            ..ScriptedOsPlatform::default()
        };
        assert_eq!(
            construct_os_globals(
                WINDOW,
                INSTANCE,
                &mut settings,
                Path::new(""),
                &mut platform
            ),
            Ok(None),
            "A failed 0x28 allocation must leave g_osGlobals null"
        );

        let temp = TempRoot::new("os_globals_bindings");
        std::fs::write(
            temp.0.join("Oblivion.ini"),
            "[Controls]\nfVersion=1.8000\nForward=0011FF00\n",
        )
        .unwrap();
        let mut platform = ScriptedOsPlatform {
            failed_allocations: vec![layout::INPUT_SIZE, layout::SOUND_SIZE],
            ..ScriptedOsPlatform::default()
        };
        assert!(
            construct_os_globals(WINDOW, INSTANCE, &mut settings, &temp.0, &mut platform).is_err(),
            "Bindings through a null input pointer must be a startup error"
        );

        let old = TempRoot::new("os_globals_old_bindings");
        std::fs::write(
            old.0.join("Oblivion.ini"),
            "[Controls]\nfVersion=1.7000\nForward=0011FF00\n",
        )
        .unwrap();
        let mismatched = load_control_bindings(None, &mut settings.ini, &old.0);
        assert!(mismatched.collection_opened && !mismatched.version_matched);
        assert_eq!(
            mismatched.keys_read, 0,
            "fVersion 1.7 must fail the exact 1.8 gate before any key"
        );
        assert!(!mismatched.null_input_write);
        assert_eq!(CONTROL_BINDING_KEYS.len(), 29);
        assert_eq!(CONTROL_BINDING_KEYS[0], "Forward");
        assert_eq!(CONTROL_BINDING_KEYS[28], "Grab");

        // 0040418B: a failed DirectInput8Create leaves a null interface that the original reads through; the host reports
        // it as a startup error.
        let empty = TempRoot::new("os_globals_no_ini");
        let mut platform = ScriptedOsPlatform {
            failed_allocations: vec![layout::SOUND_SIZE],
            direct_input: CreatedInterface {
                result: E_FAIL,
                object: None,
            },
            ..ScriptedOsPlatform::default()
        };
        assert!(
            construct_os_globals(WINDOW, INSTANCE, &mut settings, &empty.0, &mut platform).is_err(),
            "A failed DirectInput8Create must be a startup error"
        );

        // 006ABDCE..006ABF52: every early return of the sound constructor, then the full path with a CoInitialize failure
        // (the result is kept, no balance).
        let mut sound_with = |script: ScriptedOsPlatform| {
            let mut platform = ScriptedOsPlatform {
                co_initialize_result: RPC_E_CHANGED_MODE,
                ..script
            };
            let sound = OsSoundGlobals::new(HostWindow(0), &mut settings, &mut platform);
            (sound, platform)
        };
        let (sound, _) = sound_with(ScriptedOsPlatform {
            direct_sound: CreatedInterface {
                result: DSERR_NODRIVER,
                object: None,
            },
            ..ScriptedOsPlatform::default()
        });
        assert!(!sound.direct_sound_08 && sound.flags_ac == 0 && !sound.co_initialized);

        let (sound, platform) = sound_with(ScriptedOsPlatform {
            sound_cooperative_level_result: DSERR_INVALIDPARAM,
            ..ScriptedOsPlatform::default()
        });
        assert!(sound.direct_sound_08 && sound.flags_ac == 0 && !sound.co_initialized);
        assert!(platform.trace.iter().any(|entry| matches!(
            entry.call,
            OsCall::SoundSetCooperativeLevel {
                level: DSSCL_PRIORITY,
                ..
            }
        )));

        let (sound, _) = sound_with(ScriptedOsPlatform {
            sound_caps_result: DSERR_INVALIDPARAM,
            ..ScriptedOsPlatform::default()
        });
        assert!(sound.flags_ac == 0 && !sound.primary_buffer_0c && !sound.co_initialized);

        let (sound, platform) = sound_with(ScriptedOsPlatform {
            sound_buffer_result: DSERR_OUTOFMEMORY,
            ..ScriptedOsPlatform::default()
        });
        assert_eq!(sound.flags_ac, OsSoundGlobals::CAPS_FLAG);
        assert!(platform.trace.iter().any(|entry| matches!(
            entry.call,
            OsCall::CreateSoundBuffer {
                size: 0x24,
                flags: 0x91
            }
        )));
        assert!(!sound.primary_buffer_0c && !sound.co_initialized);

        let (sound, _) = sound_with(ScriptedOsPlatform {
            listener_result: E_NOINTERFACE,
            ..ScriptedOsPlatform::default()
        });
        assert!(sound.primary_buffer_0c && !sound.listener_78 && !sound.co_initialized);

        let (sound, platform) = sound_with(ScriptedOsPlatform::default());
        assert!(sound.listener_78 && sound.co_initialized);
        assert_eq!(sound.co_initialize_result, RPC_E_CHANGED_MODE);
        let positions: Vec<_> = platform
            .trace
            .iter()
            .filter_map(|entry| match entry.call {
                OsCall::ListenerSetPosition { x_bits, .. } => Some(f32::from_bits(x_bits)),
                _ => None,
            })
            .collect();
        assert_eq!(positions, [f32::MAX], "one SetPosition, at FLT_MAX");
        let rolloff = platform
            .trace
            .iter()
            .find_map(|entry| match entry.call {
                OsCall::ListenerSetRolloffFactor { rolloff_bits, .. } => {
                    Some(f32::from_bits(rolloff_bits))
                }
                _ => None,
            })
            .unwrap();
        assert!((0.0..=10.0).contains(&rolloff));
    }

    /// The hex value in a `field=` of a row.
    fn row_value<'a>(row: &'a str, field: &str) -> &'a str {
        row.split_whitespace()
            .find_map(|part| part.strip_prefix(field))
            .unwrap_or_else(|| panic!("{field} in {row}"))
    }

    /// The `a=` list of a row, as numbers.
    fn row_args(row: &str) -> Vec<u32> {
        row_value(row, "a=")
            .split(',')
            .map(|word| u32::from_str_radix(word, 16).unwrap())
            .collect()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// The original address in a row, its third field. A row of another shape has none.
    fn row_site(row: &str) -> Option<u32> {
        u32::from_str_radix(row.split_whitespace().nth(2)?, 16).ok()
    }

    /// The `P04-RUN-001` rows at the sites this block owns, after a check that each is a verbatim line of the run's
    /// records. Other blocks of `phase04.rows` are theirs.
    fn run_001_rows() -> Vec<String> {
        let path = fixtures::root().join("evidence/phase04-run001-2026-10-05.records.txt");
        let evidence = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = evidence.lines().collect();
        let rows: Vec<String> = fixtures::rows("phase04")
            .iter()
            .filter_map(|row| row.strip_prefix("P04-RUN-001 "))
            .filter(|row| {
                row_site(row).is_some_and(|site| {
                    (0x0040_3000..0x0040_5000).contains(&site)
                        || (0x006A_B000..0x006A_C000).contains(&site)
                })
            })
            .map(str::to_owned)
            .collect();
        assert!(!rows.is_empty());
        for row in &rows {
            assert!(lines.contains(&row.as_str()), "not in the records: {row}");
        }
        rows
    }

    /// P04-RUN-001 with the pinned INI: the OS globals' calls, their order and the arguments the records show. Handles and
    /// pointers are not compared. Rows are `C` for a call and `R` for its result.
    #[test]
    fn construction_matches_run_001() {
        let rows = run_001_rows();
        let mut settings = registered();
        settings
            .ini
            .set_path(fixtures::root().join("config/Oblivion.ini"));
        settings.ini.open_ini();
        settings.ini.read_all();
        settings.ini.close();
        let mut platform = ScriptedOsPlatform::default();
        let root = fixtures::root().join("config");
        let globals = construct_os_globals(WINDOW, INSTANCE, &mut settings, &root, &mut platform)
            .unwrap()
            .unwrap();

        // The 0x28 request's own call is not among the run's probes.
        assert_eq!(
            platform.trace[0],
            OsTraceEntry {
                site: 0x0040_E816,
                call: OsCall::Allocate { bytes: 0x28 }
            }
        );
        let calls: Vec<&String> = rows.iter().filter(|row| row.starts_with("C ")).collect();
        let trace = &platform.trace[1..];
        assert_eq!(calls.len(), trace.len());
        for (row, entry) in calls.iter().zip(trace) {
            assert_eq!(row_site(row), Some(entry.site), "{row}");
            let args = row_args(row);
            match &entry.call {
                OsCall::Allocate { bytes } => assert_eq!(args[0], *bytes, "{row}"),
                OsCall::DuplicateHandle {
                    source_process,
                    source_handle,
                    target_process,
                    access,
                    inherit,
                    options,
                } => {
                    let expected = [*source_process, *source_handle, *target_process];
                    assert_eq!(args[..3], expected, "{row}");
                    assert_eq!(args[4..7], [*access, *inherit, *options], "{row}");
                }
                OsCall::DirectInput8Create { instance, version } => {
                    assert_eq!(args[..2], [instance.0, *version], "{row}");
                }
                OsCall::EnumDevices { dev_type, flags } => {
                    assert_eq!((args[1], args[4]), (*dev_type, *flags), "{row}");
                }
                OsCall::CreateDevice { guid } => {
                    assert_eq!(row_value(row, "d1="), hex(&guid.to_bytes()), "{row}");
                }
                OsCall::SetCooperativeLevel { level, .. }
                | OsCall::SoundSetCooperativeLevel { level, .. } => {
                    assert_eq!(args[2], *level, "{row}");
                }
                OsCall::SetDataFormat { format, .. } => assert_eq!(args[1], format.0, "{row}"),
                OsCall::SetProperty {
                    property: DeviceProperty::Dword { property, data },
                    ..
                } => {
                    assert_eq!(args[1], *property, "{row}");
                    let mut bytes = Vec::new();
                    for word in [0x14, 0x10, 0, DIPH_DEVICE, *data] {
                        bytes.extend_from_slice(&u32::to_le_bytes(word));
                    }
                    assert_eq!(row_value(row, "d2="), hex(&bytes), "{row}");
                }
                OsCall::SwapMouseButton { swap } => assert_eq!(args[0], *swap, "{row}"),
                OsCall::CreateSoundBuffer { size, flags } => {
                    let mut bytes = Vec::new();
                    bytes.extend_from_slice(&size.to_le_bytes());
                    bytes.extend_from_slice(&flags.to_le_bytes());
                    assert!(row_value(row, "d1=").starts_with(&hex(&bytes)), "{row}");
                }
                OsCall::QueryListener { iid } => {
                    assert_eq!(row_value(row, "d1="), hex(&iid.to_bytes()), "{row}");
                }
                OsCall::ListenerSetPosition {
                    x_bits,
                    y_bits,
                    z_bits,
                    apply,
                } => assert_eq!(args[1..5], [*x_bits, *y_bits, *z_bits, *apply], "{row}"),
                OsCall::ListenerSetRolloffFactor {
                    rolloff_bits,
                    apply,
                } => assert_eq!(args[1..3], [*rolloff_bits, *apply], "{row}"),
                OsCall::NameThread { .. }
                | OsCall::GetCurrentThreadId
                | OsCall::GetCurrentThread
                | OsCall::GetActiveWindow
                | OsCall::GetCapabilities { .. }
                | OsCall::GetDoubleClickTime
                | OsCall::DirectSoundCreate8
                | OsCall::SoundGetCaps { .. }
                | OsCall::CoInitialize => {}
                other => panic!("no row expected this call: {other:?}"),
            }
        }

        // R 34 404A68: the duplicate fails and the handle stays 0.
        assert_eq!(globals.duplicate_handle_result, 0);
        assert_eq!(globals.main_thread_handle_14, 0);
        let input = globals.input_20.as_ref().unwrap().lock();
        // bBackground Mouse=1 in the pinned INI: non-exclusive, as the run's level 6 shows; no joystick.
        assert_eq!(
            input.flags_0000,
            OsInputGlobals::MOUSE_FLAG
                | OsInputGlobals::KEYBOARD_FLAG
                | OsInputGlobals::BACKGROUND_MOUSE_FLAG
        );
        assert!(!input.mouse_exclusive);
        // R 68 40425F.
        assert_eq!(input.double_click_time_1b4c, 0x1F4);

        // The 29 strtol results of 0040463D against the bindings they fill.
        let values: Vec<u32> = rows
            .iter()
            .filter(|row| row.starts_with("R ") && row.contains(" 404642 "))
            .map(|row| u32::from_str_radix(row_value(row, "eax="), 16).unwrap())
            .collect();
        assert_eq!(values.len(), 29);
        for (index, value) in values.iter().enumerate() {
            assert_eq!(
                input.binding(0, index),
                (value >> 16) as u8,
                "keyboard {index}"
            );
            assert_eq!(input.binding(1, index), (value >> 8) as u8, "mouse {index}");
            assert_eq!(input.binding(2, index), *value as u8, "joystick {index}");
        }

        // The rolloff the run's SetRolloffFactor passed comes from the pinned INI, and the sound state from its values.
        let sound = globals.sound_24.as_ref().unwrap();
        assert!(sound.listener_78 && sound.co_initialized);
        // The zeroed 006ABE5F buffer: see OsSoundGlobals::new. The original's stack bytes gave 5.
        assert_eq!(sound.flags_ac, OsSoundGlobals::CAPS_FLAG);
        assert_eq!(sound.master_volume_b8, 1.0);
        assert_eq!(sound.music_volume_2f0, 0.3);
        assert_eq!(sound.music_volume2_2f8, 0.3);
        assert_eq!(sound.effects_volume_c4, 0.8);
    }

    /// R 254 6ABE61: the original's stack buffer at 006ABE5F. With its +0x34 the branch sets bit 2, as the run's did.
    #[test]
    fn stack_caps_branch_is_reachable() {
        let rows = run_001_rows();
        let row = rows
            .iter()
            .find(|row| row.starts_with("R ") && row.contains(" 6ABE61 "))
            .unwrap();
        assert_eq!(
            u32::from_str_radix(row_value(row, "eax="), 16),
            Ok(DSERR_INVALIDPARAM)
        );
        let bytes = row_value(row, "d1=");
        // Little-endian dword at +0x34: two hex digits a byte.
        let word = &bytes[0x34 * 2..0x38 * 2];
        let raw: Vec<u8> = (0..4)
            .map(|index| u8::from_str_radix(&word[index * 2..index * 2 + 2], 16).unwrap())
            .collect();
        let value = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        assert_ne!(value, 0);
        let mut settings = registered();
        let mut platform = ScriptedOsPlatform {
            stack_caps: Some(DsCaps {
                max_hw_3d_streaming_buffers_34: value,
                ..DsCaps::default()
            }),
            ..ScriptedOsPlatform::default()
        };
        let sound = OsSoundGlobals::new(WINDOW, &mut settings, &mut platform);
        assert_eq!(
            sound.flags_ac,
            OsSoundGlobals::CAPS_FLAG | OsSoundGlobals::HARDWARE_3D_STREAMING_FLAG
        );
    }

    fn joystick_instance(guid: Guid, dev_type: u32) -> DeviceInstance {
        DeviceInstance {
            size: 0x244,
            guid_instance: guid,
            dev_type,
            ..DeviceInstance::default()
        }
    }

    const JOYSTICK_GUID: Guid = Guid::new(1, 2, 3, [4; 8]);

    fn joystick_platform() -> ScriptedOsPlatform {
        let mut platform = ScriptedOsPlatform::default();
        platform.creatable.push((JOYSTICK_GUID, InputDevice(0x10)));
        platform.devices = vec![
            joystick_instance(GUID_SYS_MOUSE, 0x112),
            joystick_instance(JOYSTICK_GUID, 0x0114),
        ];
        platform.objects = vec![
            DeviceObjectInstance {
                guid_type: GUID_X_AXIS,
                object_type: 2,
            },
            DeviceObjectInstance {
                guid_type: GUID_RZ_AXIS,
                object_type: 2,
            },
            DeviceObjectInstance {
                guid_type: Guid::default(),
                object_type: 0x0304,
            },
        ];
        platform
    }

    /// 00403DD0, 004035D0, 004030F0 and the bytes 00403960 writes for a joystick: 00403AB5..00403B38 read off the
    /// executable, since no run attached one and the C++ test has no such check.
    #[test]
    fn joystick_enumeration_and_default_bindings() {
        let settings = registered();
        let mut platform = joystick_platform();
        let input = OsInputGlobals::new(INSTANCE, &settings, &mut platform).unwrap();
        assert_eq!(input.joystick_count_18f0, 1);
        assert_ne!(input.flags_0000 & OsInputGlobals::JOYSTICK_FLAG, 0);
        assert_eq!(input.joysticks_0008[0], Some(InputDevice(0x10)));
        assert_eq!(
            input.joystick_instances_0530[0].guid_instance,
            JOYSTICK_GUID
        );
        assert_eq!(input.joystick_objects_18b0[0].axes, 0x21);
        assert_eq!(input.joystick_objects_18b0[0].buttons, 1 << 3);
        let sites: Vec<u32> = platform
            .trace
            .iter()
            .map(|entry| entry.site)
            .filter(|site| (0x0040_3000..0x0040_4000).contains(site))
            .collect();
        assert_eq!(
            sites,
            [
                0x0040_3E26,
                0x0040_3E33,
                0x0040_3E3D,
                0x0040_3E4A,
                0x0040_3E93,
                0x0040_3EC9,
                0x0040_3F14,
                0x0040_3150
            ]
        );
        assert!(platform.trace.iter().any(|entry| entry.call
            == OsCall::SetProperty {
                device: InputDevice(0x10),
                property: DeviceProperty::Dword {
                    property: DIPROP_DEADZONE,
                    data: 1000
                }
            }));

        let at = |offset: usize| input.bindings_1b7e[offset - 0x1B7E];
        for (offset, value) in [
            (0x1BC1, 0),
            (0x1BC5, 1),
            (0x1BC0, 2),
            (0x1BBD, 3),
            (0x1BC7, 4),
            (0x1BC6, 5),
            (0x1BBF, 6),
            (0x1BD4, 7),
            (0x1BBE, 8),
            (0x1BBC, 9),
            (0x1BC8, 10),
            (0x1BD5, 11),
            (0x1BCA, 0x20),
            (0x1BCB, 0x21),
            (0x1BCC, 0x22),
            (0x1BCD, 0x23),
            (0x1BCE, 0x24),
            (0x1BCF, 0x25),
            (0x1BD0, 0x26),
            (0x1BD1, 0x27),
        ] {
            assert_eq!(at(offset), value, "{offset:X}");
        }
        for offset in [
            0x1BB8, 0x1BB9, 0x1BBA, 0x1BBB, 0x1BC2, 0x1BC3, 0x1BC4, 0x1BC9, 0x1BD2, 0x1BD3,
        ] {
            assert_eq!(at(offset), 0xFF, "{offset:X}");
        }
        // Zeroed host bytes past the table, apart from the one store: heap bytes in the original.
        assert_eq!(at(0x1BD6), 0);

        // Without the joystick flag the joystick table stays reset.
        let mut plain =
            OsInputGlobals::new(INSTANCE, &settings, &mut ScriptedOsPlatform::default()).unwrap();
        assert_eq!(plain.binding(2, 9), 0xFF);
        // A one-table reset leaves the others.
        plain.set_binding(1, 4, 0x55);
        plain.reset_bindings(0);
        assert_eq!(plain.binding(1, 4), 0x55);
        plain.reset_bindings(1);
        assert_eq!(plain.binding(1, 4), 0);
        assert_eq!(plain.binding(1, 5), 0xFF);
    }

    /// 004046A0's three devices and both Acquire branches, 00403C30's double click, and the queries. Phase 06 extends these.
    #[test]
    fn poll_and_queries() {
        let settings = registered();
        let mut platform = joystick_platform();
        let mut input = OsInputGlobals::new(INSTANCE, &settings, &mut platform).unwrap();
        input.acquire_exclusive_mouse = true;
        let mut joystick = DiJoyState::default();
        joystick.buttons[3] = 0x80;
        joystick.pov[0] = 9000;
        platform.joystick_states.push_back(joystick);
        let mut keyboard = [0; 256];
        keyboard[DIK_ESCAPE as usize] = 0x80;
        keyboard[0x11] = 0x80;
        platform.keyboard_states.push_back(keyboard);
        let mut mouse = DiMouseState2::default();
        mouse.buttons[0] = 0x80;
        mouse.z = 5;
        platform.mouse_states.push_back(mouse);
        platform.trace.clear();
        input.poll_devices(1000, &mut platform);
        let sites: Vec<u32> = platform.trace.iter().map(|entry| entry.site).collect();
        assert_eq!(
            sites,
            [
                0x0040_46DC,
                0x0040_4702,
                0x0040_470F,
                0x0040_4751,
                0x0040_4777,
                0x0040_4788,
                0x0040_47EA,
                0x0040_481C,
                0x0040_482A
            ]
        );
        assert_eq!(
            input.query_scan_code_state(0x11, OsInputGlobals::QUERY_PRESSED),
            1
        );
        assert_eq!(input.query_scan_code_state(0x11, 7), 0);
        // Forward: the keyboard's W; the mouse and joystick schemes of control 0 are unbound or idle.
        assert_eq!(input.query_control_state(0, OsInputGlobals::QUERY_HELD), 1);
        assert_eq!(input.query_control_state(29, 99), 1);
        assert_eq!(input.query_control_state(30, 99), 0);
        assert_eq!(input.query_control_state(-1, 0), 0);
        assert_eq!(input.query_mouse_button_state(8, 0), 1);
        assert_eq!(input.query_mouse_button_state(9, 0), 0);
        assert_eq!(
            input.query_mouse_button_state(0, OsInputGlobals::QUERY_HELD),
            1
        );
        assert_eq!(input.query_mouse_button_state(200, 0), 0);
        assert_eq!(input.last_click_times_1b58[0], 1000);
        assert_eq!(
            input.query_mouse_button_state(0, OsInputGlobals::QUERY_DOUBLE_CLICK),
            0
        );
        assert_eq!(
            input.query_joystick_button_state(0, 3, OsInputGlobals::QUERY_RELEASED),
            0
        );
        assert_eq!(input.query_input_state(2, 3, OsInputGlobals::QUERY_HELD), 1);
        assert_eq!(input.query_input_state(3, 3, OsInputGlobals::QUERY_HELD), 0);
        assert_eq!(input.query_input_state(0xFF, 3, 0), 0);
        assert_eq!(input.joystick_pov_direction(0, 0), 9000 / 4500 + 0x20);
        assert_eq!(input.query_input_state(2, 0x22, 0), 1);
        input.joystick_states_0030[0].current.pov[0] = 0x0001_FFFF;
        assert_eq!(input.joystick_pov_direction(0, 0), -1);

        // A second press within the time: a double click, the time cleared. The button is released first.
        platform.mouse_states.push_back(DiMouseState2::default());
        input.poll_devices(1100, &mut platform);
        platform.mouse_states.push_back(mouse);
        input.poll_devices(1200, &mut platform);
        assert_eq!(
            input.query_mouse_button_state(0, OsInputGlobals::QUERY_DOUBLE_CLICK),
            1
        );
        assert_eq!(input.last_click_times_1b58[0], 0);
        // Swapped primary buttons read the other byte.
        input.swap_mouse_buttons_1b48 = 1;
        assert_eq!(
            input.query_mouse_button_state(1, OsInputGlobals::QUERY_HELD),
            1
        );
        assert_eq!(
            input.query_mouse_button_state(0, OsInputGlobals::QUERY_HELD),
            0
        );

        // A failed Acquire zeroes each state after the previous copy, and skips Poll and GetDeviceState. S_FALSE continues.
        platform.acquire_result = DIERR_NOTACQUIRED;
        platform.trace.clear();
        input.poll_devices(1300, &mut platform);
        assert_eq!(platform.trace.len(), 3);
        assert_eq!(input.keyboard_state_18f4, [0; 256]);
        assert_eq!(input.previous_keyboard_state_19f4[0x11], 0x80);
        assert_eq!(input.mouse_state_1b20, DiMouseState2::default());
        assert_eq!(input.previous_mouse_state_1b34.buttons[0], 0x80);
        platform.acquire_result = S_FALSE;
        platform.trace.clear();
        input.poll_devices(1400, &mut platform);
        assert_eq!(platform.trace.len(), 9);

        // D3: an exclusive mouse skips Acquire, and the failed branch runs, unless a fixture enables it.
        let mut exclusive_settings = registered();
        exclusive_settings
            .ini
            .find_address_mut(BACKGROUND_MOUSE)
            .unwrap()
            .raw = 0;
        let mut platform = ScriptedOsPlatform::default();
        let mut input = OsInputGlobals::new(INSTANCE, &exclusive_settings, &mut platform).unwrap();
        assert!(input.mouse_exclusive);
        assert!(platform.trace.iter().any(|entry| matches!(
            entry.call,
            OsCall::SetCooperativeLevel {
                device: ScriptedOsPlatform::MOUSE,
                level: 5,
                ..
            }
        )));
        platform.trace.clear();
        input.poll_devices(0, &mut platform);
        assert!(platform.trace.iter().all(|entry| entry.site < 0x0040_47EA));
    }

    /// `verify_startup_paths`-independent checks of the other small functions in the file.
    #[test]
    fn discard_pause_and_sound_source() {
        // No input: nothing runs.
        let mut platform = ScriptedOsPlatform::default();
        assert_eq!(
            discard_buffered_input_events(None, &mut platform),
            InputFlushResult::default()
        );
        assert!(platform.trace.is_empty());
        let settings = registered();
        let input = OsInputGlobals::new(INSTANCE, &settings, &mut platform).unwrap();
        platform.trace.clear();
        // P05-RUN-001's result, ignored by the original.
        assert_eq!(
            discard_buffered_input_events(Some(&input), &mut platform),
            InputFlushResult {
                called: true,
                device: true,
                result: DIERR_NOTACQUIRED
            }
        );
        assert_eq!(
            platform.trace,
            [OsTraceEntry {
                site: 0x0040_3183,
                call: OsCall::GetDeviceData {
                    device: ScriptedOsPlatform::KEYBOARD,
                    element_size: 20,
                    count: 0xFFFF_FFFF,
                    flags: 0
                }
            }]
        );
        // No keyboard: 00403165 skips the call.
        let mut no_keyboard = input.clone();
        no_keyboard.keyboard_0028 = None;
        platform.trace.clear();
        assert_eq!(
            discard_buffered_input_events(Some(&no_keyboard), &mut platform),
            InputFlushResult {
                called: true,
                device: false,
                result: S_OK
            }
        );
        assert!(platform.trace.is_empty());

        let mut settings = registered();
        let sound = OsSoundGlobals::new(WINDOW, &mut settings, &mut platform);
        sound.pause_game_sounds(true);
        assert_eq!(sound.game_sounds_paused_a4.load(), 1);
        sound.resume_game_sounds(false);
        assert_eq!(sound.game_sounds_paused_a4.load(), 0);
        assert_eq!(sound.game_sound_count(), 0);

        let mut slot = SoundSourceGlobal::default();
        assert!(set_global_sound_source(&mut slot, None, None).is_err());
        assert_eq!(slot.0, None);
        set_global_sound_source(&mut slot, Some(SoundSource::Other(9)), None).unwrap();
        assert_eq!(slot.0, Some(SoundSource::Other(9)));
        let mut globals = OsGlobalsData::default();
        set_global_sound_source(&mut slot, None, Some(&globals)).unwrap();
        assert_eq!(slot.0, None);
        globals.sound_24 = Some(sound);
        set_global_sound_source(&mut slot, None, Some(&globals)).unwrap();
        assert_eq!(slot.0, Some(SoundSource::OsGlobals));
    }

    // Phase 06, C++ phase06_tests.cpp. The C++ fake devices keep their state between polls; the script's queue is consumed
    // per GetDeviceState and an empty queue leaves the buffer, so a test pushes a state where the C++ changes it.

    const DIK_SPACE: u8 = 0x39;
    const DIK_A: u8 = 0x1E;
    const DIERR_INPUTLOST: u32 = 0x8007_001E;
    const DIERR_OTHERAPPHASPRIO: u32 = 0x8007_0005;

    fn phase06_input(
        background_mouse: bool,
        with_mouse: bool,
        with_joystick: bool,
    ) -> (Box<OsInputGlobals>, ScriptedOsPlatform) {
        let settings = scripted_input_settings(background_mouse);
        let mut platform = ScriptedOsPlatform::with_devices(with_mouse, with_joystick);
        let input = OsInputGlobals::new(INSTANCE, &settings, &mut platform).unwrap();
        (input, platform)
    }

    fn keyboard_bytes(down: &[u8]) -> [u8; 256] {
        let mut bytes = [0; 256];
        for &scan in down {
            bytes[scan as usize] = 0x80;
        }
        bytes
    }

    fn mouse_bytes(z: i32, buttons: &[usize]) -> DiMouseState2 {
        let mut state = DiMouseState2 {
            x: 3,
            y: -4,
            z,
            ..DiMouseState2::default()
        };
        for &button in buttons {
            state.buttons[button] = 0x80;
        }
        state
    }

    fn joystick_bytes(pov: u32, buttons: &[usize]) -> DiJoyState {
        let mut state = DiJoyState::default();
        state.axes[0] = 7;
        state.pov[0] = pov;
        for &button in buttons {
            state.buttons[button] = 0x80;
        }
        state
    }

    /// `tag.Call` of each device call in the trace, as the C++ fakes log them.
    fn device_log(platform: &ScriptedOsPlatform) -> Vec<String> {
        let tag = |device: InputDevice| match device {
            ScriptedOsPlatform::KEYBOARD => "keyboard",
            ScriptedOsPlatform::MOUSE => "mouse",
            ScriptedOsPlatform::JOYSTICK => "joystick",
            _ => "unknown",
        };
        platform
            .trace
            .iter()
            .filter_map(|entry| match entry.call {
                OsCall::Acquire { device } => Some(format!("{}.Acquire", tag(device))),
                OsCall::Poll { device } => Some(format!("{}.Poll", tag(device))),
                OsCall::GetDeviceState { device, .. } => {
                    Some(format!("{}.GetDeviceState", tag(device)))
                }
                OsCall::GetDeviceData { device, .. } => {
                    Some(format!("{}.GetDeviceData", tag(device)))
                }
                _ => None,
            })
            .collect()
    }

    /// C++ verify_poll_order_and_branches: 004046A0 call order, sizes and the S_OK/S_FALSE/error branches.
    #[test]
    fn verify_poll_order_and_branches() {
        let (mut input, mut platform) = phase06_input(true, true, true);
        assert_eq!(input.joystick_count_18f0, 1);
        assert!(!input.mouse_exclusive);
        assert!(platform.trace.iter().any(|entry| matches!(
            entry.call,
            OsCall::SetCooperativeLevel {
                device: ScriptedOsPlatform::MOUSE,
                level,
                ..
            } if level == DISCL_NONEXCLUSIVE | DISCL_FOREGROUND
        )));
        platform
            .keyboard_states
            .push_back(keyboard_bytes(&[DIK_SPACE]));
        platform.mouse_states.push_back(mouse_bytes(0, &[0]));
        platform
            .joystick_states
            .push_back(joystick_bytes(0xFFFF_FFFF, &[1]));
        platform.trace.clear();
        input.poll_devices(1000, &mut platform);
        assert_eq!(
            device_log(&platform),
            [
                "joystick.Acquire",
                "joystick.Poll",
                "joystick.GetDeviceState",
                "keyboard.Acquire",
                "keyboard.Poll",
                "keyboard.GetDeviceState",
                "mouse.Acquire",
                "mouse.Poll",
                "mouse.GetDeviceState"
            ],
            "004046A0 device call order"
        );
        let sizes: Vec<u32> = platform
            .trace
            .iter()
            .filter_map(|entry| match entry.call {
                OsCall::GetDeviceState { size, .. } => Some(size),
                _ => None,
            })
            .collect();
        assert_eq!(
            sizes,
            [0x50, 0x100, 0x14],
            "GetDeviceState sizes must be 0x50, 0x100 and 0x14"
        );
        assert_eq!(input.keyboard_state_18f4[DIK_SPACE as usize], 0x80);
        assert_eq!(input.joystick_states_0030[0].current.buttons[1], 0x80);
        assert_eq!(input.mouse_state_1b20.buttons[0], 0x80);
        assert_eq!(input.mouse_state_1b20.y, -4);
        // New press of button 0 at 1000: no previous click inside 500 ms.
        assert_eq!(input.double_clicks_1b50[0], 0);
        assert_eq!(
            input.last_click_times_1b58[0], 1000,
            "A first press must store TimeInfo+0x10 into +0x1B58"
        );

        // S_FALSE (already acquired) continues; the previous states are copied.
        platform.acquire_result = S_FALSE;
        platform.keyboard_states.push_back(keyboard_bytes(&[]));
        input.poll_devices(1100, &mut platform);
        assert_eq!(input.previous_keyboard_state_19f4[DIK_SPACE as usize], 0x80);
        assert_eq!(input.keyboard_state_18f4[DIK_SPACE as usize], 0);
        assert_eq!(input.previous_mouse_state_1b34.buttons[0], 0x80);
        assert_eq!(input.joystick_states_0030[0].previous.buttons[1], 0x80);

        // Any other HRESULT zeroes the state without Poll/GetDeviceState.
        for failure in [DIERR_INPUTLOST, DIERR_OTHERAPPHASPRIO, 2] {
            platform.acquire_result = failure;
            platform.keyboard_states.clear();
            platform.keyboard_states.push_back(keyboard_bytes(&[DIK_A]));
            platform.trace.clear();
            input.poll_devices(1200, &mut platform);
            assert_eq!(
                device_log(&platform),
                ["joystick.Acquire", "keyboard.Acquire", "mouse.Acquire"],
                "A failed Acquire must skip Poll/GetDeviceState"
            );
            assert_eq!(input.keyboard_state_18f4[DIK_A as usize], 0);
            assert_eq!(input.mouse_state_1b20.x, 0);
            assert_eq!(input.joystick_states_0030[0].current.axes[0], 0);
        }
    }

    /// C++ verify_double_click: 00403C30: the gap test against +0x1B4C (GetDoubleClickTime) is unsigned and inclusive; the
    /// flags are cleared at the start of each mouse poll.
    #[test]
    fn verify_double_click() {
        let (mut input, mut platform) = phase06_input(true, true, false);
        let window = input.double_click_time_1b4c;
        platform.mouse_states.push_back(mouse_bytes(0, &[2]));
        input.poll_devices(10000, &mut platform);
        assert_eq!(
            input.last_click_times_1b58[2], 10000,
            "First press stores the time"
        );
        assert_eq!(input.double_clicks_1b50[2], 0);
        platform.mouse_states.push_back(mouse_bytes(0, &[]));
        input.poll_devices(10100, &mut platform);
        platform.mouse_states.push_back(mouse_bytes(0, &[2]));
        input.poll_devices(10000 + window, &mut platform);
        assert_eq!(
            input.double_clicks_1b50[2], 1,
            "A press within +0x1B4C (inclusive) must set the double-click byte"
        );
        assert_eq!(input.last_click_times_1b58[2], 0);
        input.poll_devices(10000 + window + 1, &mut platform);
        assert_eq!(
            input.double_clicks_1b50[2], 0,
            "The double-click bytes clear on the next poll"
        );
        platform.mouse_states.push_back(mouse_bytes(0, &[]));
        input.poll_devices(20000, &mut platform);
        platform.mouse_states.push_back(mouse_bytes(0, &[2]));
        input.poll_devices(20000 + 1, &mut platform);
        assert_eq!(
            input.double_clicks_1b50[2], 0,
            "After a double click the cleared time makes the next press a first press"
        );
        assert_eq!(input.last_click_times_1b58[2], 20001);
        platform.mouse_states.push_back(mouse_bytes(0, &[]));
        input.poll_devices(20002, &mut platform);
        platform.mouse_states.push_back(mouse_bytes(0, &[2]));
        input.poll_devices(20001 + window + 1, &mut platform);
        assert_eq!(
            input.double_clicks_1b50[2], 0,
            "A press outside +0x1B4C must store the new time"
        );
        assert_eq!(input.last_click_times_1b58[2], 20001 + window + 1);
        // A held button is not a new press.
        input.poll_devices(20001 + window + 2, &mut platform);
        assert_eq!(
            input.last_click_times_1b58[2],
            20001 + window + 1,
            "Held buttons must not restamp"
        );
    }

    /// C++ verify_exclusive_mouse_and_missing_devices. D3 substitution: an exclusive mouse (level 5) is not acquired; the
    /// failed branch runs. Missing devices skip their blocks.
    #[test]
    fn verify_exclusive_mouse_and_missing_devices() {
        let (mut input, mut platform) = phase06_input(false, true, false);
        assert!(input.mouse_exclusive);
        assert!(platform.trace.iter().any(|entry| matches!(
            entry.call,
            OsCall::SetCooperativeLevel {
                device: ScriptedOsPlatform::MOUSE,
                level,
                ..
            } if level == DISCL_EXCLUSIVE | DISCL_FOREGROUND
        )));
        platform.mouse_states.push_back(mouse_bytes(1, &[0]));
        platform.trace.clear();
        input.poll_devices(5, &mut platform);
        let log = device_log(&platform);
        assert!(
            log.iter().all(|call| !call.starts_with("mouse.")),
            "The exclusive mouse must stay unacquired (D3)"
        );
        assert_eq!(input.mouse_state_1b20.z, 0, "and take the zero branch");
        input.acquire_exclusive_mouse = true;
        platform.trace.clear();
        input.poll_devices(6, &mut platform);
        let log = device_log(&platform);
        assert_eq!(
            log.iter().filter(|call| *call == "mouse.Acquire").count(),
            1,
            "With the policy enabled the exclusive mouse is acquired exactly"
        );
        assert_eq!(input.mouse_state_1b20.z, 1);

        let (mut keyboard_only, mut platform) = phase06_input(true, false, false);
        platform.trace.clear();
        keyboard_only.poll_devices(7, &mut platform);
        assert_eq!(keyboard_only.joystick_count_18f0, 0);
        assert!(keyboard_only.mouse_002c.is_none());
        assert_eq!(
            device_log(&platform),
            [
                "keyboard.Acquire",
                "keyboard.Poll",
                "keyboard.GetDeviceState"
            ],
            "Missing joysticks and mouse must skip their blocks"
        );
    }

    /// C++ verify_queries: 00403520/00403490/004032D0/004031E0/00402FC0/00403070.
    #[test]
    fn verify_queries() {
        type I = OsInputGlobals;
        let (mut input, mut platform) = phase06_input(true, true, true);
        // Defaults of 00403960: Activate (5) = Space; mouse Use (4) = 0. Dropped from the C++: `binding(2, 5) == 0`. That is
        // the C++ table, 4 entries too low: 00403AB5 stores 0 at +0x1BC1, control 9, and +0x1BBD, control 5, gets 3 (see
        // `reset_bindings`). So the joystick press below is the button the disassembly binds, and the expected counts stay.
        assert_eq!(input.binding(0, 5), DIK_SPACE);
        assert_eq!(input.binding(1, 4), 0);
        assert_eq!(input.binding(1, 5), 0xFF);
        platform
            .keyboard_states
            .push_back(keyboard_bytes(&[DIK_SPACE]));
        input.poll_devices(1, &mut platform);
        assert_eq!(input.query_scan_code_state(DIK_SPACE, I::QUERY_HELD), 1);
        assert_eq!(input.query_scan_code_state(DIK_SPACE, I::QUERY_PRESSED), 1);
        assert_eq!(input.query_scan_code_state(DIK_SPACE, I::QUERY_RELEASED), 0);
        assert_eq!(input.query_scan_code_state(DIK_SPACE, I::QUERY_CHANGED), 1);
        assert_eq!(input.query_scan_code_state(DIK_SPACE, 7), 0);
        assert_eq!(
            input.query_control_state(5, I::QUERY_PRESSED),
            1,
            "Control 5 (Activate) must sum the bound schemes"
        );
        let activate_button = input.binding(2, 5) as usize;
        platform
            .joystick_states
            .push_back(joystick_bytes(9000, &[activate_button]));
        input.poll_devices(2, &mut platform);
        assert_eq!(
            input.query_control_state(5, I::QUERY_HELD),
            2,
            "Keyboard held + joystick pressed must count both schemes"
        );
        assert_eq!(input.query_control_state(5, I::QUERY_PRESSED), 1);
        assert_eq!(input.joystick_pov_direction(0, 0), 0x22);
        assert_eq!(input.query_input_state(2, 0x22, I::QUERY_HELD), 1);
        assert_eq!(input.query_input_state(2, 0x23, I::QUERY_HELD), 0);
        assert_eq!(input.query_input_state(3, 0, I::QUERY_HELD), 0);
        assert_eq!(input.query_input_state(0xFF, DIK_SPACE, I::QUERY_HELD), 0);
        platform
            .joystick_states
            .push_back(joystick_bytes(0x1234_FFFF, &[]));
        input.poll_devices(3, &mut platform);
        assert_eq!(
            input.joystick_pov_direction(0, 0),
            -1,
            "A 0xFFFF POV low word must be -1"
        );
        platform
            .joystick_states
            .push_back(joystick_bytes(0x8000_0000, &[]));
        input.poll_devices(4, &mut platform);
        assert_eq!(
            input.joystick_pov_direction(0, 0),
            0x8000_0000u32 as i32 / 4500 + 0x20,
            "00403070 must divide signed"
        );

        // 29/30/31: Escape, grave, SysRq just pressed; `query` ignored.
        platform
            .keyboard_states
            .push_back(keyboard_bytes(&[DIK_ESCAPE, DIK_GRAVE, DIK_SYSRQ]));
        input.poll_devices(5, &mut platform);
        assert_eq!(input.query_control_state(29, I::QUERY_HELD), 1);
        assert_eq!(input.query_control_state(30, 99), 1);
        assert_eq!(input.query_control_state(31, 0), 1);
        assert_eq!(input.query_control_state(32, 0), 0);
        input.poll_devices(6, &mut platform);
        assert_eq!(
            input.query_control_state(29, I::QUERY_HELD),
            0,
            "A held Escape is not just pressed"
        );

        // Mouse: wheel 8/9, swapped buttons (+0x1B48 from SwapMouseButton), double-click query 4. Query button 0 reads the
        // physical primary.
        let primary = usize::from(input.swap_mouse_buttons_1b48 != 0);
        platform
            .mouse_states
            .push_back(mouse_bytes(120, &[primary]));
        input.poll_devices(7, &mut platform);
        // The click times start at 0, so a first press while TimeInfo+0x10 is within +0x1B4C of 0 (P06-RUN-001: +0x10 is 0 at
        // startup) already counts as a double click in the original.
        assert_eq!(input.query_mouse_button_state(8, 0), 1);
        assert_eq!(input.query_mouse_button_state(9, 0), 0);
        assert_eq!(input.query_mouse_button_state(0, I::QUERY_PRESSED), 1);
        assert_eq!(input.query_mouse_button_state(0, I::QUERY_DOUBLE_CLICK), 1);
        assert_eq!(input.query_mouse_button_state(10, I::QUERY_HELD), 0);
        platform.mouse_states.push_back(mouse_bytes(-120, &[]));
        input.poll_devices(8, &mut platform);
        platform
            .mouse_states
            .push_back(mouse_bytes(-120, &[primary]));
        input.poll_devices(9, &mut platform);
        assert_eq!(input.query_mouse_button_state(9, 0), 1);
        assert_eq!(input.query_mouse_button_state(0, I::QUERY_DOUBLE_CLICK), 1);
    }
}
