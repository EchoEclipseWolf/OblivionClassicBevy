//! Phase 04 row 04-B: 004060F0 Oblivion_MainWindowProc as a function over message numbers and its state, and the
//! TimeInfo state it reads. WinMain's window class and main window are in `recomp_startup::winmain_window`.
//!
//! Host substitutions: no real window exists here; `recomp_host` feeds its Bevy window's events through this. The C++
//! port's host additions are gone with the Win32 host: the MenuView hook ahead of the cases, `PostQuitMessage` for the
//! main window. The `os_input_mutex` lock is `Shared`: Phase 06's movie thread polls the same input and reads TimeInfo+0x10.

use bevy_ecs::prelude::*;
use recomp_core::x87::stored_float;

use super::os_globals::{
    HostWindow, OsGlobalsData, OsGlobalsPlatform, Shared, discard_buffered_input_events,
};

pub const WM_DESTROY: u32 = 0x0002;
pub const WM_ACTIVATE: u32 = 0x0006;
pub const WM_ERASEBKGND: u32 = 0x0014;
pub const WM_WINDOWPOSCHANGED: u32 = 0x0047;
pub const WM_NCHITTEST: u32 = 0x0084;
pub const WM_NCACTIVATE: u32 = 0x0086;
pub const WM_SYSKEYUP: u32 = 0x0105;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const HTCLIENT: i32 = 1;

/// IDA: TimeInfo (00B33E90): +0 disable count byte, +0x10 and +0x14 tick bookkeeping. Completion: 100% of these fields.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct TimeInfoState {
    pub disable_count_00: u8,
    /// +0x04 fpsClamp (00406360 0040692B; no host reader yet: the frame pacing is host-owned).
    pub fps_clamp_04: f32,
    /// +0x08 the fps clamp's carried fraction.
    pub clamp_remainder_08: f32,
    /// +0x0C the frame's seconds.
    pub frame_seconds_0c: f32,
    pub paused_ticks_10: u32,
    pub game_start_ticks_14: u32,
}

/// IDA: N/A 00B33E90 for both threads: 00410390 reads +0x10 on the MoviePlayer thread for each poll.
pub type SharedTimeInfo = Shared<TimeInfoState>;

/// IDA: 0047D170 TimeInfo_Update(tick), named here, as 0040D8AB calls it each frame. 0047D176 edx = tick - +0x14; 0047D181 the two globals 00B39DB0/00B39DB4 (kept by no reader here, so not stored); 0047D18C a disabled clock makes +0xC = 0.0; 0047D195 fpsClamp 0: step = edx - +0x10, 0 -> 1, then unsigned > 166.66667f (A3D14C) -> 166; 0047D19F otherwise the clamp: +8 + +4, rounded, its truncation is the whole step; +8 keeps the fraction; +0x10 = edx or +0x10 + step; +0xC = step * 0.001f (A30E40, a qword, at 24 bits) rounded, then * 00B06704, rounded. Completion: 100%; `time_scale_b06704` is 1.0f in the image and the caller's. 24-bit: this is the main thread after device creation.
pub fn time_info_update(time: &mut TimeInfoState, tick: u32, time_scale_b06704: f32) {
    let mut now = tick.wrapping_sub(time.game_start_ticks_14);
    if time.disable_count_00 != 0 {
        time.frame_seconds_0c = 0.0;
        return;
    }
    let step = if time.fps_clamp_04 == 0.0 {
        let elapsed = now.wrapping_sub(time.paused_ticks_10);
        let step = if elapsed == 0 {
            1
        } else if elapsed as f32 > 166.666_67f32 {
            0xA6
        } else {
            elapsed
        };
        step as f32
    } else {
        let total = time.clamp_remainder_08 + time.fps_clamp_04;
        let whole = recomp_core::x87::ftol64_low(total);
        time.clamp_remainder_08 = total - whole as f32;
        now = time.paused_ticks_10.wrapping_add(whole);
        time.fps_clamp_04
    };
    time.paused_ticks_10 = now;
    let seconds = (f64::from(step) * 0.001_000_000_047_497_451_3) as f32;
    time.frame_seconds_0c = seconds * time_scale_b06704;
}

