//! StatsMenu (1003), as 0057E150's first creation builds it: 005DCB70 StatsMenu_Create over `Data\Menus\Main\stats_menu.xml`,
//! the XML's fields (005D9BE0 SetField), and the pages it fills: 005DA1A0 the attributes and skills, 005DC630 the 21 skill
//! rows from `stat_skill_template`, 005D9CB0 the level progress, 005DC950 the factions and 005DBB00 the character page.
//! The XML's `rand` traits draw on the main thread as the tiles are built (run-aa: 3 at 005904A3, 21 at 005854EB).

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::ai_conditions::the_player;
use super::form::FormIdentity;
use super::form_components::FormRef;
use super::interface_frame::menu_destroy;
use super::interface_frame::menu_enable;
use super::interface_manager::{InterfaceManager, menu_register_tile, menu_update_cursor_depth};
use super::player::PlayerCharacter;
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_USER0, tile_numeric_trait, tile_set_numeric_trait,
    tile_set_string_trait, tile_update_scene_graph,
};
use super::tile_xml::{tile_build_template, tile_read_xml, tile_xml_clear_cached_file};

/// IDA: TileTrait 0xFAA, listindex.
const TRAIT_LIST_INDEX: u16 = 4010;
/// IDA: MenuID_Stats.
pub const MENU_ID_STATS: u32 = 1003;
const STATS_XML: &str = "Data\\Menus\\Main\\stats_menu.xml";

// IDA: TileTrait userN, 4014 + N.
const fn user(n: u16) -> u16 {
    TRAIT_USER0 + n
}

/// IDA: 005D9AD0 StatsMenu_ctor (0xB4): Menu_Constructor, +0x5C 0.0, the 13 tile pointers null (+0x28 the character page,
/// +0x2C attributes, +0x30 skills, +0x34 factions, +0x38 statistics, +0x3C the skills list pane, +0x40 its scroll bar,
/// +0x44 / +0x48 the factions list and scroll bar, +0x4C / +0x50 the statistics list and scroll bar, +0x54 the focus box,
/// +0x58 the popup background), vtable A6D9DC, the 21 skill tiles +0x60.. null. Completion: 100%.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatsMenu {
    pub character_page_28: Option<TileId>,
    pub attributes_page_2c: Option<TileId>,
    pub skills_page_30: Option<TileId>,
    pub factions_page_34: Option<TileId>,
    pub statistics_page_38: Option<TileId>,
    pub skills_list_3c: Option<TileId>,
    pub skills_scroll_40: Option<TileId>,
    pub factions_list_44: Option<TileId>,
    pub factions_scroll_48: Option<TileId>,
    pub statistics_list_4c: Option<TileId>,
    pub statistics_scroll_50: Option<TileId>,
    pub focus_box_54: Option<TileId>,
    pub popup_background_58: Option<TileId>,
    pub unknown_5c: f32,
    pub skill_tiles_60: [Option<TileId>; 21],
}

impl StatsMenu {
    /// IDA: 005D9BE0 StatsMenu_SetField: ids 10, 20, 30, 40, 50 the pages, 31/32 the skills list and scroll bar, 41/42
    /// the factions', 51/52 the statistics', 60 the focus box, 61 the popup background. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        let slot = match id {
            10 => &mut self.character_page_28,
            20 => &mut self.attributes_page_2c,
            30 => &mut self.skills_page_30,
            40 => &mut self.factions_page_34,
            50 => &mut self.statistics_page_38,
            31 => &mut self.skills_list_3c,
            32 => &mut self.skills_scroll_40,
            41 => &mut self.factions_list_44,
            42 => &mut self.factions_scroll_48,
            51 => &mut self.statistics_list_4c,
            52 => &mut self.statistics_scroll_50,
            60 => &mut self.focus_box_54,
            61 => &mut self.popup_background_58,
            _ => return,
        };
        *slot = Some(tile);
    }

    /// IDA: 005D9B80 StatsMenu_HasRequiredTiles: all 13 nonnull. Completion: 100%.
    #[must_use]
    pub const fn has_required_tiles(&self) -> bool {
        self.character_page_28.is_some()
            && self.attributes_page_2c.is_some()
            && self.skills_page_30.is_some()
            && self.factions_page_34.is_some()
            && self.statistics_page_38.is_some()
            && self.skills_list_3c.is_some()
            && self.skills_scroll_40.is_some()
            && self.factions_list_44.is_some()
            && self.factions_scroll_48.is_some()
            && self.statistics_list_4c.is_some()
            && self.statistics_scroll_50.is_some()
            && self.focus_box_54.is_some()
            && self.popup_background_58.is_some()
    }
}

