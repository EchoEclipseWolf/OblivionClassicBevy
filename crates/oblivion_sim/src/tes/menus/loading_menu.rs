//! Phase 07: LoadingMenu's own state, 005AD0D0 and the progress functions later phases call. 005ADCF0
//! LoadingMenu_Create is in `interface_manager.rs`, with the manager it needs. The C++ host view
//! (`LoadingMenu::create`,
//! `layout`, `adopt` and its DOM path) is not here: M6 draws the tile tree that 005ADCF0 builds.

use bevy_ecs::prelude::{Entity, World};
use oblivion_core::error::GameError;
use oblivion_core::logging::{Level, emit};
use oblivion_core::x87;

use crate::tes::data_handler::{DataHandlerLists, TesTexture};
use crate::tes::forms_environment::{TesLoadScreen, screen_matches};
use crate::tes::interface_frame::{interface_remove_menu_mode, menu_destroy};
use crate::tes::interface_manager::{
    InterfaceManager, interface_movie_gate, interface_render_if_no_movie,
    interface_set_cursor_alpha, manager_mut, menu_register_tile, menu_update_cursor_depth,
};
use crate::tes::resource_purge::purge_after_loading;
use crate::tes::tes::TesGlobals;
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileEngine};
use crate::tes::tile_values::*;
use crate::tes::tile_values::{
    TRAIT_USER0, TRAIT_USER3, TRAIT_VISIBLE, tile_numeric_trait, tile_set_numeric_trait,
    tile_set_string_trait, tile_update_scene_graph,
};
use crate::tes::tile_xml::tile_read_xml;
use bevy_ecs::prelude::*;

/// IDA: 005AD0D0 LoadingMenu_ctor's fields. +0x28 is the current section and +0x3C the target percent (5adf32, 5ad9e8,
/// 5ae02d); the C++ port's names and the rows' header have the two swapped, and both start at -1. The 0x18 timer object at
/// +0x54 is not modelled: its +0x10 (0047D170) is `tick - start` while its bytes +0 and +4 stay 0, which nothing in this
/// class changes, so `last_step_68` and `last_update_6c` hold ticks and every difference is the same mod 2^32. A host
/// substitution. Completion: 100% of the state the progress functions read.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadingMenu {
    /// flt_B3B3DC, the percentages 005AD18D reads into the global.
    pub section_weights_b3b3dc: [f32; 4],
    /// +0x28.
    pub current_section_28: i32,
    /// +0x2C..+0x38: the section percentages.
    pub sections_2c: [i32; 4],
    /// +0x3C.
    pub target_3c: i32,
    /// +0x40: seconds until the next slide, from flt_B14158. A float: 5ada26 stores it each tick.
    pub slide_remaining_40: f32,
    /// +0x60.
    pub displayed_60: i32,
    /// +0x64.
    pub pending_64: i32,
    pub last_step_68: u32,
    pub last_update_6c: u32,
    /// +0x70: inside 005ADB40's loop.
    pub completing_70: bool,
    /// +0x71: 5ad141 stores 1; 005ADB40 runs 00404C60 once and clears it.
    pub first_completion_71: bool,
    pub slide_delay_b14158: f32,
    /// +0x44 = slideContext (005ADE17).
    pub slide_context_44: u32,
    /// IDA: N/A +0x44's form, which the slide draw reads.
    pub slide_context: Option<Entity>,
    /// +0x4C: the slides 005AD440 chose, in order.
    pub slides_4c: Vec<bevy_ecs::prelude::Entity>,
}

impl LoadingMenu {
    /// IDA: the state stores of 005AD0D0: +0x3C and +0x28 -1, the sections 0, the tick in +0x68 and +0x6C, +0x40 =
    /// flt_B14158, +0x70 0, +0x71 1. Completion: 100%.
    #[must_use]
    pub fn new(now: u32, section_weights: [f32; 4], slide_delay: f32) -> Self {
        Self {
            section_weights_b3b3dc: section_weights,
            current_section_28: -1,
            sections_2c: [0; 4],
            target_3c: -1,
            slide_remaining_40: slide_delay,
            displayed_60: 0,
            pending_64: 0,
            last_step_68: now,
            last_update_6c: now,
            completing_70: false,
            first_completion_71: true,
            slide_delay_b14158: slide_delay,
            slide_context_44: 0,
            slide_context: None,
            slides_4c: Vec::new(),
        }
    }