/// IDA: 0047D120 TimeInfo_IncrementDisableCount: increments and, from 0, rebases both fields on GetTickCount. Completion:
/// 100%.
pub const fn time_info_increment_disable_count(time: &mut TimeInfoState, now: u32) {
    let previous = time.disable_count_00;
    time.disable_count_00 = previous.wrapping_add(1);
    if previous == 0 {
        time.paused_ticks_10 = now.wrapping_sub(time.paused_ticks_10);
        time.game_start_ticks_14 = now.wrapping_sub(time.game_start_ticks_14);
    }
}

/// IDA: 0047D0F0 TimeInfo_DecrementDisableCount: decrements and rebases when it reaches 0. Completion: 100%.
pub const fn time_info_decrement_disable_count(time: &mut TimeInfoState, now: u32) {
    if time.disable_count_00 == 0 {
        return;
    }
    time.disable_count_00 -= 1;
    if time.disable_count_00 == 0 {
        time.paused_ticks_10 = now.wrapping_sub(time.paused_ticks_10);
        time.game_start_ticks_14 = now.wrapping_sub(time.game_start_ticks_14);
    }
}

/// IDA: the globals 004060F0 reads and writes, apart from 00B33398 g_osGlobals, which the caller passes.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct WindowProcedureState {
    /// 00B34FA8 g_rendererDeviceWindow.
    pub device_window: Option<HostWindow>,
    /// 00B350D8 g_dx9Renderer != null.
    pub renderer_present: bool,
    /// 00B333F0 (dword compare; read only here).
    pub activation_blocked: u32,
    /// 00B333F5 (set by WM_NCACTIVATE(FALSE)).
    pub deactivated_nonclient: bool,
    /// 00B02F99 (.data 1).
    pub cursor_shown: bool,
    /// 00B33E90.
    pub time: SharedTimeInfo,
}

impl Default for WindowProcedureState {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            device_window: None,
            renderer_present: false,
            activation_blocked: 0,
            deactivated_nonclient: false,
            cursor_shown: true,
            time: SharedTimeInfo::default(),
        }
    }
}

/// One window message, with the original's 32-bit parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowMessage {
    pub window: HostWindow,
    pub message: u32,
    pub wparam: u32,
    pub lparam: u32,
}

/// IDA: N/A the seam for what 004060F0 calls: results come from a scripted or host implementation.
/// TODO: [ ] Carried: 00497BF0 (WM_WINDOWPOSCHANGED with a renderer) and 00405370 OSGlobals_HandleMenuMouseMove
///       (WM_MOUSEMOVE); no-ops until their phases port them.
pub trait WindowProcedureServices {
    /// IDA: see the words here, which name the addresses this relates to.
    /// The input seam the two 00403160 / 004046A0 calls use.
    fn os_platform(&mut self) -> &mut dyn OsGlobalsPlatform;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// DefWindowProcA.
    fn def_window_proc(&mut self, message: &WindowMessage) -> i32;
    /// IDA: see the words here, which name the addresses this relates to.
    /// GetTickCount, read inside 0047D0F0 / 0047D120.
    fn get_tick_count(&mut self) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    /// ShowCursor: the new display count.
    fn show_cursor(&mut self, show: bool) -> i32;
    /// IDA: see the words here, which name the addresses this relates to.
    /// 004061C1 reads lParam as WINDOWPOS*: returns its x and y.
    fn window_pos(&mut self, lparam: u32) -> (i32, i32);
    /// IDA: see the words here, which name the addresses this relates to.
    /// 00497BF0.
    fn window_position_changed(&mut self, _x: i32, _y: i32) {}
    /// IDA: see the words here, which name the addresses this relates to.
    /// 00405370; its result is whether the message is consumed. `false` until it is ported.
    fn menu_mouse_move(&mut self, _x: u32, _y: u32) -> bool {
        false
    }
}