// IDA: N/A the menu's StatsMenu part.
fn stats<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a StatsMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::Stats(stats) => stats,
        _ => unreachable!("a StatsMenu"),
    }
}

// IDA: N/A the menu's StatsMenu part, mutable.
fn stats_mut<'a>(cx: &'a mut InterfaceCx, menu: MenuId) -> &'a mut StatsMenu {
    match &mut cx.engine.menu_mut(menu).class {
        MenuClass::Stats(stats) => stats,
        _ => unreachable!("a StatsMenu"),
    }
}

/// IDA: 005DCB70 StatsMenu_Create (0057A440's tail with a cursor and +0x60): an old 1003 menu deleted; the depth (00584980);
/// Tile_ReadXML(stats_menu.xml) under the menu root; its containing menu; 00584670 the cache; a menu that isn't 1003 is
/// deleted when it has a tile, and null answers; 00584880 registers the tile; with the 13 tiles, a stacking type 6006 or 102
/// takes the depth, InterfaceManager +0x0A (the tab) other than -1 goes to user0, then 005DA1A0(-1), 005DC630, 005D9CB0,
/// 005DC950, 005DBB00, each scroll bar's user5 -99999 then 0, Menu_Enable(1) and vt+0x2C (005DA180: input-gated, nothing
/// here); without them the warning and null. Completion: 100%.
/// # Errors
/// A stop in the XML or the pages.
pub fn stats_menu_create(cx: &mut InterfaceCx) -> Result<Option<TileId>, StreamError> {
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_STATS)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, STATS_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    tile_xml_clear_cached_file(cx, STATS_XML, menu);
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != MENU_ID_STATS {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if !stats(cx, menu).has_required_tiles() {
        cx.host
            .warning("Stats Menu Creation Failed... Are your menu and art resources up to date?");
        return Ok(None);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let tab = cx.world.resource::<InterfaceManager>().tab_ids_00a[0];
    if tab != 0xFF {
        tile_set_numeric_trait(cx, created, user(0), f32::from(tab as i8))?;
    }
    refresh_attributes_and_skills(cx, menu, None)?;
    create_skill_tiles(cx, menu)?;
    refresh_level_progress(cx, menu)?;
    rebuild_faction_page(cx, menu)?;
    refresh_character_page(cx, menu)?;
    let state = stats(cx, menu).clone();
    for scroll in [
        state.skills_scroll_40,
        state.factions_scroll_48,
        state.statistics_scroll_50,
    ]
    .into_iter()
    .flatten()
    {
        tile_set_numeric_trait(cx, scroll, user(5), -99999.0)?;
        tile_set_numeric_trait(cx, scroll, user(5), 0.0)?;
    }
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, true, frame_time)?;
    Ok(Some(created))
}

// IDA: g_thePlayer, which 0057A440's caller has checked.
fn player(cx: &InterfaceCx) -> Result<Entity, StreamError> {
    the_player(cx.world).ok_or(StreamError("005DCB70: a null g_thePlayer"))
}

// IDA: vt+0x284 PlayerCharacter_GetActorValue, `fild` to a float.
fn actor_value(cx: &InterfaceCx, code: u32) -> Result<f32, StreamError> {
    let player = player(cx)?;
    Ok(super::player::player_get_actor_value(cx.world, player, code)? as f32)
}