    /// IDA: 005ADEC0 LoadingMenu_SetSectionProgress(int, float). The C++ port rejects invalid input, and so does this.
    /// Completion: 100% of the section arithmetic.
    #[allow(clippy::assign_op_pattern)] // 5ae018's operand order: the product first.
    pub fn set_section_progress(&mut self, section: i32, percent: f32) -> Result<(), GameError> {
        if !(0..4).contains(&section) || !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
            return Err(GameError::unclassified(
                0,
                "Invalid LoadingMenu section/percent",
            ));
        }
        let previous_section = self.current_section_28;
        if section < self.current_section_28 {
            emit(
                Level::Warning,
                &format!(
                    "LoadingMenu sections out of order: {section} after {}",
                    self.current_section_28
                ),
            );
            self.current_section_28 = section;
            self.sections_2c[section as usize + 1..].fill(0);
        }
        while self.current_section_28 < section {
            if self.current_section_28 != -1 {
                self.sections_2c[self.current_section_28 as usize] = 100;
            }
            self.current_section_28 += 1;
        }
        if self.current_section_28 != previous_section {
            emit(
                Level::Info,
                &format!(
                    "LoadingMenu slider entered section {} of 4",
                    self.current_section_28
                ),
            );
        }
        let current = &mut self.sections_2c[section as usize];
        if percent > *current as f32 {
            *current = x87::ftol(percent);
        }
        let mut sum = 0.0f32;
        for (&value, &weight) in self.sections_2c.iter().zip(&self.section_weights_b3b3dc) {
            // 5ae008..5ae01c fild, fmul dword, fadd dword at 24 bits: both round to float. The C++ differs: it keeps
            // the product unrounded through the sum.
            sum = value as f32 * weight + sum;
        }
        self.target_3c = x87::ftol(sum);
        Ok(())
    }

    /// IDA: 005AD980 void LoadingMenu_UpdateDisplay(char force): the tick decisions, returning what its tile writes do.
    /// `move_bar_ms` is 00B14160, 00B14168 and 00B14170. Every clock difference wraps (5ad9cf, 5ad9d4) and compares signed.
    /// The C++ differs: it stops counting at 100 and rejects a clock that goes back; the exe does neither. Completion:
    /// 100% of the tick decisions.
    pub fn update_display_state(
        &mut self,
        force: bool,
        now: u32,
        move_bar_ms: [i32; 3],
    ) -> LoadingDisplayStep {
        let mut step = LoadingDisplayStep::default();
        let elapsed = now.wrapping_sub(self.last_step_68) as i32;
        let since_update = now.wrapping_sub(self.last_update_6c) as i32;
        // 5ad9d7..5ada06.
        if elapsed > move_bar_ms[0]
            || (self.pending_64 < self.target_3c
                && (elapsed > move_bar_ms[1] || (self.completing_70 && elapsed > move_bar_ms[2])))
        {
            self.pending_64 = self.pending_64.wrapping_add(1);
        }
        // 5ada12 fild (exact), 5ada16 `jge` over 5ada18 `fadd dword 2^32` for a negative difference, 5ada1e `fdiv qword
        // 1000.0`, 5ada24 fsubp, 5ada26 fstp dword: 24-bit, one rounding per step. A 53-bit quotient cannot round
        // differently here: n / 1000 for a 32-bit n is at least 2^-35 relative from any 25-bit midpoint, or exactly on it.
        let elapsed_seconds = if since_update < 0 {
            let wrapped = (f64::from(since_update) + 4_294_967_296.0) as f32;
            (f64::from(wrapped) / 1000.0) as f32
        } else {
            (f64::from(since_update) / 1000.0) as f32
        };
        self.slide_remaining_40 -= elapsed_seconds;
        // 5ada31..5ada48 `fldz; fcompp; test ah, 1`: a NaN is not due without the force byte.
        let slide_due = self.slide_remaining_40 <= 0.0 || force;
        if (self.pending_64 != self.displayed_60 || slide_due) && self.displayed_60 < 100 {
            if self.displayed_60 != self.pending_64 {
                self.last_step_68 = now;
                self.displayed_60 = self.pending_64;
            }
            let shown = self.displayed_60 as f32;
            step.ribbon = Some(shown);
            // 5ada93 `fcomp qword 99.9999`, `test ah, 5; jp`: at 100 the slide and the render block are skipped.
            if f64::from(shown) < 99.999_900_817_871_1 {
                step.advance_slide = slide_due;
                step.changed = true;
            }
        }
        self.last_update_6c = now;
        step
    }

    /// IDA: 005ADB96..005ADB9D: +0x70 = 1 and +0x3C = 100. Completion: 100%.
    pub const fn begin_completion(&mut self) {
        self.completing_70 = true;
        self.target_3c = 100;
    }
}

/// IDA: N/A what one 005AD980 does to the menu's tile.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LoadingDisplayStep {
    pub changed: bool,
    /// user3 (0xFB1), the displayed percent.
    pub ribbon: Option<f32>,
    /// 005AD780.
    pub advance_slide: bool,
}

/// IDA: 005AD0D0 LoadingMenu_ctor, for 00587D20's case 1007: Menu_Constructor, the state stores,
/// OSSoundGlobals_PauseGameSounds when g_osGlobals has its sound object, the four [LoadingBar] percentages (off_B14120,
/// a non-null
/// static list) summed at 24 bits, the warning when the sum is not below 0.01 from 1.0 (5ad1c0 / 5ad1c8 subtract at 24
/// bits, 5ad1d7 `jnp`, so a NaN warns too), and the two ticks. The 0x18 timer object of 0047D150 and 0047D170 is only its
/// tick here. Completion: 100% of the state.
#[allow(clippy::assign_op_pattern)] // 005AD19A's operand order: the setting first.
pub fn construct_loading_menu(cx: &mut InterfaceCx) -> LoadingMenu {
    if cx.settings.sound_present {
        cx.host.pause_game_sounds();
    }
    let mut total = 0.0f32;
    for &weight in &cx.settings.loading_bar_percentages {
        total = weight + total;
    }
    // 5ad1bb `test ah, 0x41; jne`: not above 1.0, or a NaN, takes 1.0 - total.
    let difference = if total > 1.0 {
        total - 1.0
    } else {
        1.0 - total
    };
    if !f64::from(difference).lt(&0.009_999_999_776_482_582) {
        // 005AD1EF.. the buffer DebugMessage_Warning prints.
        let mut text = String::from(
            "Total value of [LoadingBar] percentages is not equal to 1.0000f in ini file.\r\n",
        );
        for (index, weight) in cx.settings.loading_bar_percentages.iter().enumerate() {
            text.push_str(&format!(
                "\tfPercentageOfBar{index}:LoadingBar - {weight:.4}\r\n"
            ));
        }
        text.push_str(&format!("INI Total = {total:.4}"));
        cx.host.warning(&text);
    }
    let now = cx.host.tick_count();
    LoadingMenu::new(
        now,
        cx.settings.loading_bar_percentages,
        cx.settings.loading_slide_delay,
    )
}

/// IDA: 005AD4C0..005AD65F, the slide draw for a context: its world (an interior cell's is null; a worldspace is its own). Each load screen of DataHandler +0x9C, in list order: matching the context (004F9BB0), one with no locations (004F9A20) is prepended to the general list, else to the context's; failing that, one matching the world is prepended to the world's. Three passes: one draw from the context's list, one from the world's, then every general one. A draw is `Random_Next` (005AD5CA) `idiv` what's left; the node at the remainder is appended to +0x4C and removed (0065C620). Completion: 100% for a null context, an interior cell or a worldspace.
fn draw_slides(
    world: &World,
    context: Option<Entity>,
    random: &mut dyn FnMut() -> Result<i32, GameError>,
) -> Result<Vec<Entity>, GameError> {
    let space = match context {
        Some(form) if crate::tes::forms_init::form_type(world, form) == 0x30 => {
            crate::tes::forms_world::cell_world(world, Some(form))
        }
        Some(form) if crate::tes::forms_init::form_type(world, form) == 0x35 => Some(form),
        Some(_) => {
            return Err(GameError::unclassified(
                0x005AD4AD,
                "005AD4AD: a reference's slide context",
            ));
        }
        None => None,
    };
    let mut general: Vec<Entity> = Vec::new();
    let mut local: Vec<Entity> = Vec::new();
    let mut regional: Vec<Entity> = Vec::new();
    if let Some(lists) = world.get_resource::<DataHandlerLists>() {
        for form in lists.load_screens_9c.items() {
            let Some(screen) = world.get::<TesLoadScreen>(form) else {
                continue;
            };
            if screen_matches(world, screen, context)? {
                if screen.locations_2c.is_empty() {
                    general.insert(0, form);
                } else if context.is_some() {
                    local.insert(0, form);
                }
            } else if space.is_some() && screen_matches(world, screen, space)? {
                regional.insert(0, form);
            }
        }
    }
    let mut slides = Vec::new();
    for (list, takes) in [(&mut local, 1), (&mut regional, 1)] {
        for _ in 0..takes {
            if !list.is_empty() {
                let at = random()? % list.len() as i32;
                slides.push(list.remove(at as usize));
            }
        }
    }
    while !general.is_empty() {
        let at = random()? % general.len() as i32;
        slides.push(general.remove(at as usize));
    }
    Ok(slides)
}