/// IDA: 004060F0 Oblivion_MainWindowProc, all eight cases. `os_globals` is 00B33398. Completion: 100% of the original
/// cases; carried callees go through the seam.
/// TODO: [ ] Decision: the disk executable (SHA-256 0244dd59) NOPs the two WM_ACTIVATE TimeInfo calls (0040616E/004061AF,
///       a background-run patch with 0040F16E); this port follows the unpatched code in the IDB.
pub fn original_main_window_proc(
    state: &mut WindowProcedureState,
    mut os_globals: Option<&mut OsGlobalsData>,
    services: &mut dyn WindowProcedureServices,
    message: WindowMessage,
) -> i32 {
    let WindowMessage {
        window,
        message: number,
        wparam,
        lparam,
    } = message;
    match number {
        WM_DESTROY => {
            // 004061F6..0040620D.
            if let Some(globals) = os_globals {
                globals.quit_game_00 = true;
            }
            services.def_window_proc(&message)
        }
        WM_ACTIVATE => {
            // 00406130..004061B4: the state is LOWORD(wParam) (movzx at 00406134).
            let activation = wparam & 0xFFFF;
            if activation == 0 {
                // 00406180..0040618E.
                if let Some(globals) = os_globals.as_deref_mut()
                    && state.activation_blocked == 0
                {
                    // 0040619E/004061A5: 00403160 then 004046A0 (06-B-3).
                    poll_for_activation(state, globals, services);
                    // 004061AF.
                    let now = services.get_tick_count();
                    time_info_increment_disable_count(&mut state.time.lock(), now);
                    globals.window_active_02 = 0;
                }
                return 0;
            }
            // 0040613B..0040614F.
            if activation.wrapping_sub(1) > 1 || wparam >> 16 != 0 {
                return 0;
            }
            let Some(globals) = os_globals else {
                return 0;
            };
            // 0040615D/00406164: 00403160 then 004046A0 (06-B-3).
            poll_for_activation(state, globals, services);
            // 0040616E.
            let now = services.get_tick_count();
            time_info_decrement_disable_count(&mut state.time.lock(), now);
            globals.window_active_02 = 1;
            0
        }
        WM_ERASEBKGND => {
            // 0040620F..00406225.
            if state.device_window != Some(window) {
                return services.def_window_proc(&message);
            }
            1
        }
        WM_WINDOWPOSCHANGED => {
            // 004061C1..004061EC.
            if state.renderer_present {
                let (x, y) = services.window_pos(lparam);
                services.window_position_changed(x, y);
            }
            services.def_window_proc(&message)
        }
        WM_NCHITTEST => {
            // 00406237..00406295.
            let hit = services.def_window_proc(&message);
            if os_globals.is_some_and(|globals| globals.input_20.is_some()) {
                if hit == HTCLIENT {
                    if state.cursor_shown {
                        while services.show_cursor(false) > 0 {}
                        state.cursor_shown = false;
                    }
                } else if !state.cursor_shown {
                    services.show_cursor(true);
                    state.cursor_shown = true;
                }
            }
            hit
        }
        WM_NCACTIVATE => {
            // 004062D7..004062DD.
            if wparam == 0 {
                state.deactivated_nonclient = true;
            }
            services.def_window_proc(&message)
        }
        // 004062AA: swallowed.
        WM_SYSKEYUP => 0,
        WM_MOUSEMOVE => {
            // 004062B7..004062D6: 00405370(g_osGlobals, LOWORD(lParam), lParam >> 16) with no null check on g_osGlobals;
            // 00405370 is carried.
            if services.menu_mouse_move(lparam & 0xFFFF, lparam >> 16) {
                return 0;
            }
            services.def_window_proc(&message)
        }
        _ => services.def_window_proc(&message),
    }
}

/// IDA: the shared `if (input)` block of both WM_ACTIVATE arms: 00403160 then 004046A0, reading TimeInfo+0x10.
fn poll_for_activation(
    state: &WindowProcedureState,
    globals: &mut OsGlobalsData,
    services: &mut dyn WindowProcedureServices,
) {
    if let Some(input) = globals.input_20.as_ref() {
        // The C++ `os_input_mutex` scope: both calls under one lock.
        let mut input = input.lock();
        discard_buffered_input_events(Some(&input), services.os_platform());
        let time_info_10 = state.time.lock().paused_ticks_10;
        input.poll_devices(time_info_10, services.os_platform());
    }
}

