//! IDA: N/A Bevy's input as DirectInput answers it: the recorded substitution in the plan's table. Each frame, before the WinMain chain, the keyboard's held keys become `DIK_*` bytes of 0x80 and the mouse's motion adds to what `GetDeviceState` will take. The game's own `OsInputGlobals` then polls as the original does, on the main thread and on the movie thread. `RawInputFrame` is the same data as a plain resource, for the screens.
//!
//! Host substitutions, recorded: joysticks aren't enumerated; wheel lines are 120 of lZ, the Windows notch, and a pixel of scroll is one; the mouse isn't taken from the system, so `bBackground Mouse=0`'s exclusive mode only means it reads as nothing without the focus.

use std::sync::{Arc, Mutex};

use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, Window};
use oblivion_sim::tes::os_globals::lock_shared;
use oblivion_sim::tes::winmain::winmain_host::HostInputFrame;

/// IDA: N/A the frame's raw input, for the screens: the DirectInput-shaped bytes and the window's cursor. `previous_keyboard` lets a reader see a press without polling the game's input.
#[derive(Resource, Clone, Debug)]
pub struct RawInputFrame {
    /// Indexed by `DIK_*`: 0x80 while held.
    pub keyboard: [u8; 256],
    pub previous_keyboard: [u8; 256],
    /// This frame's motion, in mouse counts.
    pub mouse_delta: IVec2,
    /// What's left of the motion under a whole count, for the next frame.
    pub mouse_fraction: Vec2,
    /// This frame's wheel: 120 a notch.
    pub wheel: i32,
    /// Left, right, middle, back, forward: 0x80 while held.
    pub mouse_buttons: [u8; 8],
    pub previous_mouse_buttons: [u8; 8],
    /// The window's own cursor, in physical pixels from the top left, while it's over the window.
    pub cursor: Option<Vec2>,
    pub focused: bool,
    /// Counts the frames this has been filled.
    pub frame: u64,
}

impl Default for RawInputFrame {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            keyboard: [0; 256],
            previous_keyboard: [0; 256],
            mouse_delta: IVec2::ZERO,
            mouse_fraction: Vec2::ZERO,
            wheel: 0,
            mouse_buttons: [0; 8],
            previous_mouse_buttons: [0; 8],
            cursor: None,
            focused: true,
            frame: 0,
        }
    }
}

impl RawInputFrame {
    /// IDA: N/A a key is held.
    #[must_use]
    pub const fn held(&self, dik: u8) -> bool {
        self.keyboard[dik as usize] & 0x80 != 0
    }

    /// IDA: N/A a key went down this frame.
    #[must_use]
    pub const fn just_pressed(&self, dik: u8) -> bool {
        self.held(dik) && self.previous_keyboard[dik as usize] & 0x80 == 0
    }

    /// IDA: N/A a mouse button went down this frame.
    #[must_use]
    pub const fn button_just_pressed(&self, button: usize) -> bool {
        self.mouse_buttons[button] & 0x80 != 0 && self.previous_mouse_buttons[button] & 0x80 == 0
    }
}