/// IDA: 005AD440 LoadingMenu_BuildSlideList: no DataHandler is user0 = 0; a filled +0x4C returns (5AD46B); otherwise `draw_slides`. Completion: 100% of the null context.
pub fn loading_menu_build_slide_list(
    cx: &mut InterfaceCx,
    tile: crate::tes::tile::TileId,
) -> Result<(), GameError> {
    if !cx.settings.data_handler_present {
        return tile_set_numeric_trait(cx, tile, TRAIT_USER0, 0.0);
    }
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(());
    };
    match &cx.engine.menu(menu).class {
        MenuClass::Loading(loading) if loading.slides_4c.is_empty() => {}
        _ => return Ok(()),
    }
    let host = &mut *cx.host;
    let context = match &cx.engine.menu(menu).class {
        MenuClass::Loading(loading) => loading.slide_context,
        _ => None,
    };
    let slides = draw_slides(cx.world, context, &mut || Ok(host.random_next()))?;
    if let MenuClass::Loading(loading) = &mut cx.engine.menu_mut(menu).class {
        loading.slides_4c = slides;
    }
    Ok(())
}

/// IDA: 005AE03E's 005AD440 for a 0057B950 that the load holds to a step boundary: the driver's seam has no World, but the rand stream can't wait, since 12d's rolls in the same load follow it. 00582160(0, 1) and its +0x1C, 00589B70(0x3EF) and 005898F0 are the gates. The held 005ADEC0 then finds +0x4C filled. A recorded order: the tile writes of 005ADEC0 run later, this doesn't. Completion: 100%.
pub fn loading_menu_slides_at_load_progress(
    world: &mut World,
    random: &mut dyn FnMut() -> Option<i32>,
) -> Result<(), GameError> {
    let ready = world
        .get_resource::<InterfaceManager>()
        .is_some_and(|manager| manager.cursor_01c.is_some());
    let menu = world.get_resource::<TileEngine>().and_then(|engine| {
        let menu = engine.containing_menu(engine.tile_menu_by_id(0x3EF)?)?;
        match &engine.menu(menu).class {
            MenuClass::Loading(loading) if loading.slides_4c.is_empty() => Some(menu),
            _ => None,
        }
    });
    let (true, Some(menu)) = (ready, menu) else {
        return Ok(());
    };
    let slides = draw_slides(world, None, &mut || {
        random().ok_or(GameError::unclassified(
            0x005AD5CA,
            "005AD5CA: no Random_Next on this host",
        ))
    })?;
    if let MenuClass::Loading(loading) =
        &mut world.resource_mut::<TileEngine>().menu_mut(menu).class
    {
        loading.slides_4c = slides;
    }
    Ok(())
}

impl LoadingMenu {
    /// IDA: 005AD380 LoadingMenu_SetSectionWeights(weights): each to flt_B3B3DC, its section's percentage 0, and a running float sum from 0.0 (00A2FC68); a total not 1.0 warns "Total value of [LoadingBar] percentages". Answers whether it warned. Completion: 100%.
    pub fn set_section_weights(&mut self, weights: [f32; 4]) -> bool {
        let mut total = 0.0_f32;
        for (at, &weight) in weights.iter().enumerate() {
            self.section_weights_b3b3dc[at] = weight;
            self.sections_2c[at] = 0;
            total = x87::stored_float(f64::from(weight) + f64::from(total));
        }
        total != 1.0
    }
}

/// IDA: 0057B950 LoadingMenu_SetSectionProgressIfReady(section, percent): no InterfaceManager or no +0x1C cursor returns; else 005ADEC0. Completion: 100%.
/// # Errors
/// A stop in 005ADEC0.
pub fn loading_menu_set_section_progress_if_ready(
    cx: &mut InterfaceCx,
    section: i32,
    percent: f32,
) -> Result<(), GameError> {
    if !cx
        .world
        .get_resource::<InterfaceManager>()
        .is_some_and(|manager| manager.cursor_01c.is_some())
    {
        return Ok(());
    }
    loading_menu_set_section_progress(cx, section, percent)
}

/// IDA: 005ADEC0 LoadingMenu_SetSectionProgress(section, percent)'s head and tail around the arithmetic of `set_section_progress`. Null menu or tile returns. visible 2.0, 0058FBA0(tile, 0), then 00572F60(0) below a target of 100: it fades the fader entries (00B12DD0) that 005732D0 set. On the default start nothing has set one (P15-RUN-001 and P16-RUN-001 have no 005732D0). On a start in a cell one is set before 0040EF54: P15-RUN-002, 003, 004 and 005 reach 005732D0(2, 0.3) from 0044589D, 00445D13 and 00444DEF, and the drain's 00434140 then steps its alpha here, with `fader_frame` (g_timeInfo +0x0C read as the scripts' copy, 00B33E9C). 00B14130's log lines are off in the shipped INI. Then 005AD980(0), 005AD440(+0x44) and 0057E150, which returns without +0x94 (set by MainMenu's destructor). Completion: 100% of the order.
/// TODO: [ ] 005732D0's fader on a start in a cell (tes_grid counts it).
pub fn loading_menu_set_section_progress(
    cx: &mut InterfaceCx,
    section: i32,
    percent: f32,
) -> Result<(), GameError> {
    let Some(tile) = cx.engine.tile_menu_by_id(0x3EF) else {
        return Ok(());
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(());
    };
    tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, 2.0)?;
    tile_update_scene_graph(cx, tile, false)?;
    // 005ADF10..005ADF1E: a target below 100 runs 00572F60(0) with g_timeInfo's frame seconds.
    if let MenuClass::Loading(loading) = &cx.engine.menu(menu).class
        && loading.target_3c < 100
    {
        let frame_time = cx
            .world
            .get_resource::<crate::tes::script_vm::ScriptVmGlobals>()
            .map_or(0.0, |globals| globals.frame_seconds_b33e9c);
        crate::tes::faders::fader_frame(cx.world, false, frame_time)?;
    }
    if let MenuClass::Loading(loading) = &mut cx.engine.menu_mut(menu).class {
        loading.set_section_progress(section, percent)?;
    }
    loading_menu_update_display(cx, false)?;
    loading_menu_build_slide_list(cx, tile)?;
    // 005AE056 jmp 0057E150.
    crate::tes::interface_manager::interface_rebuild_big_four_menus_if_pending(cx)
}