/// IDA: 004068FA..0040692B the inline FPS clamp of 00406360: n = iFPSClamp:General (00B02D10); `lea ecx, [n - 1]; cmp ecx,
/// 0xC6; ja` keeps 1..199 only (0 and negatives wrap above 0xC6, so the `fldz` arm is unreachable), then fild n (the `fadd
/// 2^32` fix-up is dead: n > 0), fdivr 1000.0 (dbl_A2FC70) and fstp into TimeInfo +0x04 at 24-bit x87 precision: the
/// correctly rounded float of 1000.0 / n. Returns whether the store ran. Completion: 100%.
pub fn apply_fps_clamp(time: &mut TimeInfoState, fps_clamp_setting: i32) -> bool {
    let n = fps_clamp_setting as u32;
    if n.wrapping_sub(1) > 0xC6 {
        return false;
    }
    // 24-bit fdivr of the exact operands, stored as float: the division rounded once to float. The f64 quotient rounds
    // again; with 53 >= 2 * 24 + 2 bits the double rounding is innocuous for division.
    time.fps_clamp_04 = stored_float(1000.0 / f64::from(n));
    true
}

#[cfg(test)]
mod tests {
    use super::super::os_globals::*;
    use super::*;

    #[derive(Default)]
    struct Services {
        platform: ScriptedOsPlatform,
        def_calls: Vec<u32>,
        def_result: i32,
        tick: u32,
        cursor_calls: Vec<bool>,
        cursor_counts: Vec<i32>,
        positions: Vec<(i32, i32)>,
        mouse: Option<(u32, u32)>,
        consume_mouse: bool,
    }

    impl WindowProcedureServices for Services {
        fn os_platform(&mut self) -> &mut dyn OsGlobalsPlatform {
            &mut self.platform
        }
        fn def_window_proc(&mut self, message: &WindowMessage) -> i32 {
            self.def_calls.push(message.message);
            self.def_result
        }
        fn get_tick_count(&mut self) -> u32 {
            self.tick
        }
        fn show_cursor(&mut self, show: bool) -> i32 {
            self.cursor_calls.push(show);
            self.cursor_counts.pop().unwrap_or(0)
        }
        fn window_pos(&mut self, lparam: u32) -> (i32, i32) {
            (lparam as i32, 7)
        }
        fn window_position_changed(&mut self, x: i32, y: i32) {
            self.positions.push((x, y));
        }
        fn menu_mouse_move(&mut self, x: u32, y: u32) -> bool {
            self.mouse = Some((x, y));
            self.consume_mouse
        }
    }

    const WINDOW: HostWindow = HostWindow(0x77);

    const fn message(number: u32, wparam: u32, lparam: u32) -> WindowMessage {
        WindowMessage {
            window: WINDOW,
            message: number,
            wparam,
            lparam,
        }
    }