// IDA: 005F1910 Actor_FloorCalculatedActorValue, `fild` to a float.
fn calculated(cx: &InterfaceCx, code: u32) -> Result<f32, StreamError> {
    let player = player(cx)?;
    Ok(super::player::floor_calculated_actor_value(cx.world, player, code)? as f32)
}

/// IDA: 005E3320 TESObjectREFR_GetNPCClassOrNull: an NPC reference's base +0x104 class. Completion: 100%.
fn npc_class(cx: &InterfaceCx) -> Result<Option<Entity>, StreamError> {
    let player = player(cx)?;
    let Some(base) = cx
        .world
        .get::<super::actor::TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
    else {
        return Ok(None);
    };
    Ok(cx
        .world
        .get::<super::forms_actors::TesNpc>(base)
        .and_then(|npc| match npc.class_104 {
            FormRef::Resolved(class) => class,
            FormRef::Raw(_) => None,
        }))
}

/// IDA: 0051C090 TESClass_IsMajorSkillExceptCharGenPlaceholder: false for the class whose form ID is the iClassCharactergenClass
/// setting (00B37D00); otherwise whether one of +0x44's seven major skills is the code. Completion: 100%.
fn is_major_skill(cx: &InterfaceCx, class: Entity, code: u32) -> Result<bool, StreamError> {
    let placeholder = cx
        .world
        .get_resource::<super::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.raw_at(0x00B3_7D00))
        .ok_or(StreamError(
            "0051C096: iClassCharactergenClass isn't registered",
        ))?;
    if cx
        .world
        .get::<FormIdentity>(class)
        .expect("a form")
        .form_id_0c
        == placeholder
    {
        return Ok(false);
    }
    Ok(cx
        .world
        .get::<super::forms_misc::TesClass>(class)
        .is_some_and(|class| class.major_skills_44.contains(&code)))
}

// IDA: the major-skill count over 00565BE0(2, 0..21).
fn major_skill_count(cx: &InterfaceCx) -> Result<i32, StreamError> {
    let mut count = 0;
    if let Some(class) = npc_class(cx)? {
        for index in 0..21 {
            if is_major_skill(cx, class, 12 + index)? {
                count += 1;
            }
        }
    }
    Ok(count)
}

/// IDA: 0065FCA0 PlayerCharacter_GetSkillProgressFraction: for codes 12..=32, +0x130's experience over +0x7A4's required
/// (both floats, no zero guard), else 0. Completion: 100%.
fn skill_progress(cx: &InterfaceCx, code: u32) -> Result<f32, StreamError> {
    if !(12..=32).contains(&code) {
        return Ok(0.0);
    }
    let player = player(cx)?;
    let state = cx
        .world
        .get::<PlayerCharacter>(player)
        .expect("a PlayerCharacter");
    let index = (code - 12) as usize;
    Ok(state.skill_exp_130[index] / f32::from_bits(state.required_skill_exp_7a4[index]))
}