/// IDA: 005AD980 void LoadingMenu_UpdateDisplay(char force): 00589B70(0x3EF) and 005898F0, either null returning at once,
/// then the tile writes in order: user3, 005AD780, the +0x40 reload (5adab3 00403C00 of 00B14158). 5adac2's block, with a manager: 00432860 and 00432890 pause the model loader's clone thread, which the driver's main-thread drain doesn't have; 00583F40; 00579260(0); 005792B0. 5adb35 is nullsub_2 (0060D0A0, one `ret`), called on the main thread. Returns whether the block ran. Completion: 100%.
pub fn loading_menu_update_display(cx: &mut InterfaceCx, force: bool) -> Result<bool, GameError> {
    let Some(tile) = cx.engine.tile_menu_by_id(0x3EF) else {
        return Ok(false);
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(false);
    };
    let now = cx.host.tick_count();
    let move_bar_ms = cx.settings.loading_move_bar_ms;
    let MenuClass::Loading(loading) = &mut cx.engine.menu_mut(menu).class else {
        return Err(GameError::unclassified(
            0x005AD980,
            "005AD980: not a LoadingMenu",
        ));
    };
    let step = loading.update_display_state(force, now, move_bar_ms);
    if let Some(ribbon) = step.ribbon {
        tile_set_numeric_trait(cx, tile, TRAIT_USER3, ribbon)?;
    }
    if step.advance_slide {
        loading_menu_advance_slide(cx, menu)?;
        if let MenuClass::Loading(loading) = &mut cx.engine.menu_mut(menu).class {
            loading.slide_remaining_40 = loading.slide_delay_b14158;
        }
    }
    if step.changed
        && cx
            .world
            .contains_resource::<crate::tes::interface_manager::InterfaceManager>()
    {
        crate::tes::interface_frame::interface_update_menu_visibility(cx)?;
        crate::tes::interface_manager::interface_render_if_no_movie(cx, 0)?;
        let _ = crate::tes::interface_manager::interface_movie_gate(cx);
    }
    Ok(step.changed)
}

/// IDA: 005AD780 LoadingMenu_AdvanceSlide: it reads user0 of the menu's +0x04 tile (5ad7d4, then ftol) and walks the +0x4C
/// list to that index; on an empty list or past the end that is all, and it never tests 00B33A98. It is not 005AD440, and
/// the C++ differs by running that here. A slide: the texture's path, under 004F96F0's folder unless `strstr` finds "Menus\\Loading" (5AD853), goes in user1 (0xFAF); +0x24's vt+0x10 (004F99B0, the text at +0x34) in user2 (0xFB0), with a newline at each of its last three bytes cut (5AD8E4..5AD8F7); user0 + 1. Completion: 100%.
pub fn loading_menu_advance_slide(cx: &mut InterfaceCx, menu: MenuId) -> Result<(), GameError> {
    let Some(tile) = cx.engine.menu(menu).tile_04 else {
        return Ok(());
    };
    let index = x87::ftol(tile_numeric_trait(cx, tile, TRAIT_USER0));
    let MenuClass::Loading(loading) = &cx.engine.menu(menu).class else {
        return Ok(());
    };
    let Some(&form) = loading.slides_4c.get(index.max(0) as usize) else {
        return Ok(());
    };
    let raw = cx
        .world
        .get::<TesTexture>(form)
        .and_then(|texture| texture.path_04.data_00.clone())
        .unwrap_or_default();
    let raw = trim_terminator(raw);
    let in_folder = raw
        .windows(b"Menus\\Loading".len())
        .any(|window| window == b"Menus\\Loading");
    let path = if in_folder {
        raw
    } else {
        [&b"Textures\\Menus\\Loading\\"[..], &raw].concat()
    };
    tile_set_string_trait(cx, tile, 0xFAF, &path)?;
    let mut text = cx
        .world
        .get::<TesLoadScreen>(form)
        .and_then(|screen| screen.text_34.data_00.clone())
        .map(trim_terminator)
        .unwrap_or_default();
    for back in 1..=3 {
        if let Some(at) = text.len().checked_sub(back)
            && text[at] == b'\n'
        {
            text[at] = 0;
        }
    }
    let text = trim_terminator(text);
    tile_set_string_trait(cx, tile, 0xFB0, &text)?;
    let next = (f64::from(tile_numeric_trait(cx, tile, TRAIT_USER0)) + 1.0) as f32;
    tile_set_numeric_trait(cx, tile, TRAIT_USER0, next)
}

// IDA: N/A a text as C reads it: up to its first 0.
fn trim_terminator(mut text: Vec<u8>) -> Vec<u8> {
    if let Some(end) = text.iter().position(|&byte| byte == 0) {
        text.truncate(end);
    }
    text
}

/// IDA: N/A the class of a menu 005ADB40 has found. Completion: 100%.
fn loading_state<'a>(
    cx: &'a mut InterfaceCx,
    menu: MenuId,
) -> Result<&'a mut LoadingMenu, GameError> {
    match &mut cx.engine.menu_mut(menu).class {
        MenuClass::Loading(loading) => Ok(loading),
        _ => Err(GameError::unclassified(
            0x005ADB40,
            "005ADB40: not a LoadingMenu",
        )),
    }
}

/// IDA: 005ADB40 LoadingMenu_CompleteProgress: 00589B70(0x3EF) and 005898F0, null returning; returns when TES +0x51 or
/// +0x52 is set; on the first call (+0x71) 00404C60(g_osGlobals, 1) and +0x71 = 0; then +0x70 = 1, +0x3C = 100, 005AD980(0)
/// until +0x60 reaches 100, and +0x70 = 0. The loop spins on the tick as the original does. The C++ differs: it only sets
/// the two fields, and leaves +0x70 set. `Tes` absent: the original dereferences a null 00B333A0; the host stops.
/// Completion: 100% of the function.
pub fn loading_menu_complete_progress(cx: &mut InterfaceCx) -> Result<(), GameError> {
    if let Some(menu) = loading_menu_complete_begin(cx)? {
        while !loading_menu_complete_step(cx, menu)? {}
    }
    Ok(())
}

/// IDA: 005ADB40 up to its loop, split from `loading_menu_complete_progress` so a driver can give a frame to each step, which the original draws. `Some` when the loop has to run. Completion: 100%.
pub fn loading_menu_complete_begin(cx: &mut InterfaceCx) -> Result<Option<MenuId>, GameError> {
    let Some(tile) = cx.engine.tile_menu_by_id(0x3EF) else {
        return Ok(None);
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(None);
    };
    let (byte_51, byte_52) = cx
        .world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.tes.as_ref())
        .map(|tes| (tes.cell_test_active_51, tes.byte_52))
        .ok_or(GameError::unclassified(0x005ADB5F, "005ADB5F: no TES"))?;
    if byte_51 != 0 || byte_52 != 0 {
        return Ok(None);
    }
    if loading_state(cx, menu)?.first_completion_71 {
        purge_after_loading(cx.world)?;
        loading_state(cx, menu)?.first_completion_71 = false;
    }
    loading_state(cx, menu)?.begin_completion();
    Ok(Some(menu))
}

