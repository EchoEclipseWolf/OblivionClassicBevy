//! Phase 04 row 04-B: WinMain 0040E763..0040E812 (window class and main window), the single-instance branch 0040E39B, and
//! the WinMain replay of the WM_ACTIVATE activation 0040EAB1..0040EAD7 (Phases 05/06). The window procedure 004060F0 is in
//! `recomp_game::tes::window`.
//!
//! Host substitution: no window exists here. Every Win32 call is an entry in a returned list with the values it passed, and
//! its result comes through `WindowPlatform`; `recomp_host` makes the Bevy window from the list. The C++ host's own class
//! name is gone with its Win32 window, so the list holds the original's class.

use recomp_game::tes::os_globals::discard_buffered_input_events;
use recomp_game::tes::os_globals::{
    HostInstance, HostWindow, OsGlobalsData, OsGlobalsPlatform, S_OK,
};
use recomp_game::tes::window::{SharedTimeInfo, time_info_decrement_disable_count};

/// IDA: 00B02C80 -> A2FBB4, the class name and, through the same pointer, the title.
pub const ORIGINAL_WINDOW_CLASS: &str = "Oblivion";
/// IDA: 0040E76E the procedure the class names, an address kept as evidence.
pub const ORIGINAL_WINDOW_PROCEDURE: u32 = 0x0040_60F0;
pub const ORIGINAL_WINDOW_STYLE: u32 = 0x00CA_0000;
pub const ORIGINAL_WINDOW_CREATE_STYLE: u32 = 0x10CA_0000;
pub const ORIGINAL_BOOTSTRAP_WIDTH: i32 = 320;
pub const ORIGINAL_BOOTSTRAP_HEIGHT: i32 = 240;
pub const ORIGINAL_ICON_RESOURCE: u32 = 101;
pub const BLACK_BRUSH: u32 = 4;
/// CS_VREDRAW | CS_HREDRAW.
pub const ORIGINAL_CLASS_STYLE: u32 = 3;

/// A Win32 RECT.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// IDA: WNDCLASSA, as 0040E766..0040E7B8 fills it. Handles are host values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowClass {
    pub style: u32,
    pub procedure: u32,
    pub class_extra: i32,
    pub window_extra: i32,
    pub instance: HostInstance,
    pub icon: u32,
    pub cursor: u32,
    pub background: u32,
    pub menu_name: Option<String>,
    pub class_name: String,
}

/// IDA: CreateWindowExA's twelve arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateWindowCall {
    pub ex_style: u32,
    pub class_name: String,
    pub title: String,
    pub style: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub parent: u32,
    pub menu: u32,
    pub instance: HostInstance,
    pub param: u32,
}

/// IDA: N/A one Win32 call of these blocks, with the values passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowCall {
    LoadIcon {
        instance: HostInstance,
        resource: u32,
    },
    GetStockObject {
        object: u32,
    },
    RegisterClass(WindowClass),
    AdjustWindowRect {
        rect: Rect,
        style: u32,
        menu: u32,
    },
    CreateWindowEx(CreateWindowCall),
    FindWindow {
        class_name: String,
    },
    SetForegroundWindow {
        window: HostWindow,
    },
}

/// IDA: N/A one entry: the original call site and the call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowCallEntry {
    pub site: u32,
    pub call: WindowCall,
}