    /// The window-procedure half of `verify_window`. Its WNDCLASS, style and rectangle half is in
    /// `recomp_startup::winmain_window`.
    #[test]
    fn verify_window_procedure() {
        let mut services = Services::default();
        let mut state = WindowProcedureState::default();
        let proc = original_main_window_proc;
        assert_eq!(
            proc(&mut state, None, &mut services, message(WM_SYSKEYUP, 0, 0)),
            0,
            "WM_SYSKEYUP must be swallowed"
        );
        state.device_window = Some(WINDOW);
        assert_eq!(
            proc(
                &mut state,
                None,
                &mut services,
                message(WM_ERASEBKGND, 0, 0)
            ),
            1,
            "WM_ERASEBKGND on the device window must return 1"
        );
        assert_eq!(
            proc(&mut state, None, &mut services, message(WM_ACTIVATE, 0, 0)),
            0,
            "WM_ACTIVATE with no OS globals must return 0"
        );
        proc(
            &mut state,
            None,
            &mut services,
            message(WM_NCACTIVATE, 0, 0),
        );
        assert!(
            state.deactivated_nonclient,
            "WM_NCACTIVATE(FALSE) must set 00B333F5"
        );

        let mut globals = OsGlobalsData::default();
        state.time.lock().disable_count_00 = 1;
        // WA_ACTIVE.
        assert_eq!(
            proc(
                &mut state,
                Some(&mut globals),
                &mut services,
                message(WM_ACTIVATE, 1, 0)
            ),
            0
        );
        assert_eq!(globals.window_active_02, 1);
        assert_eq!(state.time.lock().disable_count_00, 0);
        // MAKEWPARAM(WA_ACTIVE, 1): a minimized activate returns before any store.
        assert_eq!(
            proc(
                &mut state,
                Some(&mut globals),
                &mut services,
                message(WM_ACTIVATE, 0x0001_0001, 0)
            ),
            0
        );
        assert_eq!(state.time.lock().disable_count_00, 0);
        // WA_INACTIVE.
        proc(
            &mut state,
            Some(&mut globals),
            &mut services,
            message(WM_ACTIVATE, 0, 0),
        );
        assert_eq!(globals.window_active_02, 0);
        assert_eq!(state.time.lock().disable_count_00, 1);
        // 00406134 tests LOWORD(wParam): a minimized deactivate still pauses.
        state.time.lock().disable_count_00 = 0;
        globals.window_active_02 = 1;
        proc(
            &mut state,
            Some(&mut globals),
            &mut services,
            message(WM_ACTIVATE, 0x0001_0000, 0),
        );
        assert_eq!(globals.window_active_02, 0);
        assert_eq!(state.time.lock().disable_count_00, 1);
        // 00B333F0 blocks the deactivate branch.
        state.activation_blocked = 1;
        globals.window_active_02 = 7;
        proc(
            &mut state,
            Some(&mut globals),
            &mut services,
            message(WM_ACTIVATE, 0, 0),
        );
        assert_eq!(globals.window_active_02, 7);
        assert_eq!(state.time.lock().disable_count_00, 1);

        services.consume_mouse = true;
        assert_eq!(
            proc(
                &mut state,
                Some(&mut globals),
                &mut services,
                message(WM_MOUSEMOVE, 0, 0x0567_1234)
            ),
            0
        );
        assert_eq!(
            services.mouse,
            Some((0x1234, 0x0567)),
            "WM_MOUSEMOVE must pass LOWORD(lParam) and lParam >> 16 to 00405370"
        );
        services.consume_mouse = false;
        proc(
            &mut state,
            Some(&mut globals),
            &mut services,
            message(WM_DESTROY, 0, 0),
        );
        assert!(globals.quit_game_00, "WM_DESTROY must set the quit flag");
    }

    /// The TimeInfo half of `verify_window`.
    #[test]
    fn verify_time_info() {
        let mut time = TimeInfoState {
            paused_ticks_10: 100,
            game_start_ticks_14: 40,
            ..TimeInfoState::default()
        };
        time_info_increment_disable_count(&mut time, 1000);
        assert_eq!(
            (
                time.disable_count_00,
                time.paused_ticks_10,
                time.game_start_ticks_14
            ),
            (1, 900, 960),
            "0047D120 must rebase from a zero count"
        );
        time_info_increment_disable_count(&mut time, 2000);
        assert_eq!(
            (time.disable_count_00, time.paused_ticks_10),
            (2, 900),
            "0047D120 must only count above zero"
        );
        time_info_decrement_disable_count(&mut time, 3000);
        time_info_decrement_disable_count(&mut time, 4000);
        assert_eq!(
            (
                time.disable_count_00,
                time.paused_ticks_10,
                time.game_start_ticks_14
            ),
            (0, 3100, 3040),
            "0047D0F0 must rebase when the count reaches zero"
        );
    }