/// IDA: 005DA1A0 StatsMenu_RefreshAttributesAndSkills(code), `None` for -1 (all): with all, the skills page's user3 = the major
/// count. Health, magicka and fatigue (8, 9, 10) give the character page user6/8/10 the value, user7/9/11 the calculated base
/// plus the modifier's positive part, user12/13/14 the modifier (0065D270); the eight attributes give the attributes page
/// user1..8 the value and user9..16 the calculated base; a skill code's tile user1 = 1, user2 the progress (0065FCA0), user3
/// the value, user7 the calculated base; with all, each skill tile also takes its listindex, the majors counting from 0 and
/// the others from the major count plus one. Completion: 100%.
fn refresh_attributes_and_skills(
    cx: &mut InterfaceCx,
    menu: MenuId,
    code: Option<u32>,
) -> Result<(), StreamError> {
    let all = code.is_none();
    let state = stats(cx, menu).clone();
    let mut major_index = 0_i32;
    let mut minor_index = 0_i32;
    if all {
        let count = major_skill_count(cx)?;
        if let Some(page) = state.skills_page_30 {
            tile_set_numeric_trait(cx, page, user(3), count as f32)?;
        }
        minor_index = if count > 0 { count + 1 } else { count };
    }
    let player = player(cx)?;
    for (av, value_trait, base_trait, modifier_trait) in
        [(8, 6, 7, 12), (9, 8, 9, 13), (10, 10, 11, 14)]
    {
        if all || code == Some(av) {
            let modifier = super::hud_main_menu::player_max_modifier(cx.world, player, av);
            let value = actor_value(cx, av)?;
            let page = state.character_page_28.expect("checked");
            tile_set_numeric_trait(cx, page, user(value_trait), value)?;
            let positive = if modifier < 0.0 { 0.0 } else { modifier };
            let base = calculated(cx, av)? + positive;
            tile_set_numeric_trait(cx, page, user(base_trait), base)?;
            tile_set_numeric_trait(cx, page, user(modifier_trait), modifier)?;
        }
    }
    for av in 0..8_u32 {
        if all || code == Some(av) {
            let page = state.attributes_page_2c.expect("checked");
            let value = actor_value(cx, av)?;
            tile_set_numeric_trait(cx, page, user(1 + av as u16), value)?;
            let base = calculated(cx, av)?;
            tile_set_numeric_trait(cx, page, user(9 + av as u16), base)?;
        }
    }
    if let Some(code) = code.filter(|code| (12..=32).contains(code))
        && let Some(tile) = state.skill_tiles_60[(code - 12) as usize]
    {
        refresh_skill_tile(cx, tile, code)?;
    }
    if all {
        let class = npc_class(cx)?;
        for code in 12..=32_u32 {
            let Some(tile) = stats(cx, menu).skill_tiles_60[(code - 12) as usize] else {
                continue;
            };
            refresh_skill_tile(cx, tile, code)?;
            let major = match class {
                Some(class) => is_major_skill(cx, class, code)?,
                None => false,
            };
            let index = if major {
                major_index += 1;
                major_index - 1
            } else {
                minor_index += 1;
                minor_index - 1
            };
            tile_set_numeric_trait(cx, tile, TRAIT_LIST_INDEX, index as f32)?;
        }
    }
    Ok(())
}

// IDA: 005DA1A0's per-skill stores: user1 1.0, user2 the progress, user3 the value, user7 the calculated base.
fn refresh_skill_tile(cx: &mut InterfaceCx, tile: TileId, code: u32) -> Result<(), StreamError> {
    tile_set_numeric_trait(cx, tile, user(1), 1.0)?;
    let progress = skill_progress(cx, code)?;
    tile_set_numeric_trait(cx, tile, user(2), progress)?;
    let value = actor_value(cx, code)?;
    tile_set_numeric_trait(cx, tile, user(3), value)?;
    let base = calculated(cx, code)?;
    tile_set_numeric_trait(cx, tile, user(7), base)
}

/// IDA: 00565CC0 ActorValue_GetDisplayName for a skill: g_baseActorValueNamePointers (B12758)'s sSkillName* setting,
/// 00B3A094 + 8 per code past 12. Completion: 100% of the skill codes.
fn skill_name(cx: &InterfaceCx, code: u32) -> Vec<u8> {
    cx.world
        .get_resource::<super::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.text_at(0x00B3_A094 + 8 * (code - 12)))
        .unwrap_or_default()
}