/// IDA: N/A the seam for the Win32 results. `AdjustWindowRect` and `GetClientRect` give what a run recorded or a host window
/// reports; `recomp_game` stays free of Bevy windowing.
pub trait WindowPlatform {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn load_icon(&mut self, instance: HostInstance, resource: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn get_stock_object(&mut self, object: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn register_class(&mut self, class: &WindowClass) -> u16;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn adjust_window_rect(&mut self, rect: Rect, style: u32, menu: u32) -> Rect;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn create_window_ex(&mut self, call: &CreateWindowCall) -> Option<HostWindow>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn find_window(&mut self, class_name: &str) -> Option<HostWindow>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn set_foreground_window(&mut self, window: HostWindow) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn get_client_rect(&mut self, window: HostWindow) -> Rect;
}

/// IDA: N/A what `create_original_main_window` did. `registered` and `window` are unchecked by the original.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MainWindowCreation {
    pub registered: u16,
    /// After AdjustWindowRect.
    pub bootstrap: Rect,
    pub window: Option<HostWindow>,
    pub calls: Vec<WindowCallEntry>,
}

/// IDA: 0040E766..0040E80C. Completion: 100% of the fields and arguments: WNDCLASSA{style 3, 004060F0, 0, 0, hInstance,
/// LoadIconA(hInstance, 101), NULL cursor, GetStockObject(BLACK_BRUSH), NULL menu, "Oblivion"}; RegisterClassA (result
/// unchecked); AdjustWindowRect({0, 0, 320, 240}, 0x00CA0000, FALSE); CreateWindowExA(0, class, "Oblivion", 0x10CA0000, 0,
/// 0, w, h, NULL, NULL, hInstance, NULL) (result unchecked).
pub fn create_original_main_window(
    instance: HostInstance,
    platform: &mut dyn WindowPlatform,
) -> MainWindowCreation {
    let mut calls = Vec::new();
    // 0040E766..0040E7B8 in store order.
    calls.push(WindowCallEntry {
        site: 0x0040_E782,
        call: WindowCall::LoadIcon {
            instance,
            resource: ORIGINAL_ICON_RESOURCE,
        },
    });
    let icon = platform.load_icon(instance, ORIGINAL_ICON_RESOURCE);
    calls.push(WindowCallEntry {
        site: 0x0040_E792,
        call: WindowCall::GetStockObject {
            object: BLACK_BRUSH,
        },
    });
    let background = platform.get_stock_object(BLACK_BRUSH);
    let class = WindowClass {
        style: ORIGINAL_CLASS_STYLE,
        procedure: ORIGINAL_WINDOW_PROCEDURE,
        class_extra: 0,
        window_extra: 0,
        instance,
        icon,
        cursor: 0,
        background,
        class_name: ORIGINAL_WINDOW_CLASS.to_owned(),
        menu_name: None,
    };
    calls.push(WindowCallEntry {
        site: 0x0040_E7B8,
        call: WindowCall::RegisterClass(class.clone()),
    });
    let registered = platform.register_class(&class);
    // 0040E7BE..0040E7E1.
    let unadjusted = Rect {
        left: 0,
        top: 0,
        right: ORIGINAL_BOOTSTRAP_WIDTH,
        bottom: ORIGINAL_BOOTSTRAP_HEIGHT,
    };
    calls.push(WindowCallEntry {
        site: 0x0040_E7E1,
        call: WindowCall::AdjustWindowRect {
            rect: unadjusted,
            style: ORIGINAL_WINDOW_STYLE,
            menu: 0,
        },
    });
    let bootstrap = platform.adjust_window_rect(unadjusted, ORIGINAL_WINDOW_STYLE, 0);
    // 0040E7E7..0040E80C: the original passes its class-name pointer as the title too.
    let create = CreateWindowCall {
        ex_style: 0,
        class_name: ORIGINAL_WINDOW_CLASS.to_owned(),
        title: ORIGINAL_WINDOW_CLASS.to_owned(),
        style: ORIGINAL_WINDOW_CREATE_STYLE,
        x: 0,
        y: 0,
        width: bootstrap.right - bootstrap.left,
        height: bootstrap.bottom - bootstrap.top,
        parent: 0,
        menu: 0,
        instance,
        param: 0,
    };
    calls.push(WindowCallEntry {
        site: 0x0040_E80C,
        call: WindowCall::CreateWindowEx(create.clone()),
    });
    let window = platform.create_window_ex(&create);
    MainWindowCreation {
        registered,
        bootstrap,
        window,
        calls,
    }
}

/// IDA: N/A what `activate_existing_window` did. `found` means WinMain returns 0 at 0040F61D.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExistingWindowActivation {
    pub found: bool,
    pub calls: Vec<WindowCallEntry>,
}