/// IDA: one pass of 005ADB40's loop, and +0x70 = 0 at its end. True when the bar is full. Completion: 100%.
pub fn loading_menu_complete_step(cx: &mut InterfaceCx, menu: MenuId) -> Result<bool, GameError> {
    if loading_state(cx, menu)?.displayed_60 < 100 {
        loading_menu_update_display(cx, false)?;
    }
    if loading_state(cx, menu)?.displayed_60 < 100 {
        return Ok(false);
    }
    loading_state(cx, menu)?.completing_70 = false;
    Ok(true)
}

/// IDA: 00578E30 LoadingMenu_Close: 0057CFE0(1007, 0), then the class's vt+0, 005ADBC0. Completion: 100%.
pub fn loading_menu_close(cx: &mut InterfaceCx) -> Result<(), GameError> {
    if let Some(closing) = loading_menu_close_begin(cx)? {
        if closing.filling {
            while !loading_menu_complete_step(cx, closing.menu)? {}
        }
        loading_menu_close_end(cx, closing.menu)?;
    }
    Ok(())
}

/// IDA: 00578E30 up to 005ADBC0's 005ADB40: 0057CFE0(1007, 0), which leaves status 4 with no other mode. +0x3C = 100, and with no movie running 005ADB40's head. `None` with no LoadingMenu. Completion: 100%.
pub fn loading_menu_close_begin(
    cx: &mut InterfaceCx,
) -> Result<Option<LoadingMenuClosing>, GameError> {
    let Some(tile) = cx.engine.tile_menu_by_id(1007) else {
        return Ok(None);
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(None);
    };
    if !matches!(cx.engine.menu(menu).class, MenuClass::Loading(_)) {
        return Ok(None);
    }
    interface_remove_menu_mode(cx, 1007, false);
    if let MenuClass::Loading(loading) = &mut cx.engine.menu_mut(menu).class {
        loading.target_3c = 100;
    }
    let filling = !cx.host.movie_player_is_running() && loading_menu_complete_begin(cx)?.is_some();
    Ok(Some(LoadingMenuClosing { menu, filling }))
}

/// IDA: 005ADBC0 past 005ADB40: 00583DF0(255), the +0x4C slides freed and the +0x54 timer, which the port doesn't hold. 00B3A6D3 = 0 has no reader in the exe. 006A9C00 through the host's seam, then its walk over the game sounds. The renderer's accumulator set back to the BSShaderAccumulator singleton has no host form, since the host's render has no accumulator. Then 00585330. Completion: 100% of the state the port holds.
pub fn loading_menu_close_end(cx: &mut InterfaceCx, menu: MenuId) -> Result<(), GameError> {
    interface_set_cursor_alpha(cx, 255.0);
    if let MenuClass::Loading(loading) = &mut cx.engine.menu_mut(menu).class {
        loading.slides_4c.clear();
    }
    if let Some(sound_enabled) = cx.host.resume_game_sounds_after_loading() {
        crate::tes::game_sounds::resume_game_sounds_checked(cx.world, sound_enabled)?;
    }
    menu_destroy(cx, menu)
}

/// IDA: N/A a close between `loading_menu_close_begin` and `loading_menu_close_end`: the frames 005ADB40's loop draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadingMenuClosing {
    pub menu: MenuId,
    /// `loading_menu_complete_step` still has work.
    pub filling: bool,
}

/// IDA: 00578E10 LoadingMenu_EnsureCreated(0): TileMenu_GetByMenuID(1007), else a tail jump to 005ADCF0. Completion:
/// 100%.
pub fn loading_menu_ensure_created(
    cx: &mut InterfaceCx,
) -> Result<Option<crate::tes::tile::TileId>, GameError> {
    loading_menu_ensure_created_for(cx, None, 0.0)
}

/// IDA: 00578E10 LoadingMenu_EnsureCreated(slideContext), as 00440AF0 calls it with a cell. Completion: 100%.
/// # Errors
/// A stop in 005ADCF0.
pub fn loading_menu_ensure_created_for(
    cx: &mut InterfaceCx,
    context: Option<Entity>,
    frame_time: f32,
) -> Result<Option<crate::tes::tile::TileId>, GameError> {
    if let Some(existing) = cx.engine.tile_menu_by_id(1007) {
        return Ok(Some(existing));
    }
    loading_menu_create_for(cx, context, frame_time)
}

/// IDA: 005ADCF0 LoadingMenu_Create(slideContext 0). The player, HUD and DataHandler branches take their startup arms
/// (P07-RUN-001) and are host stops otherwise. 005ADEA6: a wrong menu is destroyed (vt+0) when it has a tile; the
/// destruction is carried, as in the C++. Completion: 100% of the
/// order for the startup state.
/// TODO: [ ] The player, HUD and DataHandler arms, and destroying an existing LoadingMenu or SleepWait menu (later
/// phases).
pub fn loading_menu_create(
    cx: &mut InterfaceCx,
) -> Result<Option<crate::tes::tile::TileId>, GameError> {
    loading_menu_create_for(cx, None, 0.0)
}