/// IDA: N/A `DIK_*` from dinput.h for a key: the scan codes the original's control bindings and `query_control_state` use. Completion: 100% of the keys a PC keyboard has; a key with no scan code answers `None`.
#[must_use]
pub const fn dik_for_key(key: KeyCode) -> Option<u8> {
    Some(match key {
        KeyCode::Escape => 0x01,
        KeyCode::Digit1 => 0x02,
        KeyCode::Digit2 => 0x03,
        KeyCode::Digit3 => 0x04,
        KeyCode::Digit4 => 0x05,
        KeyCode::Digit5 => 0x06,
        KeyCode::Digit6 => 0x07,
        KeyCode::Digit7 => 0x08,
        KeyCode::Digit8 => 0x09,
        KeyCode::Digit9 => 0x0A,
        KeyCode::Digit0 => 0x0B,
        KeyCode::Minus => 0x0C,
        KeyCode::Equal => 0x0D,
        KeyCode::Backspace => 0x0E,
        KeyCode::Tab => 0x0F,
        KeyCode::KeyQ => 0x10,
        KeyCode::KeyW => 0x11,
        KeyCode::KeyE => 0x12,
        KeyCode::KeyR => 0x13,
        KeyCode::KeyT => 0x14,
        KeyCode::KeyY => 0x15,
        KeyCode::KeyU => 0x16,
        KeyCode::KeyI => 0x17,
        KeyCode::KeyO => 0x18,
        KeyCode::KeyP => 0x19,
        KeyCode::BracketLeft => 0x1A,
        KeyCode::BracketRight => 0x1B,
        KeyCode::Enter => 0x1C,
        KeyCode::ControlLeft => 0x1D,
        KeyCode::KeyA => 0x1E,
        KeyCode::KeyS => 0x1F,
        KeyCode::KeyD => 0x20,
        KeyCode::KeyF => 0x21,
        KeyCode::KeyG => 0x22,
        KeyCode::KeyH => 0x23,
        KeyCode::KeyJ => 0x24,
        KeyCode::KeyK => 0x25,
        KeyCode::KeyL => 0x26,
        KeyCode::Semicolon => 0x27,
        KeyCode::Quote => 0x28,
        KeyCode::Backquote => 0x29,
        KeyCode::ShiftLeft => 0x2A,
        KeyCode::Backslash => 0x2B,
        KeyCode::KeyZ => 0x2C,
        KeyCode::KeyX => 0x2D,
        KeyCode::KeyC => 0x2E,
        KeyCode::KeyV => 0x2F,
        KeyCode::KeyB => 0x30,
        KeyCode::KeyN => 0x31,
        KeyCode::KeyM => 0x32,
        KeyCode::Comma => 0x33,
        KeyCode::Period => 0x34,
        KeyCode::Slash => 0x35,
        KeyCode::ShiftRight => 0x36,
        KeyCode::NumpadMultiply => 0x37,
        KeyCode::AltLeft => 0x38,
        KeyCode::Space => 0x39,
        KeyCode::CapsLock => 0x3A,
        KeyCode::F1 => 0x3B,
        KeyCode::F2 => 0x3C,
        KeyCode::F3 => 0x3D,
        KeyCode::F4 => 0x3E,
        KeyCode::F5 => 0x3F,
        KeyCode::F6 => 0x40,
        KeyCode::F7 => 0x41,
        KeyCode::F8 => 0x42,
        KeyCode::F9 => 0x43,
        KeyCode::F10 => 0x44,
        KeyCode::NumLock => 0x45,
        KeyCode::ScrollLock => 0x46,
        KeyCode::Numpad7 => 0x47,
        KeyCode::Numpad8 => 0x48,
        KeyCode::Numpad9 => 0x49,
        KeyCode::NumpadSubtract => 0x4A,
        KeyCode::Numpad4 => 0x4B,
        KeyCode::Numpad5 => 0x4C,
        KeyCode::Numpad6 => 0x4D,
        KeyCode::NumpadAdd => 0x4E,
        KeyCode::Numpad1 => 0x4F,
        KeyCode::Numpad2 => 0x50,
        KeyCode::Numpad3 => 0x51,
        KeyCode::Numpad0 => 0x52,
        KeyCode::NumpadDecimal => 0x53,
        KeyCode::IntlBackslash => 0x56,
        KeyCode::F11 => 0x57,
        KeyCode::F12 => 0x58,
        KeyCode::NumpadEnter => 0x9C,
        KeyCode::ControlRight => 0x9D,
        KeyCode::NumpadDivide => 0xB5,
        KeyCode::PrintScreen => 0xB7,
        KeyCode::AltRight => 0xB8,
        KeyCode::Pause => 0xC5,
        KeyCode::Home => 0xC7,
        KeyCode::ArrowUp => 0xC8,
        KeyCode::PageUp => 0xC9,
        KeyCode::ArrowLeft => 0xCB,
        KeyCode::ArrowRight => 0xCD,
        KeyCode::End => 0xCF,
        KeyCode::ArrowDown => 0xD0,
        KeyCode::PageDown => 0xD1,
        KeyCode::Insert => 0xD2,
        KeyCode::Delete => 0xD3,
        KeyCode::SuperLeft => 0xDB,
        KeyCode::SuperRight => 0xDC,
        KeyCode::ContextMenu => 0xDD,
        _ => return None,
    })
}

/// IDA: N/A a mouse button's place in DIMOUSESTATE2's rgbButtons.
const fn dinput_button(button: MouseButton) -> Option<usize> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Right => Some(1),
        MouseButton::Middle => Some(2),
        MouseButton::Back => Some(3),
        MouseButton::Forward => Some(4),
        MouseButton::Other(number) if 3 <= number && number < 8 => Some(number as usize),
        MouseButton::Other(_) => None,
    }
}