/// IDA: 005DC630 StatsMenu_CreateSkillTiles: the skills page user3 = the major count; for each of the 21 skills a
/// `stat_skill_template` child of the skills list (00585410; none warns and stops), the old tile deleted, the DataHandler's
/// skill's icon path with "_small" before its last '.', the row's listindex (majors first), the name (BSStringT_Set), user3
/// the value, user4 the name, user5 the icon, user6 the code, user7 the calculated base, then 005DA1A0(code). Completion:
/// 100%.
fn create_skill_tiles(cx: &mut InterfaceCx, menu: MenuId) -> Result<(), StreamError> {
    let count = major_skill_count(cx)?;
    let state = stats(cx, menu).clone();
    if let Some(page) = state.skills_page_30 {
        tile_set_numeric_trait(cx, page, user(3), count as f32)?;
    }
    let mut minor_index = if count > 0 { count + 1 } else { count };
    let mut major_index = 0;
    let class = npc_class(cx)?;
    let pane = state.skills_list_3c.expect("checked");
    for ordinal in 0..21_u32 {
        let Some(tile) = tile_build_template(cx, menu, pane, b"stat_skill_template")? else {
            cx.host
                .warning("Error creating skill item in Stats menu. Template not valid.");
            return Ok(());
        };
        if let Some(old) = stats(cx, menu).skill_tiles_60[ordinal as usize] {
            super::interface_frame::tile_destroy(cx, old)?;
        }
        stats_mut(cx, menu).skill_tiles_60[ordinal as usize] = Some(tile);
        let code = 12 + ordinal;
        let icon = small_skill_icon(cx, ordinal);
        let major = match class {
            Some(class) => is_major_skill(cx, class, code)?,
            None => false,
        };
        let index = if major {
            major_index += 1;
            major_index - 1
        } else {
            minor_index += 1;
            minor_index - 1
        };
        let name = skill_name(cx, code);
        cx.engine.tile_mut(tile).name_08.clone_from(&name);
        tile_set_numeric_trait(cx, tile, TRAIT_LIST_INDEX, index as f32)?;
        let value = actor_value(cx, code)?;
        tile_set_numeric_trait(cx, tile, user(3), value)?;
        tile_set_string_trait(cx, tile, user(4), &name)?;
        tile_set_string_trait(cx, tile, user(5), &icon)?;
        tile_set_numeric_trait(cx, tile, user(6), code as f32)?;
        let base = calculated(cx, code)?;
        tile_set_numeric_trait(cx, tile, user(7), base)?;
        refresh_attributes_and_skills(cx, menu, Some(code))?;
    }
    Ok(())
}

// IDA: 005DC6E5..005DC73A: DataHandler +0xD8's skill's TESTexture path (or ""), with "_small" put before the last '.'
// (strcpy over itself 6 bytes on, then strncpy).
fn small_skill_icon(cx: &InterfaceCx, ordinal: u32) -> Vec<u8> {
    let path = cx
        .world
        .get_resource::<super::data_handler::DataHandler>()
        .and_then(|handler| {
            cx.world
                .get::<super::data_handler::TesSkill>(handler.skills_d8[ordinal as usize])
        })
        .and_then(|skill| skill.texture_20.path_04.data_00.clone())
        .unwrap_or_default();
    let end = path
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(path.len());
    let mut path = path[..end].to_vec();
    if let Some(dot) = path.iter().rposition(|&byte| byte == b'.') {
        let tail = path.split_off(dot);
        path.extend_from_slice(b"_small");
        path.extend_from_slice(&tail);
    }
    path
}

/// IDA: 005D9CB0 StatsMenu_RefreshSkillLevelProgress: the skills page user1 = 1.0 and user2 = the level progress
/// (0065FD00: +0x184 over iLevelUpSkillCount, `fild`/`fidiv` to a float) clamped to 0..1. Completion: 100%.
fn refresh_level_progress(cx: &mut InterfaceCx, menu: MenuId) -> Result<(), StreamError> {
    let player = player(cx)?;
    let advances = cx
        .world
        .get::<PlayerCharacter>(player)
        .expect("a PlayerCharacter")
        .major_skill_advances_184;
    let count =
        cx.world
            .get_resource::<super::game_settings::GameSettingCollection>()
            .and_then(|settings| settings.raw_at(0x00B3_7618))
            .ok_or(StreamError("0065FD0B: iLevelUpSkillCount isn't registered"))? as i32;
    let fraction = advances as f32 / count as f32;
    let clamped = if fraction >= 1.0 {
        1.0
    } else if fraction >= 0.0 {
        fraction
    } else {
        0.0
    };
    let page = stats(cx, menu).skills_page_30.expect("checked");
    tile_set_numeric_trait(cx, page, user(1), 1.0)?;
    tile_set_numeric_trait(cx, page, user(2), clamped)
}

