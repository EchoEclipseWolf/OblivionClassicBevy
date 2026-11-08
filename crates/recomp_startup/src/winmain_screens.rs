//! IDA: N/A the first screens after `Block::Finished`: the idle loop's interface calls, a frame at a time.

use bevy_ecs::prelude::*;
use recomp_core::logging::{Level, emit};
use recomp_core::reader::StreamError;
use recomp_game::tes::interface_frame::{
    InterfaceInputFrame, interface_advance_frame, interface_update_frame,
    interface_update_menu_visibility,
};
use recomp_game::tes::interface_manager::{InterfaceManager, interface_render_if_no_movie};

use crate::winmain::{
    WinMainProgress, driver_frame_seconds, menu_media_context, with_driver_interface,
};

/// IDA: N/A where the sequence is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScreenStage {
    /// The chain hasn't finished, or stopped. The LoadingMenu's own calls draw.
    #[default]
    Loading,
    /// The idle loop's frames.
    Menu,
    /// 0040F0DB: the player has a cell, so WinMain makes no MainMenu. The Phase 17 side's.
    NoMenu,
}

/// IDA: N/A the sequence's state.
#[derive(Resource, Clone, Debug, Default)]
pub struct ScreenDirector {
    pub stage: ScreenStage,
    pub frames: u64,
}

/// IDA: N/A which screen follows `Block::MainMenu`, which ran 0040F08D. Once, in the frame the chain ends, before `run_main_menu_media_frame`. A host stop in the chain leaves the LoadingMenu up, as `main.cpp`'s nested `try`s do; an exit has no screen. Completion: 100%.
pub fn open_main_menu(world: &mut World) -> Result<(), StreamError> {
    let progress = world.resource::<WinMainProgress>();
    if !progress.finished() || !progress.stops.is_empty() || progress.exit_code.is_some() {
        return Ok(());
    }
    if world.get_resource_or_init::<ScreenDirector>().stage != ScreenStage::Loading {
        return Ok(());
    }
    // No manager: the chain stopped short of 0040EB18 without saying so.
    let stage = if !world.contains_resource::<InterfaceManager>() {
        ScreenStage::NoMenu
    } else {
        // 0040F0E5, on both of 0040F0D9's and 0040F0DB's paths.
        emit(Level::Info, "Begin Idle loop...");
        if menu_media_context(world).is_some_and(|context| context.player_has_cell) {
            ScreenStage::NoMenu
        } else {
            ScreenStage::Menu
        }
    };
    world.resource_mut::<ScreenDirector>().stage = stage;
    Ok(())
}

/// IDA: a frame of the idle loop (0040F130, 0040D800) for the interface, which `the_port_matches_the_phase_22_rows` compares once, after `run_main_menu_media_frame`: 005791A0 (00583E60), 005791E0 (005821F0), 00579220 (00583F40), and 0040D4D0's 00579260(0). `frame_time` is 00B33E9C, which the media frame's 0047D170 just set. Completion: 100% of the order.
pub fn run_screens_frame(
    world: &mut World,
    input: &InterfaceInputFrame,
) -> Result<(), StreamError> {
    let stage = world
        .get_resource::<ScreenDirector>()
        .map(|director| director.stage);
    let Some(stage) = stage else {
        return Ok(());
    };
    if stage == ScreenStage::Loading || stage == ScreenStage::NoMenu {
        return Ok(());
    }
    world.resource_mut::<ScreenDirector>().frames += 1;
    let frame_time = driver_frame_seconds(world);
    let input = InterfaceInputFrame {
        frame_time,
        ..input.clone()
    };
    with_driver_interface(world, |cx| -> Result<(), StreamError> {
        interface_advance_frame(cx, frame_time)?;
        interface_update_frame(cx, &input)?;
        interface_update_menu_visibility(cx)?;
        interface_render_if_no_movie(cx, 0)
    })
    .transpose()?;
    Ok(())
}