/// IDA: N/A the input handle: `oblivion_sim::tes::winmain::os_input_frame`, fetched when the plugin builds.
#[derive(Resource, Clone)]
pub struct HostInputHandle(pub Arc<Mutex<HostInputFrame>>);

/// IDA: N/A Bevy's frame into `RawInputFrame` and `HostInputFrame`. Completion: 100% of the host role.
#[allow(clippy::too_many_arguments)] // One Bevy input resource each.
pub fn feed_raw_input(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut raw: ResMut<RawInputFrame>,
    handle: Option<Res<HostInputHandle>>,
) {
    let window = windows.single().ok();
    raw.previous_keyboard = raw.keyboard;
    raw.previous_mouse_buttons = raw.mouse_buttons;
    raw.keyboard = [0; 256];
    for key in keys.get_pressed() {
        if let Some(dik) = dik_for_key(*key) {
            raw.keyboard[dik as usize] = 0x80;
        }
    }
    raw.mouse_buttons = [0; 8];
    for button in buttons.get_pressed() {
        if let Some(index) = dinput_button(*button) {
            raw.mouse_buttons[index] = 0x80;
        }
    }
    let motion = raw.mouse_fraction + motion.delta;
    let counts = motion.trunc();
    raw.mouse_fraction = motion - counts;
    raw.mouse_delta = counts.as_ivec2();
    raw.wheel = match scroll.unit {
        MouseScrollUnit::Line => (scroll.delta.y * 120.0) as i32,
        MouseScrollUnit::Pixel => scroll.delta.y as i32,
    };
    raw.cursor = window.and_then(Window::physical_cursor_position);
    raw.focused = window.is_none_or(|window| window.focused);
    raw.frame += 1;
    if let Some(handle) = handle {
        feed_host_input(&mut lock_shared(&handle.0), &raw);
    }
}

/// IDA: N/A one frame into what `GetDeviceState` answers. The axes add, because a poll may not come this frame. Completion: 100%.
pub fn feed_host_input(frame: &mut HostInputFrame, raw: &RawInputFrame) {
    frame.keyboard = raw.keyboard;
    frame.mouse.x = frame.mouse.x.wrapping_add(raw.mouse_delta.x);
    frame.mouse.y = frame.mouse.y.wrapping_add(raw.mouse_delta.y);
    frame.mouse.z = frame.mouse.z.wrapping_add(raw.wheel);
    frame.mouse.buttons = raw.mouse_buttons;
    frame.focused = raw.focused;
}

#[cfg(test)]
mod tests {
    use super::*;
    use oblivion_sim::tes::os_globals::{
        DISCL_FOREGROUND, DiMouseState2, HostInstance, HostWindow, InputDevice, OsGlobalsPlatform,
        OsInputGlobals, scripted_input_settings,
    };
    use oblivion_sim::tes::winmain::winmain_host::{DIERR_OTHERAPPHASPRIO, HostOsPlatform};

    #[test]
    fn dik_codes_match_dinput() {
        assert_eq!(dik_for_key(KeyCode::Escape), Some(0x01));
        assert_eq!(dik_for_key(KeyCode::Space), Some(0x39));
        assert_eq!(dik_for_key(KeyCode::KeyW), Some(0x11));
        assert_eq!(dik_for_key(KeyCode::Enter), Some(0x1C));
        assert_eq!(
            dik_for_key(KeyCode::Backquote),
            Some(0x29),
            "the console's grave"
        );
        assert_eq!(dik_for_key(KeyCode::PrintScreen), Some(0xB7), "SysRq");
        assert_eq!(dik_for_key(KeyCode::ArrowDown), Some(0xD0));
        assert_eq!(dik_for_key(KeyCode::Fn), None);
    }