/// IDA: 005DC950 StatsMenu_RebuildFactionPage: for an NPC base, the factions list's children destroyed (005893F0), then
/// each faction block whose faction has neither +0x34 bit 3 nor bit 0 and whose rank name isn't "DUMMY" gets a
/// `stat_faction_template` row (a stop: the rank name helpers aren't ported, and the prison's player has none to show);
/// then Tile_UpdateSceneGraph on the list. Completion: 100% of a player with no visible faction.
fn rebuild_faction_page(cx: &mut InterfaceCx, menu: MenuId) -> Result<(), StreamError> {
    let player = player(cx)?;
    let Some(base) = cx
        .world
        .get::<super::actor::TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
    else {
        return Ok(());
    };
    if cx.world.get::<super::forms_actors::TesNpc>(base).is_none() {
        return Ok(());
    }
    let pane = stats(cx, menu).factions_list_44.expect("checked");
    for child in cx.engine.tile(pane).children_30.clone() {
        super::interface_frame::tile_destroy(cx, child)?;
    }
    let factions = cx
        .world
        .get::<super::form_components::TesActorBaseData>(base)
        .map(|data| data.factions_18.clone())
        .unwrap_or_default();
    for entry in factions {
        let FormRef::Resolved(Some(faction)) = entry.faction_00 else {
            continue;
        };
        let flags = cx
            .world
            .get::<super::forms_misc::TesFaction>(faction)
            .map_or(0, |faction| faction.flags_34);
        if flags & 8 == 0 && flags & 1 == 0 {
            return Err(StreamError(
                "005DC9D8: a visible faction row (TESFaction_GetRankNameForSex)",
            ));
        }
    }
    tile_update_scene_graph(cx, pane, false)
}

/// IDA: 005DBB00 StatsMenu_RefreshCharacterPage: user1 the player's base full name (004DA2A0), user2 the race name
/// (005EA720), user3 the birthsign's name or "-", user4 the class name (005EA6B0), user5 the level (005E1FD0, unsigned word).
/// Completion: 100% with no birthsign; a birthsign is a stop.
fn refresh_character_page(cx: &mut InterfaceCx, menu: MenuId) -> Result<(), StreamError> {
    let player = player(cx)?;
    let page = stats(cx, menu).character_page_28.expect("checked");
    let base = cx
        .world
        .get::<super::actor::TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c);
    let name = base
        .map(|base| full_name(cx.world, base))
        .unwrap_or_default();
    tile_set_string_trait(cx, page, user(1), &name)?;
    let race = base
        .and_then(|base| cx.world.get::<super::form_components::TesRaceForm>(base))
        .and_then(|race| match race.race_04 {
            FormRef::Resolved(race) => race,
            FormRef::Raw(_) => None,
        })
        .map(|race| full_name(cx.world, race))
        .unwrap_or_default();
    tile_set_string_trait(cx, page, user(2), &race)?;
    if cx
        .world
        .get::<PlayerCharacter>(player)
        .expect("a PlayerCharacter")
        .birthsign_644
        != 0
    {
        return Err(StreamError("005DBB3C: a birthsign's name"));
    }
    tile_set_string_trait(cx, page, user(3), b"-")?;
    let class = npc_class(cx)?
        .map(|class| full_name(cx.world, class))
        .unwrap_or_default();
    tile_set_string_trait(cx, page, user(4), &class)?;
    let level = base
        .and_then(|base| {
            cx.world
                .get::<super::form_components::TesActorBaseData>(base)
        })
        .map(|data| super::forms_init::effective_level(cx.world, data))
        .transpose()?
        .unwrap_or(0);
    tile_set_numeric_trait(cx, page, user(5), f32::from(level as u16))
}

// IDA: a TESFullName's text: 0xFFFF scans, a zero length or no name answers empty.
fn full_name(world: &World, form: Entity) -> Vec<u8> {
    world
        .get::<super::form_components::TesFullName>(form)
        .and_then(|name| name.name_04.data_00.clone())
        .map(|name| {
            let end = name
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(name.len());
            name[..end].to_vec()
        })
        .unwrap_or_default()
}