    /// Not in the C++ test, which needs a real window for these: the default-procedure and cursor arms, and the order of
    /// the 00403160 / 004046A0 calls.
    #[test]
    fn default_procedure_cursor_and_poll_order() {
        let mut services = Services {
            def_result: 42,
            ..Services::default()
        };
        let mut state = WindowProcedureState::default();
        let proc = original_main_window_proc;
        // No device window: the default procedure's result.
        assert_eq!(
            proc(
                &mut state,
                None,
                &mut services,
                message(WM_ERASEBKGND, 0, 0)
            ),
            42
        );
        // An unlisted message.
        assert_eq!(
            proc(&mut state, None, &mut services, message(0x24, 0, 0)),
            42
        );
        // No renderer: 00497BF0 does not run, and the default procedure does.
        proc(
            &mut state,
            None,
            &mut services,
            message(WM_WINDOWPOSCHANGED, 0, 3),
        );
        assert!(services.positions.is_empty());
        state.renderer_present = true;
        proc(
            &mut state,
            None,
            &mut services,
            message(WM_WINDOWPOSCHANGED, 0, 3),
        );
        assert_eq!(services.positions, [(3, 7)]);
        assert_eq!(
            services.def_calls,
            [
                WM_ERASEBKGND,
                0x24,
                WM_WINDOWPOSCHANGED,
                WM_WINDOWPOSCHANGED
            ]
        );

        // WM_NCHITTEST with no input leaves the cursor alone.
        services.def_result = HTCLIENT;
        assert_eq!(
            proc(&mut state, None, &mut services, message(WM_NCHITTEST, 0, 0)),
            HTCLIENT
        );
        assert!(services.cursor_calls.is_empty());

        let mut settings = crate::tes::settings::SettingCollections::default();
        crate::tes::settings::register_static_settings(&mut settings);
        let input =
            OsInputGlobals::new(HostInstance(0x0040_0000), &settings, &mut services.platform)
                .unwrap();
        let mut globals = OsGlobalsData {
            input_20: Some(Shared::new(*input)),
            ..OsGlobalsData::default()
        };
        // Over the client area: ShowCursor(FALSE) until the count is not positive.
        services.cursor_counts = vec![0, 1, 2];
        proc(
            &mut state,
            Some(&mut globals),
            &mut services,
            message(WM_NCHITTEST, 0, 0),
        );
        assert_eq!(services.cursor_calls, [false, false, false]);
        assert!(!state.cursor_shown);
        // Again: no more calls.
        proc(
            &mut state,
            Some(&mut globals),
            &mut services,
            message(WM_NCHITTEST, 0, 0),
        );
        assert_eq!(services.cursor_calls.len(), 3);
        // Off the client area: one ShowCursor(TRUE).
        services.def_result = 2;
        proc(
            &mut state,
            Some(&mut globals),
            &mut services,
            message(WM_NCHITTEST, 0, 0),
        );
        assert_eq!(services.cursor_calls[3..], [true]);
        assert!(state.cursor_shown);

        // The activation arm discards, then polls with TimeInfo +0x10, before the TimeInfo call.
        services.platform.trace.clear();
        state.time.lock().paused_ticks_10 = 500;
        state.time.lock().disable_count_00 = 1;
        services.tick = 900;
        proc(
            &mut state,
            Some(&mut globals),
            &mut services,
            message(WM_ACTIVATE, 2, 0),
        );
        assert_eq!(services.platform.trace[0].site, 0x0040_3183);
        assert_eq!(services.platform.trace[1].site, 0x0040_4751);
        assert_eq!(state.time.lock().paused_ticks_10, 400);
    }

    /// Every value 00406360's clamp can store, against the correctly rounded float, by 24-bit reasoning.
    #[test]
    fn fps_clamp_range() {
        let mut time = TimeInfoState::default();
        assert!(!apply_fps_clamp(&mut time, 0));
        assert!(!apply_fps_clamp(&mut time, -1));
        assert!(!apply_fps_clamp(&mut time, 200));
        assert_eq!(time.fps_clamp_04.to_bits(), 0);
        assert!(apply_fps_clamp(&mut time, 199));
        assert_eq!(time.fps_clamp_04, 1000.0f32 / 199.0f32);
        assert!(apply_fps_clamp(&mut time, 1));
        assert_eq!(time.fps_clamp_04, 1000.0);
        for n in 1..=199 {
            assert!(apply_fps_clamp(&mut time, n));
            assert_eq!(time.fps_clamp_04, 1000.0f32 / n as f32);
        }
    }
}