    /// Raw input through the real poll to the game's own state: the key, the axes taken once, the buttons staying, and control 29 (Escape) just-pressed.
    #[test]
    fn a_frame_reaches_the_game_s_input() {
        let settings = scripted_input_settings(true);
        let mut platform = HostOsPlatform::default();
        let mut input =
            OsInputGlobals::new(HostInstance(0x0040_0000), &settings, &mut platform).unwrap();
        input.acquire_exclusive_mouse = true;
        let mut raw = RawInputFrame::default();
        raw.keyboard[0x01] = 0x80;
        raw.mouse_delta = IVec2::new(5, -3);
        raw.wheel = 120;
        raw.mouse_buttons[0] = 0x80;
        feed_host_input(&mut lock_shared(&platform.input), &raw);
        input.poll_devices(0, &mut platform);
        assert_eq!(input.keyboard_state_18f4[0x01], 0x80);
        assert_eq!(
            (
                input.mouse_state_1b20.x,
                input.mouse_state_1b20.y,
                input.mouse_state_1b20.z
            ),
            (5, -3, 120)
        );
        assert_eq!(input.mouse_state_1b20.buttons[0], 0x80);
        assert_eq!(
            input.query_control_state(29, OsInputGlobals::QUERY_PRESSED),
            1
        );
        // The second poll, with no new frame: the axes are gone, the button stays, and Escape is no longer new.
        input.poll_devices(0, &mut platform);
        assert_eq!((input.mouse_state_1b20.x, input.mouse_state_1b20.z), (0, 0));
        assert_eq!(input.mouse_state_1b20.buttons[0], 0x80);
        assert_eq!(
            input.query_control_state(29, OsInputGlobals::QUERY_PRESSED),
            0
        );
    }

    /// Slow motion, under a count a frame: the fractions add up, as DirectInput's counts do, in both directions.
    #[test]
    fn a_fraction_of_a_count_carries_to_the_next_frame() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_resource::<ButtonInput<MouseButton>>();
        app.init_resource::<AccumulatedMouseMotion>();
        app.init_resource::<AccumulatedMouseScroll>();
        app.init_resource::<RawInputFrame>();
        app.add_systems(Update, feed_raw_input);
        let mut run = |x: f32| {
            app.world_mut()
                .resource_mut::<AccumulatedMouseMotion>()
                .delta = Vec2::new(x, -x);
            app.update();
            app.world().resource::<RawInputFrame>().mouse_delta
        };
        assert_eq!(run(0.5), IVec2::ZERO);
        assert_eq!(run(0.25), IVec2::ZERO);
        assert_eq!(run(0.5), IVec2::new(1, -1));
        assert_eq!(run(2.5), IVec2::new(2, -2), "0.25 + 2.5");
        assert_eq!(run(0.25), IVec2::new(1, -1), "0.75 + 0.25");
    }

    /// Axes add up across frames with no poll, as DirectInput's state does.
    #[test]
    fn motion_adds_until_taken() {
        let mut frame = HostInputFrame::default();
        let raw = RawInputFrame {
            mouse_delta: IVec2::new(2, 1),
            ..RawInputFrame::default()
        };
        feed_host_input(&mut frame, &raw);
        feed_host_input(&mut frame, &raw);
        assert_eq!((frame.mouse.x, frame.mouse.y), (4, 2));
        let mut platform = HostOsPlatform {
            input: Arc::new(Mutex::new(frame)),
            ..HostOsPlatform::default()
        };
        let mut state = DiMouseState2::default();
        platform.get_mouse_state(0, InputDevice(2), &mut state);
        assert_eq!((state.x, state.y), (4, 2));
        platform.get_mouse_state(0, InputDevice(2), &mut state);
        assert_eq!((state.x, state.y), (0, 0));
    }

    /// 004046DE's other branch: a foreground device without the focus fails Acquire, and the poll zeroes the state.
    #[test]
    fn losing_focus_zeroes_a_foreground_device() {
        let mut platform = HostOsPlatform::default();
        platform.set_cooperative_level(0, InputDevice(1), HostWindow(1), 0x15 | DISCL_FOREGROUND);
        lock_shared(&platform.input).focused = false;
        assert_eq!(platform.acquire(0, InputDevice(1)), DIERR_OTHERAPPHASPRIO);
        lock_shared(&platform.input).focused = true;
        assert_eq!(platform.acquire(0, InputDevice(1)), 0);
    }

    #[test]
    fn just_pressed_needs_a_new_press() {
        let mut raw = RawInputFrame::default();
        raw.keyboard[0x39] = 0x80;
        assert!(raw.just_pressed(0x39));
        raw.previous_keyboard[0x39] = 0x80;
        assert!(!raw.just_pressed(0x39));
        assert!(raw.held(0x39));
    }
}