/// IDA: 0040E39B FindWindowA(class="Oblivion", NULL) and 0040E3A6 SetForegroundWindow, then WinMain returns 0 (0040F61D xor
/// eax,eax) before allocator registration. Completion: 100% of the branch.
pub fn activate_existing_window(
    class_name: &str,
    platform: &mut dyn WindowPlatform,
) -> ExistingWindowActivation {
    let mut calls = vec![WindowCallEntry {
        site: 0x0040_E39B,
        call: WindowCall::FindWindow {
            class_name: class_name.to_owned(),
        },
    }];
    // 0040E3A1 `cmp eax, ebp; je 0040E3B1`.
    let Some(existing) = platform.find_window(class_name) else {
        return ExistingWindowActivation {
            found: false,
            calls,
        };
    };
    calls.push(WindowCallEntry {
        site: 0x0040_E3A6,
        call: WindowCall::SetForegroundWindow { window: existing },
    });
    platform.set_foreground_window(existing);
    ExistingWindowActivation { found: true, calls }
}

/// IDA: N/A the result of `replay_startup_activation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupActivationReplay {
    /// 0040EABF took the input branch.
    pub input: bool,
    /// 00403160 GetDeviceData result (ignored).
    pub discard: u32,
    /// The keyboard at +0x28 was present.
    pub discard_device: bool,
    /// TimeInfo disable count after 0047D0F0.
    pub time_count: u8,
    /// OSGlobals+2 after 0040EAD7.
    pub window_active: u8,
}

