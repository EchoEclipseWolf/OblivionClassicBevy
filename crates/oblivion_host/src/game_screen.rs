//! IDA: N/A the host's one mapping between the window and the game's screen. The original drew into a back buffer of the
//! INI's resolution and took the pointer as a fraction of the client area (0057F490's arguments); the host's window can be any
//! size and any scale factor and can be resized. `GameScreen` holds the window as it is now (physical pixels, logical size,
//! scale factor), answers the pointer as 0..1 of the client (so a drag over the same fraction of the window moves the same
//! interface units at every size and scale) and is what the drawing follows: the interface's camera already projects the
//! renderer's pixels onto the window whatever its size, and `follow_window` keeps the world's image and its sprite at the
//! window's size when it changes.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

/// IDA: N/A the primary window as the game's screen sees it.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct GameScreen {
    /// The window's size in physical pixels (what the GPU draws and the cursor is measured in).
    pub physical: UVec2,
    /// Its size in logical units (physical over the scale factor), which the 2D cameras' default projections use.
    pub logical: Vec2,
    /// The window's scale factor.
    pub scale_factor: f32,
}

impl GameScreen {
    /// IDA: N/A the screen of a window.
    #[must_use]
    pub fn from_window(window: &Window) -> Self {
        Self {
            physical: UVec2::new(window.physical_width(), window.physical_height()),
            logical: Vec2::new(window.resolution.width(), window.resolution.height()),
            scale_factor: window.resolution.scale_factor(),
        }
    }

    /// IDA: N/A a screen for a test or a window that isn't up: `physical` pixels at `scale_factor`.
    #[must_use]
    pub fn new(physical: UVec2, scale_factor: f32) -> Self {
        Self {
            physical,
            logical: physical.as_vec2() / scale_factor,
            scale_factor,
        }
    }

    /// IDA: N/A the pointer as 0057F490 takes it: a cursor position in physical pixels as 0..1 of the client area on each
    /// axis, `None` with no area. Any window size or scale factor gives the same fraction for the same place.
    #[must_use]
    pub fn pointer_fraction(&self, physical_cursor: Vec2) -> Option<[f32; 2]> {
        let size = self.physical.as_vec2();
        (size.x > 0.0 && size.y > 0.0).then(|| (physical_cursor / size).to_array())
    }

    /// IDA: N/A the same for a cursor in logical units (what Bevy's `Window::cursor_position` answers).
    #[must_use]
    pub fn pointer_fraction_logical(&self, logical_cursor: Vec2) -> Option<[f32; 2]> {
        self.pointer_fraction(logical_cursor * self.scale_factor)
    }

    /// IDA: N/A the size a full-window render target takes: the physical pixels, at least 1 x 1.
    #[must_use]
    pub fn target_size(&self) -> UVec2 {
        self.physical.max(UVec2::ONE)
    }
}

/// IDA: N/A a render target that covers the window, with the sprite that shows it: `follow_window` resizes both.
#[derive(Component, Clone, Debug)]
pub struct WindowSizedTarget {
    pub image: Handle<Image>,
}

/// IDA: N/A the plugin: `GameScreen` from the primary window each frame, and the window-sized targets following it.
pub struct GameScreenPlugin;

impl Plugin for GameScreenPlugin {
    /// IDA: N/A Rust's `Plugin`.
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, follow_window);
    }
}

/// IDA: N/A `GameScreen` from the primary window; when its size or scale changed, every `WindowSizedTarget`'s image takes the
/// new physical size and its sprite the new logical size, so the world keeps covering the window at full resolution.
pub fn follow_window(
    mut commands: Commands,
    windows: Query<&Window, With<PrimaryWindow>>,
    screen: Option<Res<GameScreen>>,
    mut images: ResMut<Assets<Image>>,
    mut targets: Query<(&WindowSizedTarget, &mut Sprite)>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let now = GameScreen::from_window(window);
    if screen.as_deref() == Some(&now) {
        return;
    }
    commands.insert_resource(now);
    let size = now.target_size();
    for (target, mut sprite) in &mut targets {
        if let Some(mut image) = images.get_mut(&target.image) {
            let extent = bevy::render::render_resource::Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            };
            if image.texture_descriptor.size != extent {
                image.resize(extent);
            }
        }
        sprite.custom_size = Some(now.logical);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The same place in the window is the same fraction at every size and scale factor, so a drag over a fraction of the
    // window moves the interface's cursor equally.
    #[test]
    fn the_pointer_fraction_does_not_depend_on_the_window_s_size_or_scale() {
        let screens = [
            GameScreen::new(UVec2::new(1920, 1080), 1.0),
            GameScreen::new(UVec2::new(1280, 720), 1.0),
            GameScreen::new(UVec2::new(3840, 2160), 2.0),
            GameScreen::new(UVec2::new(2880, 1620), 1.5),
        ];
        for screen in screens {
            let size = screen.physical.as_vec2();
            let at = |fraction: Vec2| screen.pointer_fraction(fraction * size).unwrap();
            assert_eq!(at(Vec2::new(0.5, 0.25)), [0.5, 0.25], "{screen:?}");
            let from = at(Vec2::new(0.6, 0.5));
            let to = at(Vec2::new(0.6625, 0.5));
            assert!((to[0] - from[0] - 0.0625).abs() < 1e-6, "{screen:?}");
            let logical = screen
                .pointer_fraction_logical(Vec2::new(0.5, 0.25) * screen.logical)
                .unwrap();
            assert!(
                (logical[0] - 0.5).abs() < 1e-6 && (logical[1] - 0.25).abs() < 1e-6,
                "{screen:?}"
            );
        }
        assert_eq!(
            GameScreen::new(UVec2::ZERO, 1.0).pointer_fraction(Vec2::ZERO),
            None
        );
    }
}