/// IDA: 005ADCF0 LoadingMenu_Create(slideContext), with a slide context and the player's arms: 005A6040(1, 1), 005A4980(null, 0, 0) and 00578D50(0), each of which returns at once without a player, and with one MoviePlayer_FinishCurrentMovie when no movie runs. Completion: as `loading_menu_create`, with those arms.
/// # Errors
/// As `loading_menu_create`, or 00578D50 with the player's 3D.
pub fn loading_menu_create_for(
    cx: &mut InterfaceCx,
    context: Option<Entity>,
    frame_time: f32,
) -> Result<Option<crate::tes::tile::TileId>, GameError> {
    // 005ADCF6 an existing LoadingMenu is destroyed (none at startup).
    if cx.engine.tile_menu_by_id(1007).is_some() {
        return Err(GameError::unclassified(
            0x005ADD0A,
            "005ADD0A: destroying an existing LoadingMenu is carried",
        ));
    }
    // 005ADD0E 005A8FD0: the HUDSubtitle menu's visible = 1 (none at startup).
    if let Some(subtitle) = cx.engine.tile_menu_by_id(1010)
        && cx.engine.containing_menu(subtitle).is_some()
    {
        tile_set_numeric_trait(cx, subtitle, TRAIT_VISIBLE, 1.0)?;
    }
    let root = cx
        .world
        .get_resource::<InterfaceManager>()
        .and_then(|manager| manager.menu_root_068)
        .ok_or(GameError::unclassified(
            0x005ADD2A,
            "005ADD2A: no InterfaceManager menu root",
        ))?;
    // 005ADD21 Menu_GetNextDepth.
    let depth = menu_update_cursor_depth(cx)?;
    // 005ADD32 Tile_ReadXML(menuRoot, loading_menu.xml).
    let Some(tile) = tile_read_xml(cx, Some(root), "Data\\Menus\\loading_menu.xml")? else {
        return Ok(None);
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != 1007 {
        return Ok(None);
    }
    // 005ADD60 00583DF0(0): the cursor's alpha 0.
    interface_set_cursor_alpha(cx, 0.0);
    // 005ADD7F Menu_RegisterTile(menu, tile as TileMenu).
    let tile_menu = (cx.engine.tile(tile).kind == crate::tes::tile::TileKind::Menu).then_some(tile);
    menu_register_tile(cx, menu, tile_menu)?;
    // 005ADD9B..005ADDDE the depth trait when the stacking type is 6006 or 102.
    let stacking = tile_numeric_trait(cx, tile, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, tile, TRAIT_DEPTH, depth)?;
    }
    // 005ADDF0 +0x08 menuLoadingStatus = 2.
    manager_mut(cx).menu_loading_status_008 = 2;
    // 005ADDF4 005A6040(1, 1) and 005ADDFF 005A4980(0, 0, 0): each returns at once without g_thePlayer or the HUD's menus.
    crate::tes::hud_main_menu::hud_update_visibility(cx, true, true, frame_time)?;
    crate::tes::hud_info_menu::hud_info_clear_crosshair(cx, frame_time)?;
    // 005ADE06 00578D50(0): the player's node (vt+0x154), null returning; with one, 005A8C90.
    if let Some(player) = crate::tes::player::the_player(cx.world)
        && cx
            .world
            .get::<crate::tes::actor::TesObjectRefr>(player)
            .is_some_and(|refr| refr.loaded_3d_3c != 0)
    {
        return Err(GameError::unclassified(
            0x00578D68,
            "00578D68: 005A8C90 with the player's 3D",
        ));
    }
    // 005ADE17 +0x44 = slideContext; 005ADE21 user5 (0xFB3) = " ".
    let context_id = context.map_or(0, |form| crate::tes::forms_init::form_id(cx.world, form));
    if let MenuClass::Loading(loading) = &mut cx.engine.menu_mut(menu).class {
        loading.slide_context_44 = context_id;
        loading.slide_context = context;
    }
    tile_set_string_trait(cx, tile, TRAIT_USER5, b" ")?;
    // 005ADE2C LoadingMenu_BuildSlideList, 005ADE34 AdvanceSlide.
    loading_menu_build_slide_list(cx, tile)?;
    loading_menu_advance_slide(cx, menu)?;
    // 005ADE39 with a player and no movie running, MoviePlayer_FinishCurrentMovie.
    if crate::tes::player::the_player(cx.world).is_some()
        && let Some(movies) = cx
            .world
            .get_resource::<crate::tes::movie_player::MoviePlayer>()
            .cloned()
        && !crate::tes::movie_player::movie_player_is_running(&movies)
    {
        crate::tes::movie_player::finish_current_movie(&movies);
    }
    // 005ADE53 0058FBA0(tile->parent, 0), 005ADE5E 0058FBA0(tile, 0).
    if let Some(parent) = cx.engine.tile(tile).parent {
        tile_update_scene_graph(cx, parent, false)?;
    }
    tile_update_scene_graph(cx, tile, false)?;
    // 005ADE68 the SleepWait menu (1012) is destroyed when present (none).
    if cx.engine.tile_menu_by_id(1012).is_some() {
        return Err(GameError::unclassified(
            0x005ADE7C,
            "005ADE7C: destroying the SleepWait menu is carried",
        ));
    }
    // 005ADE80..005ADE91 the two render and gate pairs; 005ADE98 the shared stub.
    interface_render_if_no_movie(cx, 0)?;
    let _ = interface_movie_gate(cx);
    interface_render_if_no_movie(cx, 0)?;
    let _ = interface_movie_gate(cx);
    Ok(Some(tile))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamebryo::scene_update::NoSceneControllers;
    use crate::tes::interface_manager::{InterfaceSettings, interface_settings_from};
    use crate::tes::settings::{SettingCollections, register_static_settings};
    use crate::tes::tile::{Menu, NullInterfaceHost, TileId, TileKind, with_interface_cx};
    use crate::tes::tile_values::{tile_initialize, tile_string_trait};

    const WEIGHTS: [f32; 4] = [0.06, 0.35, 0.44, 0.15];
    /// 00B14160, 00B14168 and 00B14170 in the file.
    const MOVE_BAR: [i32; 3] = [2500, 100, 10];

    // The C++ loading_menu_tests progress_tests, except the StartupWork checks: that class is the host's job tracker
    // (src/app, M6).
    #[test]
    fn progress() {
        let mut now = 0u32;
        let mut menu = LoadingMenu::new(0, WEIGHTS, 15.0);
        assert!(menu.target_3c == -1 && menu.displayed_60 == 0);
        menu.set_section_progress(0, 0.0).unwrap();
        assert!(!menu.update_display_state(false, now, MOVE_BAR).changed);
        menu.set_section_progress(0, 100.0).unwrap();
        assert!(menu.target_3c == 6 && menu.displayed_60 == 0);
        now = 100;
        assert!(!menu.update_display_state(false, now, MOVE_BAR).changed);
        now = 101;
        assert!(menu.update_display_state(false, now, MOVE_BAR).changed && menu.displayed_60 == 1);
        now = 1200;
        assert!(menu.update_display_state(false, now, MOVE_BAR).changed && menu.displayed_60 == 2);
        menu.set_section_progress(3, 50.0).unwrap();
        assert!(menu.target_3c == 92 && menu.sections_2c[1] == 100 && menu.sections_2c[2] == 100);
        menu.set_section_progress(3, 5.0).unwrap();
        assert_eq!(menu.target_3c, 92);
        menu.set_section_progress(1, 20.0).unwrap();
        assert!(menu.target_3c == 41 && menu.sections_2c[2] == 0 && menu.sections_2c[3] == 0);
        assert!(menu.set_section_progress(-1, 0.0).is_err());
        assert!(menu.set_section_progress(4, 0.0).is_err());
        assert!(menu.set_section_progress(0, f32::NAN).is_err());
        assert!(menu.set_section_progress(0, 101.0).is_err());
        assert!(menu.set_section_progress(0, -1.0).is_err());
        menu.begin_completion();
        now = 1210;
        assert!(!menu.update_display_state(false, now, MOVE_BAR).changed);
        now = 1211;
        assert!(menu.update_display_state(false, now, MOVE_BAR).changed && menu.displayed_60 == 3);
        while menu.displayed_60 < 100 {
            now += 11;
            let _ = menu.update_display_state(false, now, MOVE_BAR);
        }
        now += 100_000;
        assert!(
            !menu.update_display_state(false, now, MOVE_BAR).changed && menu.displayed_60 == 100
        );
        let mut idle = LoadingMenu::new(now, WEIGHTS, 15.0);
        now += 2500;
        assert!(!idle.update_display_state(false, now, MOVE_BAR).changed);
        now += 1;
        assert!(
            idle.update_display_state(false, now, MOVE_BAR).changed
                && idle.displayed_60 == 1
                && idle.target_3c == -1
        );
        assert!(idle.update_display_state(true, now, MOVE_BAR).changed);
        now += 15000;
        assert!(idle.update_display_state(false, now, MOVE_BAR).changed && idle.displayed_60 == 2);
        let mut fractional = LoadingMenu::new(now, WEIGHTS, 15.0);
        fractional.set_section_progress(0, 99.9).unwrap();
        assert!(fractional.sections_2c[0] == 99 && fractional.target_3c == 5);
    }

    // 5ad9cf, 5ad9d4: wrapping subtractions, compared signed; a negative +0x6C difference is corrected by 2^32 at
    // 5ada18 and rounded per step at 24 bits (5ada26). The old code stopped with an error here.
    #[test]
    fn the_clock_wraps() {
        let mut menu = LoadingMenu::new(100, WEIGHTS, 15.0);
        let step = menu.update_display_state(false, 50, MOVE_BAR);
        assert!(
            step.advance_slide,
            "the corrected difference is huge, so the slide is due"
        );
        // -50 + 2^32 rounds to 2^32 in 24 bits, / 1000 rounds again, and 15 minus that again.
        let expected = 15.0f32 - (4_294_967_296.0f64 / 1000.0) as f32;
        assert_eq!(menu.slide_remaining_40.to_bits(), expected.to_bits());
        // Over the wrap: 0xFFFFFFF0 to 0x10 is 32 ms.
        let mut wrapped = LoadingMenu::new(0xFFFF_FFF0, WEIGHTS, 15.0);
        wrapped.update_display_state(false, 0x10, MOVE_BAR);
        assert_eq!(
            wrapped.slide_remaining_40.to_bits(),
            (15.0f32 - 0.032f32).to_bits()
        );
        // 32 ms is over no limit.
        assert_eq!(wrapped.pending_64, 0);
        // +0x68 ahead of the clock: elapsed is signed, so -50 is over no limit either.
        assert_eq!(menu.pending_64, 0);
    }

    // 00B14160, 00B14168, 00B14170: the limits are the settings'.
    #[test]
    fn the_limits_come_from_the_settings() {
        let mut menu = LoadingMenu::new(0, WEIGHTS, 15.0);
        assert!(!menu.update_display_state(false, 5, [5, 100, 10]).changed);
        assert!(menu.update_display_state(false, 6, [5, 100, 10]).changed);
        assert_eq!(menu.displayed_60, 1);
    }

    // 5ada26: +0x40 is a float, each tick rounds to it. 1 ms at a time from 15 keeps float steps, which an f64 does not.
    #[test]
    fn the_slide_timer_rounds_each_tick() {
        let mut menu = LoadingMenu::new(0, WEIGHTS, 15.0);
        let mut expected = 15.0f32;
        for now in 1..=1000 {
            menu.update_display_state(false, now, MOVE_BAR);
            expected -= 0.001f32;
            assert_eq!(menu.slide_remaining_40.to_bits(), expected.to_bits());
        }
        let wide = 15.0f64 - 1000.0 * 0.001;
        assert_ne!(expected.to_bits(), (wide as f32).to_bits());
    }

    // 5ae008..5ae01c: the product rounds to float before the sum. 3 sections at 100 and one at 1, with weights whose
    // product needs more than 24 bits: the target is the float chain's.
    #[test]
    #[allow(clippy::assign_op_pattern)] // 5ae018's operand order: the product first.
    fn the_section_sum_rounds_each_step() {
        let weights = [0.3f32, 0.3, 0.3, 0.1];
        let mut menu = LoadingMenu::new(0, weights, 15.0);
        menu.set_section_progress(3, 100.0).unwrap();
        let mut sum = 0.0f32;
        for (&value, &weight) in menu.sections_2c.iter().zip(&weights) {
            sum = value as f32 * weight + sum;
        }
        assert_eq!(menu.target_3c, x87::ftol(sum));
    }

    // 5ada93: at 100 the ribbon is written, and neither the slide nor the render block run. The counter goes on.
    #[test]
    fn at_a_hundred_the_slide_and_render_stop() {
        let mut menu = LoadingMenu::new(0, WEIGHTS, 15.0);
        menu.displayed_60 = 99;
        menu.pending_64 = 99;
        menu.target_3c = 100;
        let step = menu.update_display_state(true, 101, MOVE_BAR);
        assert_eq!(menu.displayed_60, 100);
        assert_eq!(step.ribbon, Some(100.0));
        assert!(!step.advance_slide && !step.changed);
        menu.update_display_state(false, 3000, MOVE_BAR);
        assert_eq!(menu.pending_64, 101, "5ad9df has no upper stop");
        assert_eq!(menu.displayed_60, 100);
    }

    fn settings() -> InterfaceSettings {
        let mut collections = SettingCollections::default();
        register_static_settings(&mut collections);
        interface_settings_from(&collections.ini, 1920, 1080, 8, Default::default())
    }

    fn loading_menu_tile(cx: &mut InterfaceCx) -> (TileId, MenuId) {
        let root = tile_initialize(cx, TileKind::Rect, None, None, None).unwrap();
        let tile = tile_initialize(cx, TileKind::Menu, Some(root), None, None).unwrap();
        let menu = MenuId(cx.engine.menus.len() as u32);
        let mut owner = Menu::new(MenuClass::Loading(LoadingMenu::new(0, WEIGHTS, 15.0)));
        owner.tile_04 = Some(tile);
        cx.engine.menus.push(owner);
        cx.engine.tile_mut(tile).menu_44 = Some(menu);
        cx.engine.tile_menus[1007 - 1001] = Some(tile);
        (tile, menu)
    }

    // 005AD780 on an empty list stores nothing and never tests 00B33A98: a set DataHandler does not stop it, and a
    // user0 of 7 stays. The old code ran 005AD440, which stops on the first and writes 0 on the second.
    #[test]
    fn advancing_an_empty_slide_list_stores_nothing() {
        let mut settings = settings();
        settings.data_handler_present = true;
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        with_interface_cx(
            &mut world,
            &settings,
            &mut host,
            &mut NoSceneControllers,
            |cx| {
                let (tile, menu) = loading_menu_tile(cx);
                tile_set_numeric_trait(cx, tile, TRAIT_USER0, 7.0).unwrap();
                assert!(loading_menu_update_display(cx, true).unwrap());
                assert_eq!(tile_numeric_trait(cx, tile, TRAIT_USER0), 7.0);
                let MenuClass::Loading(state) = &cx.engine.menu(menu).class else {
                    unreachable!()
                };
                assert_eq!(state.slide_remaining_40, 15.0, "5adabf reloads +0x40");
            },
        );
    }

    // 005ADB40: the TES gate, the first-call boundary and +0x71, the loop, and +0x70 cleared afterwards.
    #[test]
    fn completing_runs_to_a_hundred() {
        let settings = settings();
        let mut world = World::new();
        world.insert_resource(TesGlobals::default());
        let mut host = NullInterfaceHost {
            tick_step: 11,
            ..NullInterfaceHost::default()
        };
        with_interface_cx(
            &mut world,
            &settings,
            &mut host,
            &mut NoSceneControllers,
            |cx| {
                let (_, menu) = loading_menu_tile(cx);
                // No TES object yet: a host stop for the original's null dereference.
                assert!(loading_menu_complete_progress(cx).is_err());
                cx.world.resource_mut::<TesGlobals>().tes = Some(Box::default());
                cx.world
                    .resource_mut::<TesGlobals>()
                    .tes
                    .as_mut()
                    .unwrap()
                    .byte_52 = 1;
                loading_menu_complete_progress(cx).unwrap();
                let state = |cx: &InterfaceCx| {
                    let MenuClass::Loading(state) = &cx.engine.menu(menu).class else {
                        unreachable!()
                    };
                    state.clone()
                };
                assert_eq!(state(cx).displayed_60, 0, "5adb6e returns");
                assert!(state(cx).first_completion_71);
                cx.world
                    .resource_mut::<TesGlobals>()
                    .tes
                    .as_mut()
                    .unwrap()
                    .byte_52 = 0;
                loading_menu_complete_progress(cx).unwrap();
                let done = state(cx);
                assert_eq!(done.displayed_60, 100);
                assert_eq!(done.target_3c, 100);
                assert!(!done.completing_70, "5adbb1");
                assert!(!done.first_completion_71, "5adb8c");
                loading_menu_complete_progress(cx).unwrap();
            },
        );
    }

    fn load_screen(world: &mut World, path: &[u8], text: &[u8], locations: usize) -> Entity {
        let mut screen = TesLoadScreen::default();
        screen.text_34.set(Some(text), 0);
        screen.locations_2c = vec![[1, 0, 0]; locations];
        let mut texture = TesTexture::default();
        texture.path_04.set(Some(path), 0);
        world.spawn((screen, texture)).id()
    }

    // 005AD440: with a null context only the forms with no LNAM list (4F9BB0, 4F9A20) pass; each one draws `Random_Next` (5AD5CA) and moves from the prepended list to +0x4C; a filled list isn't built again (5AD46B). 005ADEC0 at 5AE03E reaches it.
    #[test]
    fn the_slide_list_draws_once_for_each_general_screen() {
        let mut settings = settings();
        settings.data_handler_present = true;
        let mut world = World::new();
        world.insert_resource(InterfaceManager::default());
        let (a, b, c);
        {
            let mut lists = DataHandlerLists::default();
            a = load_screen(&mut world, b"a.dds", b"first", 0);
            b = load_screen(&mut world, b"b.dds", b"second", 1);
            c = load_screen(&mut world, b"c.dds", b"third", 0);
            lists.load_screens_9c.prepend_non_null(a);
            lists.load_screens_9c.prepend_non_null(b);
            lists.load_screens_9c.prepend_non_null(c);
            world.insert_resource(lists);
        }
        let mut host = NullInterfaceHost::default();
        with_interface_cx(
            &mut world,
            &settings,
            &mut host,
            &mut NoSceneControllers,
            |cx| {
                let (_, menu) = loading_menu_tile(cx);
                loading_menu_set_section_progress(cx, 0, 1.0).unwrap();
                let MenuClass::Loading(state) = &cx.engine.menu(menu).class else {
                    unreachable!()
                };
                // +0x9C is c, b, a; the prepended list a, c. Random_Next answers 16383: 16383 % 2 = 1 takes c, then a.
                assert_eq!(state.slides_4c, [c, a]);
                loading_menu_set_section_progress(cx, 0, 2.0).unwrap();
                let MenuClass::Loading(state) = &cx.engine.menu(menu).class else {
                    unreachable!()
                };
                assert_eq!(state.slides_4c, [c, a], "5AD472: a list stays");
            },
        );
    }

    // 005AD780: the slide at user0: 0xFAF's path, with the folder unless it holds "Menus\\Loading"; 0xFB0's text with its last newlines gone; user0 + 1. Past the end nothing.
    #[test]
    fn a_slide_shows_its_picture_and_text() {
        let settings = settings();
        let mut world = World::new();
        let first = load_screen(&mut world, b"load_a.dds", b"First.\r\n", 0);
        let second = load_screen(&mut world, b"Menus\\Loading\\b.dds", b"Second", 0);
        let mut host = NullInterfaceHost::default();
        with_interface_cx(
            &mut world,
            &settings,
            &mut host,
            &mut NoSceneControllers,
            |cx| {
                let (tile, menu) = loading_menu_tile(cx);
                let MenuClass::Loading(state) = &mut cx.engine.menu_mut(menu).class else {
                    unreachable!()
                };
                state.slides_4c = vec![first, second];
                loading_menu_advance_slide(cx, menu).unwrap();
                assert_eq!(
                    tile_string_trait(cx, tile, 4015).unwrap(),
                    b"Textures\\Menus\\Loading\\load_a.dds"
                );
                assert_eq!(tile_string_trait(cx, tile, 4016).unwrap(), b"First.\r");
                assert_eq!(tile_numeric_trait(cx, tile, TRAIT_USER0), 1.0);
                loading_menu_advance_slide(cx, menu).unwrap();
                assert_eq!(
                    tile_string_trait(cx, tile, 4015).unwrap(),
                    b"Menus\\Loading\\b.dds"
                );
                assert_eq!(tile_numeric_trait(cx, tile, TRAIT_USER0), 2.0);
                loading_menu_advance_slide(cx, menu).unwrap();
                assert_eq!(
                    tile_numeric_trait(cx, tile, TRAIT_USER0),
                    2.0,
                    "past the end"
                );
            },
        );
    }
}