/// IDA: 0040EAB1..0040EAD7, the WinMain replay of the WM_ACTIVATE activation path (Phase 05 0040EAC1 discard plus Phase 06
/// rows 06-B-2/06-B-3): `if (g_osGlobals->input)` { 00403160 discard; 004046A0 poll (reading TimeInfo+0x10) }, 0047D0F0 on
/// g_timeInfo, OSGlobals+2 = 1. Completion: 100% of the order and stores; `now` is the GetTickCount value 0047D0F0 would
/// read. The disk executable keeps this 0047D0F0 call (0040EAD2), unlike 0040616E.
/// The MoviePlayer thread starts later, at 0040EAEA, but polls the same state, so this takes the same `Shared` locks as the
/// WM_ACTIVATE sequence, the C++ `os_input_mutex` scope.
pub fn replay_startup_activation(
    globals: &mut OsGlobalsData,
    time: &SharedTimeInfo,
    now: u32,
    platform: &mut dyn OsGlobalsPlatform,
) -> StartupActivationReplay {
    let mut result = StartupActivationReplay {
        input: false,
        discard: S_OK,
        discard_device: false,
        time_count: 0,
        window_active: 0,
    };
    // 0040EAB1..0040EABF: esi = g_osGlobals; `jz 0040EACD` on a null input.
    if let Some(input) = globals.input_20.as_ref() {
        let mut input = input.lock();
        result.input = true;
        // 0040EAC1 (Phase 05).
        let flush = discard_buffered_input_events(Some(&input), platform);
        result.discard = flush.result;
        result.discard_device = flush.device;
        // 0040EAC6..0040EAC8 (Phase 06, 06-B-1).
        let time_info_10 = time.lock().paused_ticks_10;
        input.poll_devices(time_info_10, platform);
    }
    // 0040EACD..0040EAD2: 0047D0F0 on g_timeInfo 00B33E90.
    let mut time = time.lock();
    time_info_decrement_disable_count(&mut time, now);
    result.time_count = time.disable_count_00;
    drop(time);
    // 0040EAD7: mov byte ptr [esi+2], 1.
    globals.window_active_02 = 1;
    result.window_active = globals.window_active_02;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use recomp_core::fixtures;
    use recomp_game::tes::os_globals::{
        DIERR_NOTACQUIRED, HostWindow, OsGlobalsPlatform, OsInputGlobals, ScriptedOsPlatform,
        Shared, scripted_input_settings,
    };
    use recomp_game::tes::settings::{SettingCollections, register_static_settings};
    use recomp_game::tes::window::{
        TimeInfoState, WM_ACTIVATE, WindowMessage, WindowProcedureServices, WindowProcedureState,
        original_main_window_proc,
    };

    const INSTANCE: HostInstance = HostInstance(0x0040_0000);

    /// IDA: N/A the seam's script. It keeps registered classes and created windows, so `find_window` answers from what ran.
    #[derive(Default)]
    struct ScriptedWindows {
        /// What AdjustWindowRect returns: P04-RUN-001's record.
        adjusted: Rect,
        classes: Vec<WindowClass>,
        windows: Vec<(String, HostWindow, i32, i32)>,
        foreground: Vec<HostWindow>,
    }

    impl WindowPlatform for ScriptedWindows {
        fn load_icon(&mut self, _instance: HostInstance, _resource: u32) -> u32 {
            0x51
        }
        fn get_stock_object(&mut self, _object: u32) -> u32 {
            0x52
        }
        fn register_class(&mut self, class: &WindowClass) -> u16 {
            self.classes.push(class.clone());
            0xC000 + self.classes.len() as u16
        }
        fn adjust_window_rect(&mut self, _rect: Rect, _style: u32, _menu: u32) -> Rect {
            self.adjusted
        }
        fn create_window_ex(&mut self, call: &CreateWindowCall) -> Option<HostWindow> {
            if !self
                .classes
                .iter()
                .any(|class| class.class_name == call.class_name)
            {
                return None;
            }
            let window = HostWindow(0x100 + self.windows.len() as u32);
            self.windows
                .push((call.class_name.clone(), window, call.width, call.height));
            Some(window)
        }
        fn find_window(&mut self, class_name: &str) -> Option<HostWindow> {
            self.windows
                .iter()
                .find(|(class, ..)| class == class_name)
                .map(|(_, window, ..)| *window)
        }
        fn set_foreground_window(&mut self, window: HostWindow) -> u32 {
            self.foreground.push(window);
            1
        }
        /// The frame's size less the margins of the recorded rectangle. No run recorded GetClientRect itself.
        fn get_client_rect(&mut self, window: HostWindow) -> Rect {
            let (.., width, height) = self
                .windows
                .iter()
                .find(|(_, candidate, ..)| *candidate == window)
                .unwrap();
            let horizontal = (self.adjusted.right - ORIGINAL_BOOTSTRAP_WIDTH) - self.adjusted.left;
            let vertical = (self.adjusted.bottom - ORIGINAL_BOOTSTRAP_HEIGHT) - self.adjusted.top;
            Rect {
                left: 0,
                top: 0,
                right: width - horizontal,
                bottom: height - vertical,
            }
        }
    }

    /// `P04-RUN-001`'s window rows, after a check that each is a verbatim line of the run's records.
    fn run_001_rows() -> Vec<String> {
        let path = fixtures::root().join("evidence/phase04-run001-2026-10-05.records.txt");
        let evidence = std::fs::read_to_string(path).unwrap();
        let rows: Vec<String> = fixtures::rows("phase04")
            .iter()
            .filter_map(|row| row.strip_prefix("P04-RUN-001 "))
            .filter(|row| row.contains(" 40E7E7 ") || row.contains(" 40E80C "))
            .map(str::to_owned)
            .collect();
        for row in &rows {
            assert!(
                evidence.lines().any(|line| line == row),
                "not in the records: {row}"
            );
        }
        rows
    }

    /// `S 3 40E7E7 RECT ... rect=fdffffffe6ffffff43010000f3000000`: four little-endian dwords.
    fn recorded_rect() -> Rect {
        let rows = run_001_rows();
        let row = rows
            .iter()
            .find(|row| row.contains(" 40E7E7 RECT "))
            .unwrap();
        let hex = row.rsplit("rect=").next().unwrap();
        let word = |index: usize| {
            let start = index * 8;
            let bytes: Vec<u8> = (0..4)
                .map(|byte| {
                    u8::from_str_radix(&hex[start + byte * 2..start + byte * 2 + 2], 16).unwrap()
                })
                .collect();
            i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        };
        Rect {
            left: word(0),
            top: word(1),
            right: word(2),
            bottom: word(3),
        }
    }

    /// The window half of `verify_window`. Each check of a real HWND in the C++ test checks a recorded call's argument here:
    /// no window exists. The window-procedure half is in `recomp_game::tes::window`.
    #[test]
    fn verify_window_creation() {
        let mut platform = ScriptedWindows {
            adjusted: recorded_rect(),
            ..ScriptedWindows::default()
        };
        let created = create_original_main_window(INSTANCE, &mut platform);

        // C++: `created.registered != 0 && created.window != nullptr`. Here: the seam's results came back.
        assert_ne!(created.registered, 0);
        let window = created.window.expect("Original window was not created");

        // C++: `EqualRect(bootstrap, AdjustWindowRect({0,0,320,240}, 0x00CA0000))` on a live desktop. Here: the arguments of
        // the recorded call, and the recorded result through the seam.
        let adjust = created
            .calls
            .iter()
            .find_map(|entry| match entry.call {
                WindowCall::AdjustWindowRect { rect, style, menu } => Some((rect, style, menu)),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            adjust,
            (
                Rect {
                    left: 0,
                    top: 0,
                    right: 320,
                    bottom: 240
                },
                0x00CA_0000,
                0
            )
        );
        assert_eq!(
            created.bootstrap,
            Rect {
                left: -3,
                top: -26,
                right: 323,
                bottom: 243
            }
        );

        // C++: `GetClientRect == 320x240` on the real window. No run recorded it. Here: the seam's client from the recorded
        // frame margins and the width and height passed; the arguments are compared with `C 4 40E80C` below.
        let client = platform.get_client_rect(window);
        assert_eq!((client.right, client.bottom), (320, 240));

        let creation = created
            .calls
            .iter()
            .find_map(|entry| match &entry.call {
                WindowCall::CreateWindowEx(call) => Some(call.clone()),
                _ => None,
            })
            .unwrap();
        // C++: `GetWindowLongPtrW(GWL_STYLE) & 0x10CA0000`. Here: the style argument.
        assert_eq!(creation.style, 0x10CA_0000);
        // C++: `GetWindowTextW == "Oblivion"`. Here: the title argument, the class pointer in the original.
        assert_eq!(creation.title, "Oblivion");
        assert_eq!(creation.class_name, creation.title);

        // C++: `GetClassLongPtrW` cursor, style and brush. Here: the WNDCLASS fields of the recorded RegisterClass, the brush
        // being GetStockObject(BLACK_BRUSH)'s result.
        let class = created
            .calls
            .iter()
            .find_map(|entry| match &entry.call {
                WindowCall::RegisterClass(class) => Some(class.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(class.cursor, 0);
        assert_eq!(class.style & 3, 3);
        assert_eq!(class.background, 0x52);
        assert_eq!(class.procedure, 0x0040_60F0);
        assert_eq!(class.icon, 0x51);
        assert_eq!(class.menu_name, None);

        // The order of the block.
        let sites: Vec<u32> = created.calls.iter().map(|entry| entry.site).collect();
        assert_eq!(
            sites,
            [
                0x0040_E782,
                0x0040_E792,
                0x0040_E7B8,
                0x0040_E7E1,
                0x0040_E80C
            ]
        );
        assert!(created.calls.contains(&WindowCallEntry {
            site: 0x0040_E782,
            call: WindowCall::LoadIcon {
                instance: INSTANCE,
                resource: 101
            }
        }));
        assert!(created.calls.contains(&WindowCallEntry {
            site: 0x0040_E792,
            call: WindowCall::GetStockObject { object: 4 }
        }));

        // `C 4 40E80C ... a=0,A2FBB4,A2FBB4,10CA0000,0,0,146,10D,0,0,400000,0`.
        let rows = run_001_rows();
        let row = rows.iter().find(|row| row.contains(" 40E80C ")).unwrap();
        let args: Vec<u32> = row
            .rsplit("a=")
            .next()
            .unwrap()
            .split(',')
            .map(|word| u32::from_str_radix(word, 16).unwrap())
            .collect();
        assert_eq!(args[1], args[2], "the class and title are one pointer");
        assert_eq!(args[0], creation.ex_style);
        assert_eq!(args[3], creation.style);
        assert_eq!(args[4], creation.x as u32);
        assert_eq!(args[5], creation.y as u32);
        assert_eq!(args[6], creation.width as u32);
        assert_eq!(args[7], creation.height as u32);
        assert_eq!(args[8], creation.parent);
        assert_eq!(args[9], creation.menu);
        assert_eq!(args[10], creation.instance.0);
        assert_eq!(args[11], creation.param);
    }

    /// `verify_existing_window_branch`, scripted: the class and window it registers and creates are the seam's.
    #[test]
    fn verify_existing_window_branch() {
        let name = format!("OblivionRecompInstanceTest-{}", std::process::id());
        let mut platform = ScriptedWindows::default();
        let definition = WindowClass {
            style: 0,
            procedure: 0,
            class_extra: 0,
            window_extra: 0,
            instance: INSTANCE,
            icon: 0,
            cursor: 0,
            background: 0,
            menu_name: None,
            class_name: name.clone(),
        };
        assert_ne!(
            platform.register_class(&definition),
            0,
            "Could not register instance test window"
        );
        let no_window = activate_existing_window(&name, &mut platform);
        assert!(!no_window.found, "No-window path did not advance");
        assert_eq!(no_window.calls.len(), 1);
        assert!(platform.foreground.is_empty());

        let window = platform
            .create_window_ex(&CreateWindowCall {
                ex_style: 0,
                class_name: name.clone(),
                title: "instance test".to_owned(),
                style: 0,
                x: 0,
                y: 0,
                width: 20,
                height: 20,
                parent: 0,
                menu: 0,
                instance: INSTANCE,
                param: 0,
            })
            .expect("Could not create instance test window");
        let existing = activate_existing_window(&name, &mut platform);
        assert!(existing.found, "Existing-window early return was lost");
        assert_eq!(
            existing.calls,
            [
                WindowCallEntry {
                    site: 0x0040_E39B,
                    call: WindowCall::FindWindow {
                        class_name: name.clone()
                    }
                },
                WindowCallEntry {
                    site: 0x0040_E3A6,
                    call: WindowCall::SetForegroundWindow { window }
                }
            ]
        );
        assert_eq!(platform.foreground, [window]);
    }

    /// The small part of Phase 06's check that runs without its other state: the order and stores of 0040EAB1..0040EAD7.
    #[test]
    fn startup_activation_replay() {
        let mut settings = SettingCollections::default();
        register_static_settings(&mut settings);
        let mut platform = ScriptedOsPlatform::default();
        let input = OsInputGlobals::new(INSTANCE, &settings, &mut platform).unwrap();
        let mut globals = OsGlobalsData {
            input_20: Some(Shared::new(*input)),
            ..OsGlobalsData::default()
        };
        let time = SharedTimeInfo::new(TimeInfoState {
            disable_count_00: 1,
            paused_ticks_10: 100,
            game_start_ticks_14: 50,
            ..TimeInfoState::default()
        });
        platform.trace.clear();
        let replay = replay_startup_activation(&mut globals, &time, 1000, &mut platform);
        assert_eq!(
            replay,
            StartupActivationReplay {
                input: true,
                discard: DIERR_NOTACQUIRED,
                discard_device: true,
                time_count: 0,
                window_active: 1
            }
        );
        assert_eq!(platform.trace[0].site, 0x0040_3183);
        assert_eq!(platform.trace[1].site, 0x0040_4751);
        let after = *time.lock();
        assert_eq!(
            (after.paused_ticks_10, after.game_start_ticks_14),
            (900, 950)
        );

        // No input: straight to 0047D0F0, which returns at a zero count.
        let mut bare = OsGlobalsData::default();
        let zero = SharedTimeInfo::default();
        platform.trace.clear();
        let replay = replay_startup_activation(&mut bare, &zero, 5, &mut platform);
        assert!(!replay.input && !replay.discard_device);
        assert_eq!(replay.window_active, 1);
        assert!(platform.trace.is_empty());
    }

    struct ActivationServices {
        platform: ScriptedOsPlatform,
        tick: u32,
    }

    impl WindowProcedureServices for ActivationServices {
        fn os_platform(&mut self) -> &mut dyn OsGlobalsPlatform {
            &mut self.platform
        }
        fn def_window_proc(&mut self, _message: &WindowMessage) -> i32 {
            0
        }
        fn get_tick_count(&mut self) -> u32 {
            self.tick
        }
        fn show_cursor(&mut self, _show: bool) -> i32 {
            0
        }
        fn window_pos(&mut self, _lparam: u32) -> (i32, i32) {
            (0, 0)
        }
    }

    /// C++ verify_activation: 0040EAB1..0040EAD7 and the 004060F0 WM_ACTIVATE wiring (06-B-2/3).
    #[test]
    fn verify_activation() {
        let settings = scripted_input_settings(true);
        let mut platform = ScriptedOsPlatform::with_devices(true, false);
        let input = OsInputGlobals::new(INSTANCE, &settings, &mut platform).unwrap();
        let mut globals = OsGlobalsData {
            input_20: Some(Shared::new(*input)),
            ..OsGlobalsData::default()
        };
        let time = SharedTimeInfo::new(TimeInfoState {
            disable_count_00: 1,
            paused_ticks_10: 40,
            game_start_ticks_14: 10,
            ..TimeInfoState::default()
        });
        platform.trace.clear();
        let replay = replay_startup_activation(&mut globals, &time, 1000, &mut platform);
        assert!(replay.input && replay.discard_device);
        assert_eq!(replay.discard, DIERR_NOTACQUIRED);
        assert_eq!(
            platform.trace[0].site, 0x0040_3183,
            "The replay must discard then poll when the input exists"
        );
        let acquires = |platform: &ScriptedOsPlatform, device| {
            platform
                .trace
                .iter()
                .filter(|entry| {
                    entry.call == recomp_game::tes::os_globals::OsCall::Acquire { device }
                })
                .count()
        };
        assert_eq!(acquires(&platform, ScriptedOsPlatform::KEYBOARD), 1);
        assert_eq!(acquires(&platform, ScriptedOsPlatform::MOUSE), 1);
        assert_eq!(replay.time_count, 0);
        {
            let time = time.lock();
            assert_eq!(time.disable_count_00, 0);
            assert_eq!(
                time.paused_ticks_10, 960,
                "0047D0F0 must rebase on reaching zero"
            );
            assert_eq!(time.game_start_ticks_14, 990);
        }
        assert_eq!(globals.window_active_02, 1, "+2 must become 1");
        assert_eq!(replay.window_active, 1);

        let mut bare = OsGlobalsData::default();
        let zero = SharedTimeInfo::default();
        let no_input = replay_startup_activation(&mut bare, &zero, 5, &mut platform);
        assert!(!no_input.input);
        assert_eq!(no_input.time_count, 0);
        assert_eq!(zero.lock().paused_ticks_10, 0);
        assert_eq!(
            bare.window_active_02, 1,
            "Without input the replay skips discard/poll but still stores +2"
        );

        // WM_ACTIVATE calls the same discard and poll (00406160..00406173 and 0040619E..004061B4).
        let mut services = ActivationServices { platform, tick: 0 };
        let mut state = WindowProcedureState::default();
        let data_calls = |platform: &ScriptedOsPlatform| {
            platform
                .trace
                .iter()
                .filter(|entry| entry.site == 0x0040_3183)
                .count()
        };
        let discards = data_calls(&services.platform);
        let mouse = acquires(&services.platform, ScriptedOsPlatform::MOUSE);
        let window = HostWindow(1);
        original_main_window_proc(
            &mut state,
            Some(&mut globals),
            &mut services,
            WindowMessage {
                window,
                message: WM_ACTIVATE,
                wparam: 0, // WA_INACTIVE
                lparam: 0,
            },
        );
        assert_eq!(data_calls(&services.platform), discards + 1);
        assert_eq!(
            acquires(&services.platform, ScriptedOsPlatform::MOUSE),
            mouse + 1
        );
        assert_eq!(globals.window_active_02, 0);
        assert_eq!(
            state.time.lock().disable_count_00,
            1,
            "WM_ACTIVATE(WA_INACTIVE) must discard, poll, pause and clear +2"
        );
        original_main_window_proc(
            &mut state,
            Some(&mut globals),
            &mut services,
            WindowMessage {
                window,
                message: WM_ACTIVATE,
                wparam: 2, // WA_CLICKACTIVE
                lparam: 0,
            },
        );
        assert_eq!(data_calls(&services.platform), discards + 2);
        assert_eq!(
            acquires(&services.platform, ScriptedOsPlatform::MOUSE),
            mouse + 2
        );
        assert_eq!(globals.window_active_02, 1);
        assert_eq!(
            state.time.lock().disable_count_00,
            0,
            "WM_ACTIVATE(WA_CLICKACTIVE) must discard, poll, resume and set +2"
        );
    }

    /// P06-RUN-001's watch: 0040EAD7 stored 1 in OSGlobals+2. P06-RUN-001 and -002's flags, joystick count and double-click
    /// time, from a script of their configuration.
    #[test]
    fn activation_and_input_match_the_runs() {
        let settings = scripted_input_settings(true);
        let mut platform = ScriptedOsPlatform::default();
        let input = OsInputGlobals::new(INSTANCE, &settings, &mut platform).unwrap();
        let rows: Vec<Vec<String>> = fixtures::rows("phase06")
            .into_iter()
            .filter_map(|row| row.strip_prefix("MOVIE|").map(str::to_owned))
            .map(|row| row.split('|').map(str::to_owned).collect())
            .collect();
        let inputs: Vec<&Vec<String>> = rows.iter().filter(|row| row[1] == "INPUT").collect();
        assert_eq!(inputs.len(), 2);
        for row in inputs {
            let flags: String = input
                .flags_0000
                .to_le_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            assert_eq!(flags, row[2], "{row:?}");
            assert_eq!(input.joystick_count_18f0.to_string(), row[3], "{row:?}");
            assert_eq!(input.double_click_time_1b4c.to_string(), row[4], "{row:?}");
        }
        let active = rows.iter().find(|row| row[1] == "ACTIVE").unwrap();
        let mut globals = OsGlobalsData::default();
        let replay =
            replay_startup_activation(&mut globals, &SharedTimeInfo::default(), 5, &mut platform);
        assert_eq!(format!("{:02}", replay.window_active), active[3]);
        // BINKSOUND_RET reads the same byte, +2, just after that store.
        for row in rows.iter().filter(|row| row[1] == "SOUND") {
            assert_eq!(format!("{:02}", replay.window_active), row[3], "{row:?}");
        }
    }
}
