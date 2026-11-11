//! The RaceSexMenu (1036) as New Game opens it: `0057B7E0` InterfaceManager_OpenNewGameRaceMenu and `005CA010`
//! RaceSexMenu_Create, with the class's ctor `005C4E40`, its SetField `005C26D0` and the entry builders `005C4340` (a pane),
//! `005C4480` (a button), `005C4630` (a toggle), `005C93F0` (a slider, with `005C6860`'s value) and `005C4800` (a text).
//! `ENTRIES` is 005CA42F..005CBB79's 139 calls in order, read from the exe (pane parents are menu fields, names GMSTs).
//!
//! The pages are filled from the player's forms as the original does (once host deferral D23): `005C7BA0`(1) (race, eyes,
//! hair, colour, gender, age and complexion), `005C6EA0` (the first opening's defaults: the race's default hair, the colour
//! preset, `005C5F00`, `005C34D0`, and through `005C5C30`/`005C50A0` the hair's length and tint) and `005C7070`(0) (the
//! head's FaceGen age and gender round trip and its morph). They make no main-thread draw (run-aa, run-af); run-af's
//! after_move holds the player's NPC and head as they leave them. Left: the race description's text (`RaceDescriptionReads`).
//! `005C2BF0`(1.0, 0.0), the camera at the face, is ported (`race_sex_menu_update_camera`). Evidence: run-af's call log
//! (`records-run-af-batch0.txt`, from 005CA010 to 0057B8DE) and run-x's after_move.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::bs::scene_graphs::{AttachChildStatus, attach_scene_child};
use crate::gamebryo::mesh_geometry::MeshTriShapePayload;
use crate::tes::face_gen_head::{apply_part_morph, combine_coefficients};
use crate::tes::face_gen_nodes::{apply_hair_tint, make_hair_clone};
use crate::tes::facegen::{FaceGenMatrix, FaceGenSingleton};
use crate::tes::forms_actors::{
    TesNpc, TesRace, npc_own_coefficients, npc_randomize_face, npc_reset_face, npc_undo_face,
};
use crate::tes::forms_misc::{eyes_playable, hair_allowed};
use crate::tes::game_settings::{GameSettingCollection, text_at_or_empty};
use crate::tes::interface_frame::{TRAIT_XDEFAULT, menu_destroy, menu_enable};
use crate::tes::interface_manager::{
    InterfaceManager, menu_register_tile, menu_update_cursor_depth,
};
use crate::tes::player::the_player;
use crate::tes::reference_3d::player_face_node;
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use crate::tes::tile_values::{
    TRAIT_DEPTH, TRAIT_ID, TRAIT_STACKING, TRAIT_STRING, TRAIT_USER0, TRAIT_VISIBLE,
    tile_child_named, tile_numeric_trait, tile_set_numeric_trait, tile_set_string_trait,
};
use crate::tes::tile_xml::{tile_build_template, tile_read_xml};

/// IDA: MenuID_RaceSex.
pub const MENU_ID_RACE_SEX: u32 = 1036;
const RACE_SEX_XML: &str = "Data\\Menus\\CharGen\\race_sex_menu.xml";
/// IDA: TileTrait 0xFAA, listindex.
const TRAIT_LIST_INDEX: u16 = 4010;
/// IDA: TileTrait 0xFD0, childcount.
const TRAIT_CHILD_COUNT: u16 = 4048;

// IDA: TileTrait userN, 4014 + N.
pub(crate) const fn user(n: u16) -> u16 {
    TRAIT_USER0 + n
}

/// IDA: the GMSTs 005C3110 RaceSexMenu_GetPaneByName compares (`_mbscmp`), in order: sMain, sRace, sHair, sFace, sShape, sTone,
/// sGeneralFace, sBrow, sCheeks, sChin, sEyes, sForehead, sJaw, sMouth, sNose, sGeneralSkin, sBeard, sEyesTex, sEyebrows,
/// sMouthLips, sNoseTex: the panes at +0x40..+0x90.
const PANE_NAMES: [u32; 21] = [
    0x00B3_8F70,
    0x00B3_8F78,
    0x00B3_8F88,
    0x00B3_8F80,
    0x00B3_8FE0,
    0x00B3_8FE8,
    0x00B3_9000,
    0x00B3_9008,
    0x00B3_9010,
    0x00B3_9018,
    0x00B3_8F90,
    0x00B3_9020,
    0x00B3_9028,
    0x00B3_9030,
    0x00B3_9038,
    0x00B3_9040,
    0x00B3_9050,
    0x00B3_9320,
    0x00B3_9058,
    0x00B3_9060,
    0x00B3_9328,
];

/// IDA: GMST sGeneralFace (00B39000).
const GMST_GENERAL_FACE: u32 = 0x00B3_9000;
/// IDA: GMST sMain (00B38F70).
const GMST_MAIN: u32 = 0x00B3_8F70;
/// IDA: GMST sEnterName (00B39440).
const GMST_ENTER_NAME: u32 = 0x00B3_9440;
/// IDA: the iHairColor00..15 GMSTs (00B393B8, every 8 bytes) and sHairColor0..15 (00B39338).
const GMST_HAIR_COLORS: u32 = 0x00B3_93B8;
const GMST_HAIR_COLOR_NAMES: u32 = 0x00B3_9338;

/// IDA: a name or text argument: a GMST's string (its +0x00 value) or a literal in the image.
#[derive(Clone, Copy, Debug)]
pub enum Text {
    Gmst(u32),
    Literal(&'static [u8]),
}

/// IDA: one of 005CA010's builder calls: `at` its site, `parent` the menu field holding the parent tile, `store` where a
/// pane's tile goes.
#[derive(Clone, Copy, Debug)]
pub enum Entry {
    Pane {
        at: u32,
        parent: u32,
        name: Text,
        id: i32,
        a6: i32,
        store: u32,
    },
    Button {
        at: u32,
        parent: u32,
        name: Text,
        text: Text,
        show_name: bool,
        x_default: bool,
    },
    Toggle {
        at: u32,
        parent: u32,
        name: Text,
        x_default: bool,
    },
    Slider {
        at: u32,
        parent: u32,
        name: Text,
        index: i32,
        kind: i32,
        x_default: bool,
    },
    Text {
        at: u32,
        parent: u32,
        name: Text,
    },
}

/// IDA: 005CA42F..005CBB79, in order.
#[rustfmt::skip]
pub const ENTRIES: [Entry; 139] = [
    Entry::Pane { at: 0x5CA448, parent: 0x28, name: Text::Gmst(0xB38F70), id: 0, a6: 0, store: 0x40 },
    Entry::Button { at: 0x5CA491, parent: 0x40, name: Text::Gmst(0xB38F78), text: Text::Literal(b"          "), show_name: true, x_default: false },
    Entry::Button { at: 0x5CA4D5, parent: 0x40, name: Text::Gmst(0xB38F80), text: Text::Gmst(0xB38F80), show_name: false, x_default: false },
    Entry::Button { at: 0x5CA518, parent: 0x40, name: Text::Gmst(0xB38F88), text: Text::Literal(b"          "), show_name: true, x_default: false },
    Entry::Toggle { at: 0x5CA53A, parent: 0x40, name: Text::Gmst(0xB38F90), x_default: false },
    Entry::Slider { at: 0x5CA55E, parent: 0x40, name: Text::Gmst(0xB38F98), index: -1, kind: -1, x_default: false },
    Entry::Slider { at: 0x5CA582, parent: 0x40, name: Text::Gmst(0xB38FA0), index: -1, kind: -1, x_default: false },
    Entry::Pane { at: 0x5CA5A4, parent: 0x28, name: Text::Gmst(0xB38F78), id: -1, a6: -1, store: 0x44 },
    Entry::Toggle { at: 0x5CA5C9, parent: 0x44, name: Text::Gmst(0xB38FA8), x_default: false },
    Entry::Toggle { at: 0x5CA5EB, parent: 0x44, name: Text::Gmst(0xB38F78), x_default: false },
    Entry::Text { at: 0x5CA60B, parent: 0x44, name: Text::Gmst(0xB38FB0) },
    Entry::Pane { at: 0x5CA62D, parent: 0x28, name: Text::Gmst(0xB38F88), id: -1, a6: -1, store: 0x48 },
    Entry::Toggle { at: 0x5CA652, parent: 0x48, name: Text::Gmst(0xB38FB8), x_default: true },
    Entry::Toggle { at: 0x5CA674, parent: 0x48, name: Text::Gmst(0xB39330), x_default: false },
    Entry::Slider { at: 0x5CA698, parent: 0x48, name: Text::Gmst(0xB38FC0), index: -1, kind: -1, x_default: false },
    Entry::Slider { at: 0x5CA6BC, parent: 0x48, name: Text::Gmst(0xB38FC8), index: -1, kind: -1, x_default: false },
    Entry::Slider { at: 0x5CA6E0, parent: 0x48, name: Text::Gmst(0xB38FD0), index: -1, kind: -1, x_default: false },
    Entry::Slider { at: 0x5CA704, parent: 0x48, name: Text::Gmst(0xB38FD8), index: -1, kind: -1, x_default: false },
    Entry::Pane { at: 0x5CA726, parent: 0x28, name: Text::Gmst(0xB38F80), id: -1, a6: -1, store: 0x4C },
    Entry::Button { at: 0x5CA76D, parent: 0x4C, name: Text::Gmst(0xB38FE0), text: Text::Gmst(0xB38FE0), show_name: false, x_default: true },
    Entry::Button { at: 0x5CA7B1, parent: 0x4C, name: Text::Gmst(0xB38FE8), text: Text::Gmst(0xB38FE8), show_name: false, x_default: false },
    Entry::Button { at: 0x5CA7F5, parent: 0x4C, name: Text::Gmst(0xB38FF0), text: Text::Gmst(0xB38FF0), show_name: false, x_default: false },
    Entry::Button { at: 0x5CA839, parent: 0x4C, name: Text::Gmst(0xB38FF8), text: Text::Gmst(0xB38FF8), show_name: false, x_default: false },
    Entry::Pane { at: 0x5CA85B, parent: 0x28, name: Text::Gmst(0xB38FE0), id: -1, a6: -1, store: 0x50 },
    Entry::Button { at: 0x5CA8A2, parent: 0x50, name: Text::Gmst(0xB39000), text: Text::Gmst(0xB38F80), show_name: false, x_default: true },
    Entry::Button { at: 0x5CA8E6, parent: 0x50, name: Text::Gmst(0xB39008), text: Text::Gmst(0xB39008), show_name: false, x_default: false },
    Entry::Button { at: 0x5CA92A, parent: 0x50, name: Text::Gmst(0xB39010), text: Text::Gmst(0xB39010), show_name: false, x_default: false },
    Entry::Button { at: 0x5CA96E, parent: 0x50, name: Text::Gmst(0xB39018), text: Text::Gmst(0xB39018), show_name: false, x_default: false },
    Entry::Button { at: 0x5CA9B2, parent: 0x50, name: Text::Gmst(0xB38F90), text: Text::Gmst(0xB38F90), show_name: false, x_default: false },
    Entry::Button { at: 0x5CA9F6, parent: 0x50, name: Text::Gmst(0xB39020), text: Text::Gmst(0xB39020), show_name: false, x_default: false },
    Entry::Button { at: 0x5CAA3A, parent: 0x50, name: Text::Gmst(0xB39028), text: Text::Gmst(0xB39028), show_name: false, x_default: false },
    Entry::Button { at: 0x5CAA7E, parent: 0x50, name: Text::Gmst(0xB39030), text: Text::Gmst(0xB39030), show_name: false, x_default: false },
    Entry::Button { at: 0x5CAAC2, parent: 0x50, name: Text::Gmst(0xB39038), text: Text::Gmst(0xB39038), show_name: false, x_default: false },
    Entry::Pane { at: 0x5CAAE4, parent: 0x28, name: Text::Gmst(0xB38FE8), id: -1, a6: -1, store: 0x54 },
    Entry::Button { at: 0x5CAB2B, parent: 0x54, name: Text::Gmst(0xB39040), text: Text::Gmst(0xB39048), show_name: false, x_default: true },
    Entry::Button { at: 0x5CAB6F, parent: 0x54, name: Text::Gmst(0xB39050), text: Text::Gmst(0xB39050), show_name: false, x_default: false },
    Entry::Button { at: 0x5CABB3, parent: 0x54, name: Text::Gmst(0xB39320), text: Text::Gmst(0xB38F90), show_name: false, x_default: false },
    Entry::Button { at: 0x5CABF7, parent: 0x54, name: Text::Gmst(0xB39058), text: Text::Gmst(0xB39058), show_name: false, x_default: false },
    Entry::Button { at: 0x5CAC3B, parent: 0x54, name: Text::Gmst(0xB39060), text: Text::Gmst(0xB39068), show_name: false, x_default: false },
    Entry::Button { at: 0x5CAC7F, parent: 0x54, name: Text::Gmst(0xB39328), text: Text::Gmst(0xB39038), show_name: false, x_default: false },
    Entry::Pane { at: 0x5CACA1, parent: 0x28, name: Text::Gmst(0xB39000), id: -1, a6: -1, store: 0x58 },
    Entry::Slider { at: 0x5CACCA, parent: 0x58, name: Text::Gmst(0xB39070), index: 19, kind: 0, x_default: true },
    Entry::Slider { at: 0x5CACF0, parent: 0x58, name: Text::Gmst(0xB39078), index: 20, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CAD16, parent: 0x58, name: Text::Gmst(0xB39080), index: 21, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CAD3C, parent: 0x58, name: Text::Gmst(0xB39088), index: 22, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CAD62, parent: 0x58, name: Text::Gmst(0xB39090), index: 23, kind: 0, x_default: false },
    Entry::Pane { at: 0x5CAD84, parent: 0x28, name: Text::Gmst(0xB39008), id: -1, a6: -1, store: 0x5C },
    Entry::Slider { at: 0x5CADAD, parent: 0x5C, name: Text::Gmst(0xB39098), index: 0, kind: 0, x_default: true },
    Entry::Slider { at: 0x5CADD3, parent: 0x5C, name: Text::Gmst(0xB390A0), index: 1, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CADF9, parent: 0x5C, name: Text::Gmst(0xB390A8), index: 2, kind: 0, x_default: false },
    Entry::Pane { at: 0x5CAE1B, parent: 0x28, name: Text::Gmst(0xB39010), id: -1, a6: -1, store: 0x60 },
    Entry::Slider { at: 0x5CAE44, parent: 0x60, name: Text::Gmst(0xB390B0), index: 3, kind: 0, x_default: true },
    Entry::Slider { at: 0x5CAE6A, parent: 0x60, name: Text::Gmst(0xB390B8), index: 4, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CAE90, parent: 0x60, name: Text::Gmst(0xB390C0), index: 5, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CAEB6, parent: 0x60, name: Text::Gmst(0xB390C8), index: 6, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CAEDC, parent: 0x60, name: Text::Gmst(0xB390D0), index: 7, kind: 0, x_default: false },
    Entry::Pane { at: 0x5CAEFE, parent: 0x28, name: Text::Gmst(0xB39018), id: -1, a6: -1, store: 0x64 },
    Entry::Slider { at: 0x5CAF27, parent: 0x64, name: Text::Gmst(0xB390D8), index: 8, kind: 0, x_default: true },
    Entry::Slider { at: 0x5CAF4D, parent: 0x64, name: Text::Gmst(0xB390E0), index: 9, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CAF73, parent: 0x64, name: Text::Gmst(0xB390E8), index: 10, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CAF99, parent: 0x64, name: Text::Gmst(0xB390F0), index: 11, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CAFBF, parent: 0x64, name: Text::Gmst(0xB390F8), index: 12, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CAFE5, parent: 0x64, name: Text::Gmst(0xB39100), index: 13, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB00B, parent: 0x64, name: Text::Gmst(0xB39108), index: 14, kind: 0, x_default: false },
    Entry::Pane { at: 0x5CB02D, parent: 0x28, name: Text::Gmst(0xB38F90), id: -1, a6: -1, store: 0x68 },
    Entry::Slider { at: 0x5CB056, parent: 0x68, name: Text::Gmst(0xB39110), index: 15, kind: 0, x_default: true },
    Entry::Slider { at: 0x5CB07C, parent: 0x68, name: Text::Gmst(0xB39118), index: 16, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB0A2, parent: 0x68, name: Text::Gmst(0xB39120), index: 17, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB0C8, parent: 0x68, name: Text::Gmst(0xB39128), index: 18, kind: 0, x_default: false },
    Entry::Pane { at: 0x5CB0EA, parent: 0x28, name: Text::Gmst(0xB39020), id: -1, a6: -1, store: 0x6C },
    Entry::Slider { at: 0x5CB113, parent: 0x6C, name: Text::Gmst(0xB39130), index: 24, kind: 0, x_default: true },
    Entry::Slider { at: 0x5CB139, parent: 0x6C, name: Text::Gmst(0xB39138), index: 25, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB15F, parent: 0x6C, name: Text::Gmst(0xB39140), index: 26, kind: 0, x_default: false },
    Entry::Pane { at: 0x5CB181, parent: 0x28, name: Text::Gmst(0xB39028), id: -1, a6: -1, store: 0x70 },
    Entry::Slider { at: 0x5CB1AA, parent: 0x70, name: Text::Gmst(0xB39148), index: 27, kind: 0, x_default: true },
    Entry::Slider { at: 0x5CB1D0, parent: 0x70, name: Text::Gmst(0xB39150), index: 28, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB1F6, parent: 0x70, name: Text::Gmst(0xB39158), index: 29, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB21C, parent: 0x70, name: Text::Gmst(0xB39160), index: 30, kind: 0, x_default: false },
    Entry::Pane { at: 0x5CB23E, parent: 0x28, name: Text::Gmst(0xB39030), id: -1, a6: -1, store: 0x74 },
    Entry::Slider { at: 0x5CB267, parent: 0x74, name: Text::Gmst(0xB39168), index: 31, kind: 0, x_default: true },
    Entry::Slider { at: 0x5CB28D, parent: 0x74, name: Text::Gmst(0xB39170), index: 32, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB2B3, parent: 0x74, name: Text::Gmst(0xB39178), index: 33, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB2D9, parent: 0x74, name: Text::Gmst(0xB39180), index: 34, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB2FF, parent: 0x74, name: Text::Gmst(0xB39188), index: 35, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB325, parent: 0x74, name: Text::Gmst(0xB39190), index: 36, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB34B, parent: 0x74, name: Text::Gmst(0xB39198), index: 37, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB371, parent: 0x74, name: Text::Gmst(0xB391A0), index: 38, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB397, parent: 0x74, name: Text::Gmst(0xB391A8), index: 39, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB3BD, parent: 0x74, name: Text::Gmst(0xB391B0), index: 40, kind: 0, x_default: false },
    Entry::Pane { at: 0x5CB3DF, parent: 0x28, name: Text::Gmst(0xB39038), id: -1, a6: -1, store: 0x78 },
    Entry::Slider { at: 0x5CB408, parent: 0x78, name: Text::Gmst(0xB391B8), index: 41, kind: 0, x_default: true },
    Entry::Slider { at: 0x5CB42E, parent: 0x78, name: Text::Gmst(0xB391C0), index: 42, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB454, parent: 0x78, name: Text::Gmst(0xB391C8), index: 43, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB47A, parent: 0x78, name: Text::Gmst(0xB391D0), index: 44, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB4A0, parent: 0x78, name: Text::Gmst(0xB391D8), index: 45, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB4C6, parent: 0x78, name: Text::Gmst(0xB391E0), index: 46, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB4EC, parent: 0x78, name: Text::Gmst(0xB391E8), index: 47, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB512, parent: 0x78, name: Text::Gmst(0xB391F0), index: 48, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB538, parent: 0x78, name: Text::Gmst(0xB391F8), index: 49, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB55E, parent: 0x78, name: Text::Gmst(0xB39200), index: 50, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB584, parent: 0x78, name: Text::Gmst(0xB39208), index: 51, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB5AA, parent: 0x78, name: Text::Gmst(0xB39210), index: 52, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB5D0, parent: 0x78, name: Text::Gmst(0xB39218), index: 53, kind: 0, x_default: false },
    Entry::Slider { at: 0x5CB5F6, parent: 0x78, name: Text::Gmst(0xB39220), index: 54, kind: 0, x_default: false },
    Entry::Pane { at: 0x5CB618, parent: 0x28, name: Text::Gmst(0xB39040), id: -1, a6: -1, store: 0x7C },
    Entry::Slider { at: 0x5CB641, parent: 0x7C, name: Text::Gmst(0xB39228), index: 28, kind: 1, x_default: true },
    Entry::Slider { at: 0x5CB667, parent: 0x7C, name: Text::Gmst(0xB39230), index: 29, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CB68D, parent: 0x7C, name: Text::Gmst(0xB39238), index: 30, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CB6B3, parent: 0x7C, name: Text::Gmst(0xB39240), index: 31, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CB6D9, parent: 0x7C, name: Text::Gmst(0xB39248), index: 6, kind: 1, x_default: false },
    Entry::Pane { at: 0x5CB6FB, parent: 0x28, name: Text::Gmst(0xB39050), id: -1, a6: -1, store: 0x80 },
    Entry::Slider { at: 0x5CB72A, parent: 0x80, name: Text::Gmst(0xB39250), index: 0, kind: 1, x_default: true },
    Entry::Slider { at: 0x5CB753, parent: 0x80, name: Text::Gmst(0xB39258), index: 1, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CB77C, parent: 0x80, name: Text::Gmst(0xB39260), index: 2, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CB7A5, parent: 0x80, name: Text::Gmst(0xB39268), index: 3, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CB7CE, parent: 0x80, name: Text::Gmst(0xB39270), index: 4, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CB7F7, parent: 0x80, name: Text::Gmst(0xB39278), index: 5, kind: 1, x_default: false },
    Entry::Pane { at: 0x5CB819, parent: 0x28, name: Text::Gmst(0xB39320), id: -1, a6: -1, store: 0x84 },
    Entry::Slider { at: 0x5CB848, parent: 0x84, name: Text::Gmst(0xB39280), index: 7, kind: 1, x_default: true },
    Entry::Slider { at: 0x5CB871, parent: 0x84, name: Text::Gmst(0xB39288), index: 8, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CB89A, parent: 0x84, name: Text::Gmst(0xB39290), index: 16, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CB8C3, parent: 0x84, name: Text::Gmst(0xB39298), index: 17, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CB8EC, parent: 0x84, name: Text::Gmst(0xB392A0), index: 18, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CB915, parent: 0x84, name: Text::Gmst(0xB392A8), index: 19, kind: 1, x_default: false },
    Entry::Pane { at: 0x5CB937, parent: 0x28, name: Text::Gmst(0xB39058), id: -1, a6: -1, store: 0x88 },
    Entry::Slider { at: 0x5CB966, parent: 0x88, name: Text::Gmst(0xB392C0), index: 9, kind: 1, x_default: true },
    Entry::Slider { at: 0x5CB98F, parent: 0x88, name: Text::Gmst(0xB392C8), index: 10, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CB9B8, parent: 0x88, name: Text::Gmst(0xB392D0), index: 11, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CB9E1, parent: 0x88, name: Text::Gmst(0xB392D8), index: 12, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CBA0A, parent: 0x88, name: Text::Gmst(0xB392E0), index: 13, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CBA33, parent: 0x88, name: Text::Gmst(0xB392E8), index: 14, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CBA5C, parent: 0x88, name: Text::Gmst(0xB392F0), index: 15, kind: 1, x_default: false },
    Entry::Pane { at: 0x5CBA7E, parent: 0x28, name: Text::Gmst(0xB39060), id: -1, a6: -1, store: 0x8C },
    Entry::Slider { at: 0x5CBAAD, parent: 0x8C, name: Text::Gmst(0xB392F8), index: 22, kind: 1, x_default: true },
    Entry::Slider { at: 0x5CBAD6, parent: 0x8C, name: Text::Gmst(0xB39300), index: 23, kind: 1, x_default: false },
    Entry::Slider { at: 0x5CBAFF, parent: 0x8C, name: Text::Gmst(0xB39308), index: 24, kind: 1, x_default: false },
    Entry::Pane { at: 0x5CBB21, parent: 0x28, name: Text::Gmst(0xB39328), id: -1, a6: -1, store: 0x90 },
    Entry::Slider { at: 0x5CBB50, parent: 0x90, name: Text::Gmst(0xB39310), index: 25, kind: 1, x_default: true },
    Entry::Slider { at: 0x5CBB79, parent: 0x90, name: Text::Gmst(0xB39318), index: 26, kind: 1, x_default: false },
];

/// IDA: 005C4E40 RaceSexMenu_ctor (0x9B0): Menu_Constructor, then the fields the port keeps: +0x28..+0x3C the SetField tiles
/// (1 race_window, 2 race_player_grab, 10 race_name, 20 race_scroll_bar, 21, 90 race_button_done), +0x40..+0x90 the panes,
/// +0x94 the tiles by entry ID (0x7D0 bytes zeroed), +0x84C the item 005CA010 unequips, +0x890 the opening tick, +0x894 the
/// player's +0x588 at opening, +0x8D8 the second FgMatrix array (a pointer to 4 matrices, 00552990's output), +0x868..+0x88C
/// the pages' values (005C7BA0), +0x8F0 the 16 hair colours and +0x930 their 16 names. Completion: the fields the creation writes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RaceSexMenu {
    pub window_28: Option<TileId>,
    pub player_grab_2c: Option<TileId>,
    pub name_30: Option<TileId>,
    pub scroll_bar_34: Option<TileId>,
    pub tile_38: Option<TileId>,
    pub done_3c: Option<TileId>,
    pub panes_40: [Option<TileId>; 21],
    pub tiles_94: std::collections::BTreeMap<i32, TileId>,
    pub unequipped_84c: Option<Entity>,
    /// TODO: [ ] 005CC1B2 g_timeInfo +0x10, which the interface can't read yet.
    pub opened_ticks_890: Option<u32>,
    pub third_person_894: u8,
    /// +0x8D0 the frame's pause byte (`005CE570` skips the player's animation while it's set; `005CC200` clears it).
    pub pause_8d0: u8,
    /// +0x8EC the name field's MenuTextInputState (the ctor's 0057FE70; `005C2730` enables it, a click ends it).
    pub name_input_8ec: super::text_edit_menu::MenuTextInputState,
    /// +0x898 and +0x89C the drag origins `005C2790` stores at a press (x for the sliders and the camera turn, y for the scroll
    /// bar); +0x8A0 the camera's turn, the cursor's x less +0x898, which `005C3BE0` sets while id 2 is held and `005CE570`
    /// passes over 5.0 as `005C2BF0`'s angle. The ctor's 0.0.
    pub drag_origin_x_898: f32,
    pub drag_origin_y_89c: f32,
    pub camera_turn_8a0: f32,
    /// +0x8A4 the camera's position at the face (`005C2BF0`).
    pub camera_position_8a4: [f32; 3],
    pub coefficients_8d8: [FaceGenMatrix; 4],
    /// +0x868 the NPC is female (005C7BA0).
    pub female_868: u8,
    /// +0x86C the race's index among the playable races, +0x870 the hair's among the race's playable hairs for the sex,
    /// +0x87C the eyes' among the race's playable eyes (005C7BA0).
    pub race_index_86c: i32,
    pub hair_index_870: i32,
    /// +0x874 the NPC's hair length (005C7BA0).
    pub hair_length_874: f32,
    /// +0x878 the hair colour's four bytes, red first (005C7BA0 copies the NPC's +0x1E8; 005C5F00 takes a preset's).
    pub hair_color_878: u32,
    pub eyes_index_87c: i32,
    /// +0x880 the Age slider's value, +0x884 the Complexion's (005C7BA0, from the age and gender controls).
    pub age_880: f32,
    pub complexion_884: f32,
    /// +0x88C the Hair Color toggle's user0 at 005C7BA0, `ftol`'d.
    pub hair_color_index_88c: i32,
    /// +0x8F0 the 16 iHairColorNN values and +0x930 the 16 "%s" copies of sHairColorN (005CA1DE..).
    pub hair_colors_8f0: [u32; 16],
    pub hair_color_names_930: [Vec<u8>; 16],
}

impl RaceSexMenu {
    /// IDA: 005C26D0 RaceSexMenu_SetField(id, tile): 1 +0x28, 2 +0x2C, 10 +0x30, 20 +0x34, 21 +0x38, 90 +0x3C. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        match id {
            1 => self.window_28 = Some(tile),
            2 => self.player_grab_2c = Some(tile),
            10 => self.name_30 = Some(tile),
            20 => self.scroll_bar_34 = Some(tile),
            21 => self.tile_38 = Some(tile),
            90 => self.done_3c = Some(tile),
            _ => {}
        }
    }

    // IDA: N/A a menu field by its offset.
    fn field(&self, offset: u32) -> Option<TileId> {
        match offset {
            0x28 => self.window_28,
            0x2C => self.player_grab_2c,
            0x30 => self.name_30,
            0x34 => self.scroll_bar_34,
            0x38 => self.tile_38,
            0x3C => self.done_3c,
            0x40..=0x90 => self.panes_40[((offset - 0x40) / 4) as usize],
            _ => None,
        }
    }
}

/// IDA: the statics the menu uses: 00B14500 g_raceSexMenuInitialSetupPending (image 1; 0057C12E sets it, 005CBCA4 clears
/// it), 00B3B444 + 0x8C the remembered name buffer, + 0x190 the entry IDs' counter (the ctor stores 200), + 0x194 the camera
/// byte 005C2BF0 tests (the ctor clears it), + 0x84 the word the ctor clears; and 00B3BB04, which 0065D660 clears.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct RaceSexMenuStatics {
    pub initial_setup_pending_b14500: u8,
    pub name_b3b4d0: Vec<u8>,
    pub next_id_b3b5d4: i32,
    pub camera_byte_b3b5d8: u8,
    pub word_b3b4c8: u16,
    pub vanity_b3bb04: u8,
}

impl Default for RaceSexMenuStatics {
    /// IDA: N/A the image's values.
    fn default() -> Self {
        Self {
            initial_setup_pending_b14500: 1,
            name_b3b4d0: Vec::new(),
            next_id_b3b5d4: 0,
            camera_byte_b3b5d8: 0,
            word_b3b4c8: 0,
            vanity_b3bb04: 0,
        }
    }
}

/// IDA: N/A the counted calls of the menu's creation: the ctor's two ModelLoader_LoadModelNow calls
/// (`Characters\_Male\Skeleton.nif`, `SkeletonBeast.nif`).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct RaceSexMenuCountedCalls {
    pub calls: std::collections::BTreeMap<u32, u32>,
}

/// IDA: 005C4E40 RaceSexMenu_ctor's statics: 00B3B444 + 0x190 = 200, + 0x84 (a word) = 0, + 0x194 = 0, and the two
/// ModelLoader_LoadModelNow(..., 0, 0, 1) of the male skeletons, counted (the interface lends no archives). Completion: 100%
/// of the state; the loads are a counted boundary.
pub fn race_sex_menu_constructed(world: &mut World) -> RaceSexMenu {
    let mut statics = world.get_resource_or_init::<RaceSexMenuStatics>();
    statics.next_id_b3b5d4 = 200;
    statics.word_b3b4c8 = 0;
    statics.camera_byte_b3b5d8 = 0;
    let mut deferrals = world.get_resource_or_init::<RaceSexMenuCountedCalls>();
    *deferrals.calls.entry(0x005C_4FEA).or_default() += 1;
    *deferrals.calls.entry(0x005C_4FFC).or_default() += 1;
    RaceSexMenu::default()
}

// IDA: N/A the menu's class.
pub(crate) fn race_sex<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a RaceSexMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::RaceSex(race_sex) => race_sex,
        _ => unreachable!("a RaceSexMenu"),
    }
}

// IDA: N/A the menu's class.
pub(crate) fn race_sex_mut<'a>(cx: &'a mut InterfaceCx, menu: MenuId) -> &'a mut RaceSexMenu {
    match &mut cx.engine.menu_mut(menu).class {
        MenuClass::RaceSex(race_sex) => race_sex,
        _ => unreachable!("a RaceSexMenu"),
    }
}

// IDA: N/A a `Text` argument's bytes.
fn text(cx: &InterfaceCx, value: Text) -> Vec<u8> {
    match value {
        Text::Gmst(address) => text_at_or_empty(cx.world, address),
        Text::Literal(bytes) => bytes.to_vec(),
    }
}

/// IDA: 005C3110 RaceSexMenu_GetPaneByName(name): `_mbscmp` against the 21 pane GMSTs in order, the matching pane (+0x40 +
/// 4 * i); none answers null. Completion: 100%.
fn pane_by_name(cx: &InterfaceCx, menu: MenuId, name: &[u8]) -> Option<TileId> {
    PANE_NAMES
        .iter()
        .position(|&address| text_at_or_empty(cx.world, address) == name)
        .and_then(|index| race_sex(cx, menu).panes_40[index])
}

/// IDA: 005C3440 RaceSexMenu_GetPaneChild(pane, child): 005C3110's pane, then 00589930 its child by name. A null pane
/// faults in the original. Completion: 100%.
/// # Errors
/// A null pane.
pub fn pane_child(
    cx: &InterfaceCx,
    menu: MenuId,
    pane: &[u8],
    child: &[u8],
) -> Result<Option<TileId>, GameError> {
    let pane = pane_by_name(cx, menu, pane).ok_or(GameError::unclassified(
        0x005C3466,
        "005C3466: a null pane faults",
    ))?;
    Ok(tile_child_named(cx, pane, child))
}

// IDA: N/A the template child the builders make, with the "%s" copy of the name as its name.
fn build(
    cx: &mut InterfaceCx,
    menu: MenuId,
    parent: TileId,
    template: &[u8],
    name: &[u8],
) -> Result<TileId, GameError> {
    let tile = tile_build_template(cx, menu, parent, template)?.ok_or(GameError::unclassified(
        0x005C4363,
        "005C4363: no template, and the original faults",
    ))?;
    cx.engine.tile_mut(tile).name_08 = name.to_vec();
    Ok(tile)
}

// IDA: N/A the builders' id and listindex: id the entry's ID, listindex the parent's childcount less 1.0.
fn number(cx: &mut InterfaceCx, tile: TileId, parent: TileId, id: i32) -> Result<(), GameError> {
    tile_set_numeric_trait(cx, tile, TRAIT_ID, id as f32)?;
    let index = tile_numeric_trait(cx, parent, TRAIT_CHILD_COUNT) - 1.0;
    tile_set_numeric_trait(cx, tile, TRAIT_LIST_INDEX, index)
}

// IDA: N/A 00B3B444 + 0x190, the entry IDs' counter.
fn next_id(cx: &mut InterfaceCx) -> i32 {
    let mut statics = cx.world.resource_mut::<RaceSexMenuStatics>();
    statics.next_id_b3b5d4 += 1;
    statics.next_id_b3b5d4
}

/// IDA: (float)0x80000000, which the builders store to user3 before its value.
const USER3_RESET: f32 = -2_147_483_648.0;

/// IDA: 005C6860 RaceSexMenu_GetSliderValue(id): the player's coefficients 00552990(race +0x29C, 00521A10's NPC +0x168 or
/// +0x108); the clamp from the race: +0xA4 (else 3.0) when the menu tile's user0 equals sGeneralFace's pane's id, else +0xA0
/// (else 5.0). The slider (+0x94 + 4 * id): its user7 less 1 the control and user8 the kind; 005538F0(coefficients, kind,
/// 0, control) is linear control set `kind * 2`'s entry times the set's coefficient matrix (FgMatrix_Multiply, each sum
/// stored as a float). A texture slider (user8 set) answers (v + 10) / 20, any other (v + clamp) / (clamp + clamp); then
/// 1.0 * that + 0.0. Completion: 100%, f32 for the main thread's 24-bit x87.
/// # Errors
/// No g_faceGen, or a coefficients shape mismatch.
fn slider_value(cx: &mut InterfaceCx, menu: MenuId, id: i32) -> Result<f32, GameError> {
    let player = the_player(cx.world).ok_or(GameError::unclassified(
        0x005C6884,
        "005C6884: a null g_thePlayer",
    ))?;
    let npc = cx
        .world
        .get::<crate::tes::actor::TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
        .ok_or(GameError::unclassified(
            0x005C6884,
            "005C6884: the player with no base form",
        ))?;
    let race = crate::tes::forms_actors::base_race(cx.world, npc)?.ok_or(
        GameError::unclassified(0x005C6897, "005C6897: a null race faults"),
    )?;
    let vampire = crate::tes::actor_values::base_actor_value(cx.world, npc, 69)? != 0;
    let (race_coefficients, main, face) = {
        let data = cx.world.get::<TesRace>(race).expect("a race");
        (
            data.facegen_29c.clone(),
            data.facegen_main_a0,
            data.facegen_face_a4,
        )
    };
    let own = {
        let data = cx.world.get::<TesNpc>(npc).expect("an NPC");
        if vampire {
            data.facegen_168.clone()
        } else {
            data.facegen_108.clone()
        }
    };
    let mut coefficients: [FaceGenMatrix; 4] = Default::default();
    combine_coefficients(&race_coefficients, &own, &mut coefficients)?;
    let general = pane_by_name(cx, menu, &text_at_or_empty(cx.world, GMST_GENERAL_FACE));
    let page = cx
        .engine
        .menu(menu)
        .tile_04
        .map_or(0.0, |tile| tile_numeric_trait(cx, tile, user(0)));
    let general_id = general.map_or(0.0, |tile| tile_numeric_trait(cx, tile, TRAIT_ID));
    let clamp = if general_id == page {
        if face <= 0.0 { 3.0 } else { face }
    } else if main <= 0.0 {
        5.0
    } else {
        main
    };
    let slider = race_sex(cx, menu)
        .tiles_94
        .get(&id)
        .copied()
        .ok_or(GameError::unclassified(
            0x005C6912,
            "005C6912: a null slider faults",
        ))?;
    let kind = tile_numeric_trait(cx, slider, user(8));
    let control = (tile_numeric_trait(cx, slider, user(7)) - 1.0) as i64 as u32;
    let set = (kind as i64 as usize) * 2;
    let value = {
        let state = cx
            .world
            .get_resource::<FaceGenSingleton>()
            .and_then(|singleton| singleton.0.as_ref())
            .ok_or(GameError::unclassified(
                0x005538FE,
                "005538FE: g_faceGen is null",
            ))?;
        let controls = state
            .linear_controls
            .get(set)
            .ok_or(GameError::unclassified(
                0x00553922,
                "00553922: a linear control set past the four",
            ))?;
        match controls.get(control as usize) {
            None => 0.0,
            Some(entry) => {
                let matrix = &coefficients[set];
                let row = &entry.coefficients;
                if row.cols != matrix.rows {
                    return Err(GameError::unclassified(
                        0x005523E0,
                        "005523E0: a control against coefficients of another size",
                    ));
                }
                let mut sum = 0.0_f32;
                for k in 0..row.cols as usize {
                    sum += matrix.data[k * matrix.cols as usize] * row.data[k];
                }
                sum
            }
        }
    };
    let fraction = if kind != 0.0 {
        (value - -10.0) / 20.0
    } else {
        let low = -clamp;
        (value - low) / (clamp - low)
    };
    Ok((1.0 - 0.0) * fraction + 0.0)
}

/// IDA: 005CA42F..005CBB79: each entry's builder. 005C4340 (a pane): `race_template_pane` under the parent (00585410), its
/// name, an id (the given one, or the counter's next with -1), listindex, user0 = id, the tile at +0x94 + 4 * id, stored at
/// `store`. 005C4480 (a button): the counter's next, `race_template_button`, name, id, listindex, user1 0.0, user2 the name
/// or "   ", user3 INT_MIN then 0.0, user6 the text, xdefault 2.0 when asked. 005C4630 (a toggle): `race_template_toggle`,
/// user1 0.0, user2 the name, user3 = -user0, 0.0, 0.0, user4 0.0, user5 0.0, user6 "          ", xdefault. 005C93F0 (a
/// slider): IDs counter + 1 (the tile's id) and counter + 2 (user6), both mapping to it, `race_template_slider`, user1 0.0,
/// user2 the name, user3 INT_MIN, 0.0, 0.0, user6, user7 = index + 1, user8 = kind; with an index, 005C6860's value into user3
/// (INT_MIN, the value, 0.0), user10 and user20 1.0; xdefault. 005C4800 (a text): `race_template_text`, id, listindex.
/// Completion: 100%.
/// # Errors
/// A missing parent or template, or a stop in the traits.
#[allow(clippy::too_many_lines)] // The five builders, as the table drives them.
fn build_entries(cx: &mut InterfaceCx, menu: MenuId) -> Result<(), GameError> {
    for entry in ENTRIES {
        let parent_of = |cx: &InterfaceCx, offset: u32| {
            race_sex(cx, menu)
                .field(offset)
                .ok_or(GameError::unclassified(
                    0x005C4363,
                    "005C4363: a null parent tile faults",
                ))
        };
        match entry {
            Entry::Pane {
                parent,
                name,
                id,
                store,
                ..
            } => {
                let parent = parent_of(cx, parent)?;
                let name = text(cx, name);
                let tile = build(cx, menu, parent, b"race_template_pane", &name)?;
                let id = if id == -1 { next_id(cx) } else { id };
                number(cx, tile, parent, id)?;
                tile_set_numeric_trait(cx, tile, user(0), id as f32)?;
                let menu_data = race_sex_mut(cx, menu);
                menu_data.tiles_94.insert(id, tile);
                menu_data.panes_40[((store - 0x40) / 4) as usize] = Some(tile);
            }
            Entry::Button {
                parent,
                name,
                text: label,
                show_name,
                x_default,
                ..
            } => {
                let id = next_id(cx);
                let parent = parent_of(cx, parent)?;
                let name = text(cx, name);
                let label = text(cx, label);
                let tile = build(cx, menu, parent, b"race_template_button", &name)?;
                number(cx, tile, parent, id)?;
                tile_set_numeric_trait(cx, tile, user(1), 0.0)?;
                let shown: &[u8] = if show_name { &name } else { b"   " };
                tile_set_string_trait(cx, tile, user(2), shown)?;
                tile_set_numeric_trait(cx, tile, user(3), USER3_RESET)?;
                tile_set_numeric_trait(cx, tile, user(3), 0.0)?;
                tile_set_string_trait(cx, tile, user(6), &label)?;
                if x_default {
                    tile_set_numeric_trait(cx, tile, TRAIT_XDEFAULT, 2.0)?;
                }
                race_sex_mut(cx, menu).tiles_94.insert(id, tile);
            }
            Entry::Toggle {
                parent,
                name,
                x_default,
                ..
            } => {
                let id = next_id(cx);
                let parent = parent_of(cx, parent)?;
                let name = text(cx, name);
                let tile = build(cx, menu, parent, b"race_template_toggle", &name)?;
                number(cx, tile, parent, id)?;
                tile_set_numeric_trait(cx, tile, user(1), 0.0)?;
                tile_set_string_trait(cx, tile, user(2), &name)?;
                let own = -tile_numeric_trait(cx, tile, user(0));
                tile_set_numeric_trait(cx, tile, user(3), own)?;
                tile_set_numeric_trait(cx, tile, user(3), 0.0)?;
                tile_set_numeric_trait(cx, tile, user(3), 0.0)?;
                tile_set_numeric_trait(cx, tile, user(4), 0.0)?;
                tile_set_numeric_trait(cx, tile, user(5), 0.0)?;
                tile_set_string_trait(cx, tile, user(6), b"          ")?;
                if x_default {
                    tile_set_numeric_trait(cx, tile, TRAIT_XDEFAULT, 2.0)?;
                }
                race_sex_mut(cx, menu).tiles_94.insert(id, tile);
            }
            Entry::Slider {
                parent,
                name,
                index,
                kind,
                x_default,
                ..
            } => {
                let (first, second) = {
                    let mut statics = cx.world.resource_mut::<RaceSexMenuStatics>();
                    let first = statics.next_id_b3b5d4 + 1;
                    let second = statics.next_id_b3b5d4 + 2;
                    statics.next_id_b3b5d4 = second;
                    (first, second)
                };
                let parent = parent_of(cx, parent)?;
                let name = text(cx, name);
                let tile = build(cx, menu, parent, b"race_template_slider", &name)?;
                number(cx, tile, parent, first)?;
                tile_set_numeric_trait(cx, tile, user(1), 0.0)?;
                tile_set_string_trait(cx, tile, user(2), &name)?;
                tile_set_numeric_trait(cx, tile, user(3), USER3_RESET)?;
                tile_set_numeric_trait(cx, tile, user(3), 0.0)?;
                tile_set_numeric_trait(cx, tile, user(3), 0.0)?;
                tile_set_numeric_trait(cx, tile, user(6), second as f32)?;
                tile_set_numeric_trait(cx, tile, user(7), (index + 1) as f32)?;
                tile_set_numeric_trait(cx, tile, user(8), kind as f32)?;
                {
                    let menu_data = race_sex_mut(cx, menu);
                    menu_data.tiles_94.insert(first, tile);
                    menu_data.tiles_94.insert(second, tile);
                }
                if index != -1 {
                    let value = slider_value(cx, menu, second)?;
                    tile_set_numeric_trait(cx, tile, user(3), USER3_RESET)?;
                    tile_set_numeric_trait(cx, tile, user(3), value)?;
                    tile_set_numeric_trait(cx, tile, user(3), 0.0)?;
                    tile_set_numeric_trait(cx, tile, user(10), value)?;
                    tile_set_numeric_trait(cx, tile, user(20), 1.0)?;
                }
                if x_default {
                    tile_set_numeric_trait(cx, tile, TRAIT_XDEFAULT, 2.0)?;
                }
            }
            Entry::Text { parent, name, .. } => {
                let id = next_id(cx);
                let parent = parent_of(cx, parent)?;
                let name = text(cx, name);
                let tile = build(cx, menu, parent, b"race_template_text", &name)?;
                number(cx, tile, parent, id)?;
                race_sex_mut(cx, menu).tiles_94.insert(id, tile);
            }
        }
    }
    Ok(())
}

/// IDA: the GMSTs the page fills name: sRace, sHair, sEyes, sAge, sComplexion, sGender, sRaceDescription, sStyle, sRed,
/// sGreen, sBlue, sLength, sColor ("Hair Color"), sBald, sCustomColor, sMale and sFemale.
const GMST_RACE: u32 = 0x00B3_8F78;
const GMST_HAIR: u32 = 0x00B3_8F88;
const GMST_EYES: u32 = 0x00B3_8F90;
const GMST_AGE: u32 = 0x00B3_8F98;
const GMST_COMPLEXION: u32 = 0x00B3_8FA0;
const GMST_GENDER: u32 = 0x00B3_8FA8;
const GMST_RACE_DESCRIPTION: u32 = 0x00B3_8FB0;
const GMST_STYLE: u32 = 0x00B3_8FB8;
const GMST_RED: u32 = 0x00B3_8FC0;
const GMST_GREEN: u32 = 0x00B3_8FC8;
const GMST_BLUE: u32 = 0x00B3_8FD0;
const GMST_LENGTH: u32 = 0x00B3_8FD8;
const GMST_HAIR_COLOR: u32 = 0x00B3_9330;
const GMST_BALD: u32 = 0x00B3_8B80;
const GMST_CUSTOM_COLOR: u32 = 0x00B3_94D8;
const GMST_MALE: u32 = 0x00B3_9520;
const GMST_FEMALE: u32 = 0x00B3_9528;

// IDA: N/A 005C3440 on two GMSTs' strings, for a tile the original then writes (a null child faults there: a stop).
fn page_child(cx: &InterfaceCx, menu: MenuId, pane: u32, child: u32) -> Result<TileId, GameError> {
    pane_child(
        cx,
        menu,
        &text_at_or_empty(cx.world, pane),
        &text_at_or_empty(cx.world, child),
    )?
    .ok_or(GameError::unclassified(
        0x005C3440,
        "005C3440: a null pane child, which the original writes through",
    ))
}

// IDA: N/A a pane child's numeric trait, read through 00588BD0 (a null child faults: a stop).
fn page_number(
    cx: &InterfaceCx,
    menu: MenuId,
    pane: u32,
    child: u32,
    trait_id: u16,
) -> Result<f32, GameError> {
    let tile = page_child(cx, menu, pane, child)?;
    Ok(tile_numeric_trait(cx, tile, trait_id))
}

/// IDA: the race menu's slider store (005C7BA0's and 005C6860's): user3 = (float)0x80000000, then the value, then 0.0
/// (0058CEB0 three times), for a host capture or test to move a slider as the game does. Completion: 100%.
/// # Errors
/// As `tile_set_numeric_trait`.
pub fn race_sex_menu_set_slider(
    cx: &mut InterfaceCx,
    tile: TileId,
    value: f32,
) -> Result<(), GameError> {
    set_user3(cx, tile, value)
}

// IDA: N/A the sliders' store: user3 = (float)0x80000000, then the value, then 0.0 (0058CEB0 three times).
fn set_user3(cx: &mut InterfaceCx, tile: TileId, value: f32) -> Result<(), GameError> {
    tile_set_numeric_trait(cx, tile, user(3), USER3_RESET)?;
    tile_set_numeric_trait(cx, tile, user(3), value)?;
    tile_set_numeric_trait(cx, tile, user(3), 0.0)
}

/// IDA: 005C2B50 RaceSexToggle_SetValue(tile, value) (`retn 8`): user3 = -user0, then the value, then 0.0. Completion: 100%.
fn toggle_set_value(cx: &mut InterfaceCx, tile: TileId, value: f32) -> Result<(), GameError> {
    let own = -tile_numeric_trait(cx, tile, user(0));
    tile_set_numeric_trait(cx, tile, user(3), own)?;
    tile_set_numeric_trait(cx, tile, user(3), value)?;
    tile_set_numeric_trait(cx, tile, user(3), 0.0)
}

// IDA: N/A the player's base form (vt+0x170), its race (+0xE8) and its sex (TESActorBase_GetNPCSexOrMinusOne).
fn player_npc(cx: &InterfaceCx) -> Result<(Entity, Entity, Entity, i32), GameError> {
    let player = the_player(cx.world).ok_or(GameError::unclassified(
        0x005C7BA6,
        "005C7BA6: a null g_thePlayer faults",
    ))?;
    let npc = cx
        .world
        .get::<crate::tes::actor::TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
        .ok_or(GameError::unclassified(
            0x005C7BAC,
            "005C7BAC: the player with no base form",
        ))?;
    let race = crate::tes::forms_actors::base_race(cx.world, npc)?.ok_or(
        GameError::unclassified(0x005C7BB8, "005C7BB8: a null race faults"),
    )?;
    let sex = crate::tes::forms_actors::npc_sex(cx.world, npc).map_or(-1, |sex| sex as i32);
    Ok((player, npc, race, sex))
}

pub(crate) fn hair_flags(world: &World, hair: Entity) -> u8 {
    world
        .get::<crate::tes::forms_misc::TesHair>(hair)
        .map_or(0, |data| data.flags_48)
}

/// IDA: N/A host deferral D23's last part: TESDescription vt+0x10 (0046A710 → 0046A5F0) reads the race's DESC text back from
/// its plugin at +0x04 (TESFileData_GetForCurrentThread, SetOffset and the chunk walk) into the 00B33C08 cache. The port
/// keeps no plugin open after the load, so each read is recorded here and the tile takes "".
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct RaceDescriptionReads {
    pub races: Vec<Entity>,
}

/// IDA: 005C7BA0 RaceSexMenu_UpdatePages(menu, initial). +0x87C = 0. The race's eyes (+0xA8): with any, the Eyes toggle's
/// user13 is 1.0 for one and 2.0 for more, and each eye in list order: when the NPC has eyes (+0x1D0), the eye isn't
/// playable or one was counted, the NPC's own eye takes the Eyes name and +0x87C its index; otherwise the first playable eye
/// does and +0x87C = 0; each playable eye counts. With none, DataHandler +0x3C's first eyes whose texture is
/// "Characters\Eyes\EyeDefault.dds" (`_stricmp`) names it, count 0. Eyes: user4 0.0, user5 count - 1, and a user0 other than
/// +0x87C takes it (005C2B50). The races in DataHandler +0x44: the NPC's names the Race button and the Race toggle,
/// +0x86C its index among the playable (+0x70 bit 0) races before it, and its description (vt+0x10(0, DESC)) the Race
/// Description; the Race toggle's user4 0.0, user5 count - 1, user0 against +0x86C. The race's hairs (+0x8C): as the eyes,
/// with the NPC's hair (+0x1C8) and 0051FFD0, naming the Hair button and the Style toggle, +0x870; none names the Hair button
/// sBald. Style: user4, user5, user0 against +0x870. Hair Color: user4 0.0, user5 15.0, +0x88C its user0 (`ftol`). +0x878 =
/// the NPC's +0x1E8; not `initial`, or the setup pending, runs 005C5F00. The Red, Green and Blue sliders take +0x878's bytes
/// / 255.0; +0x874 = the hair length (+0x1CC) for Length. +0x868 = female; the Gender toggle names sMale or sFemale, user4
/// 0.0, user5 1.0, and (user0 == 2.0) against +0x868. The coefficients (00552990 of the race's +0x29C and 00521A10's): +0x880
/// = (1.0 - 0.0) x ((age - 15.0) / 50.0) + 0.0 (00553B30(0, 0)) into Age, and with s = -2.0 for a male, else 2.0, +0x884 =
/// ((gender - s) - -2.0) x 0.25 x (1.0 - 0.0) + 0.0 (00553B30(1, 0)) into Complexion. Answers 1. Completion: 100%, f32 for the
/// 24-bit x87, but the race description (`RaceDescriptionReads`).
/// # Errors
/// A null pane child, a null g_faceGen or a coefficient shape mismatch.
#[allow(clippy::too_many_lines)] // 005C7BA0's own length.
pub fn race_sex_menu_update_pages(
    cx: &mut InterfaceCx,
    menu: MenuId,
    initial: bool,
) -> Result<bool, GameError> {
    let (_, npc, race, sex) = player_npc(cx)?;
    race_sex_mut(cx, menu).eyes_index_87c = 0;
    let (race_eyes, race_hairs) = {
        let data = cx.world.get::<TesRace>(race).expect("a race");
        (data.eyes_a8.clone(), data.hairs_8c.clone())
    };
    let npc_data = cx.world.get::<TesNpc>(npc).expect("an NPC").clone();
    let mut counted: i32 = 0;
    if race_eyes.is_empty() {
        let lists = cx
            .world
            .get_resource::<crate::tes::data_handler::DataHandlerLists>()
            .map(|lists| lists.eyes_3c.items().collect::<Vec<_>>())
            .unwrap_or_default();
        for eyes in lists {
            let path = cx
                .world
                .get::<crate::tes::data_handler::TesTexture>(eyes)
                .map(|texture| crate::tes::face_gen_head::path_text(&texture.path_04))
                .unwrap_or_default();
            if oblivion_core::crt::stricmp(&path, b"Characters\\Eyes\\EyeDefault.dds") == 0 {
                let name = crate::tes::reference_3d::base_full_name(cx.world, eyes);
                let tile = page_child(cx, menu, GMST_MAIN, GMST_EYES)?;
                tile_set_string_trait(cx, tile, user(6), &name)?;
                break;
            }
        }
    } else {
        let toggle = page_child(cx, menu, GMST_MAIN, GMST_EYES)?;
        let single = if race_eyes.len() == 1 { 1.0 } else { 2.0 };
        tile_set_numeric_trait(cx, toggle, user(13), single)?;
        for eyes in race_eyes {
            let playable = eyes_playable(cx.world, eyes);
            if npc_data.eyes_1d0.is_some() || !playable || counted != 0 {
                if npc_data.eyes_1d0 == Some(eyes) {
                    let name = crate::tes::reference_3d::base_full_name(cx.world, eyes);
                    let tile = page_child(cx, menu, GMST_MAIN, GMST_EYES)?;
                    tile_set_string_trait(cx, tile, user(6), &name)?;
                    race_sex_mut(cx, menu).eyes_index_87c = counted;
                }
            } else {
                let name = crate::tes::reference_3d::base_full_name(cx.world, eyes);
                let tile = page_child(cx, menu, GMST_MAIN, GMST_EYES)?;
                tile_set_string_trait(cx, tile, user(6), &name)?;
                race_sex_mut(cx, menu).eyes_index_87c = 0;
            }
            if playable {
                counted += 1;
            }
        }
    }
    let eyes_tile = page_child(cx, menu, GMST_MAIN, GMST_EYES)?;
    tile_set_numeric_trait(cx, eyes_tile, user(4), 0.0)?;
    tile_set_numeric_trait(cx, eyes_tile, user(5), (counted - 1) as f32)?;
    let index = race_sex(cx, menu).eyes_index_87c;
    if f64::from(tile_numeric_trait(cx, eyes_tile, user(0))) != f64::from(index) {
        toggle_set_value(cx, eyes_tile, index as f32)?;
    }
    // The races.
    let races = cx
        .world
        .get_resource::<crate::tes::data_handler::DataHandlerLists>()
        .map(|lists| lists.races_44.items().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut playable_races: i32 = 0;
    for listed in races {
        if listed == race {
            let name = crate::tes::reference_3d::base_full_name(cx.world, listed);
            let tile = page_child(cx, menu, GMST_MAIN, GMST_RACE)?;
            tile_set_string_trait(cx, tile, user(6), &name)?;
            let tile = page_child(cx, menu, GMST_RACE, GMST_RACE)?;
            tile_set_string_trait(cx, tile, user(6), &name)?;
            race_sex_mut(cx, menu).race_index_86c = playable_races;
            cx.world
                .get_resource_or_init::<RaceDescriptionReads>()
                .races
                .push(listed);
            let tile = page_child(cx, menu, GMST_RACE, GMST_RACE_DESCRIPTION)?;
            tile_set_string_trait(cx, tile, user(6), b"")?;
        }
        if cx
            .world
            .get::<TesRace>(listed)
            .is_some_and(|data| data.data_50[0x20] & 1 != 0)
        {
            playable_races += 1;
        }
    }
    let race_tile = page_child(cx, menu, GMST_RACE, GMST_RACE)?;
    tile_set_numeric_trait(cx, race_tile, user(4), 0.0)?;
    tile_set_numeric_trait(cx, race_tile, user(5), (playable_races - 1) as f32)?;
    let index = race_sex(cx, menu).race_index_86c;
    if f64::from(tile_numeric_trait(cx, race_tile, user(0))) != f64::from(index) {
        toggle_set_value(cx, race_tile, index as f32)?;
    }
    // The hairs.
    let mut allowed_hairs: i32 = 0;
    if race_hairs.is_empty() {
        let bald = text_at_or_empty(cx.world, GMST_BALD);
        let tile = page_child(cx, menu, GMST_MAIN, GMST_HAIR)?;
        tile_set_string_trait(cx, tile, user(6), &bald)?;
    } else {
        for hair in race_hairs {
            let playable = hair_flags(cx.world, hair) & 1 != 0;
            let allowed = hair_allowed(cx.world, hair, npc)?;
            if npc_data.hair_1c8.is_some() || !playable || !allowed || allowed_hairs != 0 {
                if npc_data.hair_1c8 == Some(hair) {
                    let name = crate::tes::reference_3d::base_full_name(cx.world, hair);
                    let tile = page_child(cx, menu, GMST_MAIN, GMST_HAIR)?;
                    tile_set_string_trait(cx, tile, user(6), &name)?;
                    let tile = page_child(cx, menu, GMST_HAIR, GMST_STYLE)?;
                    tile_set_string_trait(cx, tile, user(6), &name)?;
                    race_sex_mut(cx, menu).hair_index_870 = allowed_hairs;
                }
            } else {
                let name = crate::tes::reference_3d::base_full_name(cx.world, hair);
                let tile = page_child(cx, menu, GMST_MAIN, GMST_HAIR)?;
                tile_set_string_trait(cx, tile, user(6), &name)?;
                let tile = page_child(cx, menu, GMST_HAIR, GMST_STYLE)?;
                tile_set_string_trait(cx, tile, user(6), &name)?;
                race_sex_mut(cx, menu).hair_index_870 = 0;
            }
            if playable && allowed {
                allowed_hairs += 1;
            }
        }
    }
    let style = page_child(cx, menu, GMST_HAIR, GMST_STYLE)?;
    tile_set_numeric_trait(cx, style, user(4), 0.0)?;
    tile_set_numeric_trait(cx, style, user(5), (allowed_hairs - 1) as f32)?;
    let index = race_sex(cx, menu).hair_index_870;
    if f64::from(tile_numeric_trait(cx, style, user(0))) != f64::from(index) {
        toggle_set_value(cx, style, index as f32)?;
    }
    let color = page_child(cx, menu, GMST_HAIR, GMST_HAIR_COLOR)?;
    tile_set_numeric_trait(cx, color, user(4), 0.0)?;
    tile_set_numeric_trait(cx, color, user(5), 15.0)?;
    let index = oblivion_core::x87::ftol(tile_numeric_trait(cx, color, user(0)));
    {
        let data = race_sex_mut(cx, menu);
        data.hair_color_index_88c = index;
        data.hair_color_878 = npc_data.hair_color_1e8;
    }
    let pending = cx
        .world
        .resource::<RaceSexMenuStatics>()
        .initial_setup_pending_b14500;
    if !initial || pending != 0 {
        race_sex_menu_apply_hair_color_preset(cx, menu)?;
    }
    let color = race_sex(cx, menu).hair_color_878;
    for (shift, name) in [(0, GMST_RED), (8, GMST_GREEN), (16, GMST_BLUE)] {
        let value = ((color >> shift) & 0xFF) as f32 / 255.0;
        let tile = page_child(cx, menu, GMST_HAIR, name)?;
        set_user3(cx, tile, value)?;
    }
    let length = cx.world.get::<TesNpc>(npc).expect("an NPC").hair_length_1cc;
    race_sex_mut(cx, menu).hair_length_874 = length;
    let tile = page_child(cx, menu, GMST_HAIR, GMST_LENGTH)?;
    set_user3(cx, tile, length)?;
    race_sex_mut(cx, menu).female_868 = u8::from(sex == 1);
    let gender_name = text_at_or_empty(cx.world, if sex == 0 { GMST_MALE } else { GMST_FEMALE });
    let gender = page_child(cx, menu, GMST_RACE, GMST_GENDER)?;
    tile_set_string_trait(cx, gender, user(6), &gender_name)?;
    tile_set_numeric_trait(cx, gender, user(4), 0.0)?;
    tile_set_numeric_trait(cx, gender, user(5), 1.0)?;
    let shown = i32::from(tile_numeric_trait(cx, gender, user(0)) == 2.0);
    let female = i32::from(race_sex(cx, menu).female_868);
    if female != shown {
        toggle_set_value(cx, gender, female as f32)?;
    }
    // 005C8F8F..: the age and gender controls of the race's and the NPC's coefficients.
    let race_coefficients = cx
        .world
        .get::<TesRace>(race)
        .expect("a race")
        .facegen_29c
        .clone();
    let own = npc_own_coefficients(cx.world, npc)?;
    let mut coefficients: [FaceGenMatrix; 4] = Default::default();
    combine_coefficients(&race_coefficients, &own, &mut coefficients)?;
    let age =
        crate::tes::face_gen_head::face_gen_age_gender_control(cx.world, &coefficients, 0, 0)?;
    let span = 1.0_f32 - 0.0;
    let age = span * ((age - 15.0) / 50.0) + 0.0;
    race_sex_mut(cx, menu).age_880 = age;
    let tile = page_child(cx, menu, GMST_MAIN, GMST_AGE)?;
    set_user3(cx, tile, age)?;
    let side = if sex == 0 { -2.0_f32 } else { 2.0 };
    let gender =
        crate::tes::face_gen_head::face_gen_age_gender_control(cx.world, &coefficients, 1, 0)?
            - side;
    let complexion = (gender - -2.0) * 0.25 * span + 0.0;
    race_sex_mut(cx, menu).complexion_884 = complexion;
    let tile = page_child(cx, menu, GMST_MAIN, GMST_COMPLEXION)?;
    set_user3(cx, tile, complexion)?;
    Ok(true)
}

/// IDA: 005C6EA0 RaceSexMenu_SetInitialDefaults(menu): the race's default hair for the NPC's sex (0052B490: race +0x94 +
/// 4 sex), when it has one, becomes the NPC's hair (+0x1C8); the race's hair colour index (+0x9C, `movsx`) goes into the
/// Hair Color toggle (user3 = -user0, the index, 0.0) and its sHairColorN (0052B780; past 15 a null string) into the
/// toggle's user6; then 005C5F00 and 005C34D0. Completion: 100%.
/// # Errors
/// A null pane child, or a stop in a callee.
pub fn race_sex_menu_set_initial_defaults(
    cx: &mut InterfaceCx,
    menu: MenuId,
) -> Result<(), GameError> {
    let (_, npc, race, sex) = player_npc(cx)?;
    let (default_hair, color_index) = {
        let data = cx.world.get::<TesRace>(race).expect("a race");
        let slot = usize::try_from(sex)
            .ok()
            .and_then(|sex| data.default_hairs_94.get(sex).copied())
            .ok_or(GameError::unclassified(
                0x005C6EEB,
                "005C6EEB: 0052B490 with no sex reads the hair list",
            ))?;
        let hair = match slot {
            crate::tes::form_components::FormRef::Resolved(hair) => hair,
            crate::tes::form_components::FormRef::Raw(0) => None,
            crate::tes::form_components::FormRef::Raw(_) => {
                return Err(GameError::unclassified(
                    0x005C6F09,
                    "005C6F09: an unresolved default hair (raw ID)",
                ));
            }
        };
        (hair, data.hair_color_9c as i8)
    };
    if let Some(hair) = default_hair {
        cx.world.get_mut::<TesNpc>(npc).expect("an NPC").hair_1c8 = Some(hair);
    }
    let toggle = page_child(cx, menu, GMST_HAIR, GMST_HAIR_COLOR)?;
    toggle_set_value(cx, toggle, f32::from(color_index))?;
    let name = match color_index {
        0..=15 => text_at_or_empty(cx.world, GMST_HAIR_COLOR_NAMES + 8 * color_index as u32),
        _ => Vec::new(),
    };
    let toggle = page_child(cx, menu, GMST_HAIR, GMST_HAIR_COLOR)?;
    tile_set_string_trait(cx, toggle, user(6), &name)?;
    race_sex_menu_apply_hair_color_preset(cx, menu)?;
    race_sex_menu_update_hair_color_toggles(cx, menu)
}

/// IDA: 005C5F00 RaceSexMenu_ApplyHairColorPreset(menu): +0x878 = the colour at +0x8F0 + 4 x `ftol`(Hair Color's user0);
/// the Red, Green and Blue sliders take its bytes / 255.0 (user3: INT_MIN, the value, 0.0); the NPC's hair (+0x1C8) not
/// tintable (0051FE90) blanks Hair Color's user6, else it takes the +0x930 name at that index; then 005C5C30(0).
/// Completion: 100%.
/// # Errors
/// An index past the 16, a null hair (0051FE90 faults), a null pane child or a stop below.
pub fn race_sex_menu_apply_hair_color_preset(
    cx: &mut InterfaceCx,
    menu: MenuId,
) -> Result<(), GameError> {
    let index =
        oblivion_core::x87::ftol(page_number(cx, menu, GMST_HAIR, GMST_HAIR_COLOR, user(0))?);
    let color = *race_sex(cx, menu)
        .hair_colors_8f0
        .get(usize::try_from(index).unwrap_or(usize::MAX))
        .ok_or(GameError::unclassified(
            0x005C5F59,
            "005C5F59: a hair colour index past the 16",
        ))?;
    race_sex_mut(cx, menu).hair_color_878 = color;
    for (shift, name) in [(0, GMST_RED), (8, GMST_GREEN), (16, GMST_BLUE)] {
        let value = ((color >> shift) & 0xFF) as f32 / 255.0;
        let tile = page_child(cx, menu, GMST_HAIR, name)?;
        set_user3(cx, tile, value)?;
    }
    let (_, npc, _, _) = player_npc(cx)?;
    let hair = cx
        .world
        .get::<TesNpc>(npc)
        .expect("an NPC")
        .hair_1c8
        .ok_or(GameError::unclassified(
            0x005C60D5,
            "005C60D5: 0051FE90 on a null hair faults",
        ))?;
    let label = if hair_flags(cx.world, hair) & 8 != 0 {
        Vec::new()
    } else {
        let index =
            oblivion_core::x87::ftol(page_number(cx, menu, GMST_HAIR, GMST_HAIR_COLOR, user(0))?);
        race_sex(cx, menu)
            .hair_color_names_930
            .get(usize::try_from(index).unwrap_or(usize::MAX))
            .cloned()
            .ok_or(GameError::unclassified(
                0x005C6153,
                "005C6153: a hair colour name index past the 16",
            ))?
    };
    let tile = page_child(cx, menu, GMST_HAIR, GMST_HAIR_COLOR)?;
    tile_set_string_trait(cx, tile, user(6), &label)?;
    race_sex_menu_store_hair_color(cx, menu, false).map(|_| ())
}

// IDA: N/A 005C34D0's and 005C5C30's packing: `ftol`(255.0 x red) | (`ftol`(green x 255.0) & 0xFF | (`ftol`(blue x 255.0)
// & 0xFF) << 8) << 8.
fn slider_color(cx: &InterfaceCx, menu: MenuId) -> Result<u32, GameError> {
    let red = page_number(cx, menu, GMST_HAIR, GMST_RED, user(0))?;
    let green = page_number(cx, menu, GMST_HAIR, GMST_GREEN, user(0))?;
    let blue = page_number(cx, menu, GMST_HAIR, GMST_BLUE, user(0))?;
    let blue = oblivion_core::x87::ftol(blue * 255.0) as u32;
    let green = oblivion_core::x87::ftol(green * 255.0) as u32;
    let red = oblivion_core::x87::ftol(255.0 * red) as u32;
    Ok(red | (((blue & 0xFF) << 8 | (green & 0xFF)) << 8))
}

/// IDA: 005C34D0 RaceSexMenu_UpdateHairColorToggles(menu): with the NPC's hair not tintable (0051FE90), Hair Color's user13
/// = 1.0 and user6 "", and the Red, Green and Blue sliders' user13 1.0; otherwise the sliders' colour (`slider_color`), Hair
/// Color's user13 2.0, its user6 the +0x930 name when the colour at +0x8F0 + 4 x Hair Color's user0 equals it, else
/// sCustomColor, and the sliders' user13 2.0. Completion: 100%.
/// # Errors
/// A null hair, a null pane child or an index past the 16.
pub fn race_sex_menu_update_hair_color_toggles(
    cx: &mut InterfaceCx,
    menu: MenuId,
) -> Result<(), GameError> {
    let (_, npc, _, _) = player_npc(cx)?;
    let hair = cx
        .world
        .get::<TesNpc>(npc)
        .expect("an NPC")
        .hair_1c8
        .ok_or(GameError::unclassified(
            0x005C34F5,
            "005C34F5: 0051FE90 on a null hair faults",
        ))?;
    let sliders = [GMST_RED, GMST_GREEN, GMST_BLUE];
    if hair_flags(cx.world, hair) & 8 != 0 {
        let tile = page_child(cx, menu, GMST_HAIR, GMST_HAIR_COLOR)?;
        tile_set_numeric_trait(cx, tile, user(13), 1.0)?;
        let tile = page_child(cx, menu, GMST_HAIR, GMST_HAIR_COLOR)?;
        tile_set_string_trait(cx, tile, user(6), b"")?;
        for name in sliders {
            let tile = page_child(cx, menu, GMST_HAIR, name)?;
            tile_set_numeric_trait(cx, tile, user(13), 1.0)?;
        }
        return Ok(());
    }
    let packed = slider_color(cx, menu)?;
    let tile = page_child(cx, menu, GMST_HAIR, GMST_HAIR_COLOR)?;
    tile_set_numeric_trait(cx, tile, user(13), 2.0)?;
    let index =
        oblivion_core::x87::ftol(page_number(cx, menu, GMST_HAIR, GMST_HAIR_COLOR, user(0))?);
    let lookup = |index: i32| -> Result<usize, GameError> {
        usize::try_from(index)
            .ok()
            .filter(|&index| index < 16)
            .ok_or(GameError::unclassified(
                0x005C3712,
                "005C3712: a hair colour index past the 16",
            ))
    };
    let label = if packed == race_sex(cx, menu).hair_colors_8f0[lookup(index)?] {
        let index =
            oblivion_core::x87::ftol(page_number(cx, menu, GMST_HAIR, GMST_HAIR_COLOR, user(0))?);
        race_sex(cx, menu).hair_color_names_930[lookup(index)?].clone()
    } else {
        text_at_or_empty(cx.world, GMST_CUSTOM_COLOR)
    };
    let tile = page_child(cx, menu, GMST_HAIR, GMST_HAIR_COLOR)?;
    tile_set_string_trait(cx, tile, user(6), &label)?;
    for name in sliders {
        let tile = page_child(cx, menu, GMST_HAIR, name)?;
        tile_set_numeric_trait(cx, tile, user(13), 2.0)?;
    }
    Ok(())
}

/// IDA: 005C5C30 RaceSexMenu_StoreHairColor(menu, label) (`retn 4`): the sliders' colour (`slider_color`); with `label`, Hair
/// Color's user6 is "" for a hair that isn't tintable, else sCustomColor; the NPC's +0x1E8 = the colour; then 005C50A0(0).
/// Completion: 100%.
/// # Errors
/// A null pane child, a null hair with `label`, or a stop in 005C50A0.
pub fn race_sex_menu_store_hair_color(
    cx: &mut InterfaceCx,
    menu: MenuId,
    label: bool,
) -> Result<bool, GameError> {
    let packed = slider_color(cx, menu)?;
    let (_, npc, _, _) = player_npc(cx)?;
    if label {
        let hair = cx
            .world
            .get::<TesNpc>(npc)
            .expect("an NPC")
            .hair_1c8
            .ok_or(GameError::unclassified(
                0x005C5D7A,
                "005C5D7A: 0051FE90 on a null hair faults",
            ))?;
        let text = if hair_flags(cx.world, hair) & 8 != 0 {
            Vec::new()
        } else {
            text_at_or_empty(cx.world, GMST_CUSTOM_COLOR)
        };
        let tile = page_child(cx, menu, GMST_HAIR, GMST_HAIR_COLOR)?;
        tile_set_string_trait(cx, tile, user(6), &text)?;
    }
    cx.world
        .get_mut::<TesNpc>(npc)
        .expect("an NPC")
        .hair_color_1e8 = packed;
    race_sex_menu_update_hair(cx, menu, false)
}

// IDA: N/A a node's child slots in order (+0xB0, its +0xB6 count).
fn child_slots(world: &World, node: Entity) -> Vec<Option<Entity>> {
    world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(node)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default()
}

// IDA: N/A vt+0x10 on a scene object: a NiTriShape or NiTriStrips answers itself, anything else null.
fn as_geometry(world: &World, object: Entity) -> Option<Entity> {
    matches!(
        world.get::<crate::gamebryo::stream_objects::NiClass>(object),
        Some(
            crate::gamebryo::stream_objects::NiClass::TriShape
                | crate::gamebryo::stream_objects::NiClass::TriStrips
        )
    )
    .then_some(object)
}

// IDA: N/A an object's NiObjectNET name.
fn object_name(world: &World, object: Entity) -> Vec<u8> {
    world
        .get::<crate::gamebryo::object_net::NiObjectNet>(object)
        .and_then(|net| net.name.clone())
        .unwrap_or_default()
}

/// IDA: 005C50A0 RaceSexMenu_UpdateHair(menu, rebuild) (`retn 4`): the player's FaceGen biped node (vt+0x130(0)); none answers
/// 0. With `rebuild`: the race's hairs (+0x8C) walked from a count of -1, each playable (`0051FE80`) and allowed (`0051FFD0`)
/// one counting, to the node where the count meets +0x870 (a null race, an end of the list or a null hair there answers 0);
/// its name (+0x1C) into Main's Hair and Hair's Style user6, the NPC's hair (+0x1C8) = it, and 005C34D0. Then the node's
/// children in slot order (a null slot faults) up to "FaceGenHair" (`repe cmpsb`, 12 bytes; none answers 1): its vt+0x10
/// geometry; with `rebuild` the hair made again and swapped in (`face_gen_nodes::swap_hair`, with the driver's model
/// services), otherwise 00550980 with the NPC's hair length and its type-4 shader property's tint (type 5 at +0xA8, type 10
/// at +0xF0: the NPC's +0x1E8 bytes / 255.0 and 1.0; `HairTint`); then NiAVObject_Update(node, 0.0, 1). Answers 1.
/// Completion: 100%, with `swap_hair`'s texturing property D13's.
/// # Errors
/// A null child slot, no model services, or a stop in the swap or the update.
pub fn race_sex_menu_update_hair(
    cx: &mut InterfaceCx,
    menu: MenuId,
    rebuild: bool,
) -> Result<bool, GameError> {
    let (player, npc, _, _) = player_npc(cx)?;
    let Some(node) = player_face_node(cx, player, b"BSFaceGenNiNodeBiped") else {
        return Ok(false);
    };
    if rebuild {
        // 005C5129..005C5312: the hair the Style toggle names.
        let Some(race) = crate::tes::forms_actors::base_race(cx.world, npc)? else {
            return Ok(false);
        };
        let list = cx
            .world
            .get::<TesRace>(race)
            .expect("a race")
            .hairs_8c
            .clone();
        let index = race_sex(cx, menu).hair_index_870;
        let nodes = list.len().max(1);
        let mut counted: i32 = -1;
        let mut at = Some(0_usize);
        while let Some(node_at) = at {
            if counted == index {
                break;
            }
            if let Some(&listed) = list.get(node_at)
                && hair_flags(cx.world, listed) & 1 != 0
                && hair_allowed(cx.world, listed, npc)?
            {
                counted += 1;
            }
            if counted != index {
                at = (node_at + 1 < nodes).then_some(node_at + 1);
            }
        }
        let Some(picked) = at.and_then(|node_at| list.get(node_at).copied()) else {
            return Ok(false);
        };
        let name = crate::tes::reference_3d::base_full_name(cx.world, picked);
        let tile = page_child(cx, menu, GMST_MAIN, GMST_HAIR)?;
        tile_set_string_trait(cx, tile, user(6), &name)?;
        let tile = page_child(cx, menu, GMST_HAIR, GMST_STYLE)?;
        tile_set_string_trait(cx, tile, user(6), &name)?;
        cx.world.get_mut::<TesNpc>(npc).expect("an NPC").hair_1c8 = Some(picked);
        race_sex_menu_update_hair_color_toggles(cx, menu)?;
    }
    let mut hair = None;
    for slot in child_slots(cx.world, node) {
        let child = slot.ok_or(GameError::unclassified(
            0x005C53EE,
            "005C53EE: a null child slot under the FaceGen node faults",
        ))?;
        if object_name(cx.world, child) == b"FaceGenHair" {
            hair = Some(child);
            break;
        }
    }
    let Some(geometry) = hair.and_then(|hair| as_geometry(cx.world, hair)) else {
        return Ok(true);
    };
    if rebuild {
        let picked = cx
            .world
            .get::<TesNpc>(npc)
            .expect("an NPC")
            .hair_1c8
            .expect("picked above");
        let InterfaceCx { world, runtime, .. } = cx;
        runtime.with_model_services(world, &mut |world, services| {
            swap_hair(world, npc, picked, node, geometry, services)
        })?;
        crate::tes::face_gen_nodes::face_gen_node_update(cx.world, node, 0.0, true)?;
        crate::tes::reference_3d::update_with_controllers(cx.world, node, 0.0)?;
        return Ok(true);
    }
    let (length, color) = {
        let data = cx.world.get::<TesNpc>(npc).expect("an NPC");
        (data.hair_length_1cc, data.hair_color_1e8)
    };
    crate::tes::face_gen_nodes::apply_hair_length(cx.world, geometry, length);
    let channel = |shift: u32| ((color >> shift) & 0xFF) as f32 / 255.0;
    cx.world
        .entity_mut(geometry)
        .insert(crate::tes::face_gen_nodes::HairTint([
            channel(0),
            channel(8),
            channel(16),
            1.0,
        ]));
    crate::tes::face_gen_nodes::face_gen_node_update(cx.world, node, 0.0, true)?;
    crate::tes::reference_3d::update_with_controllers(cx.world, node, 0.0)?;
    Ok(true)
}

/// IDA: N/A what 005C7070 left, for a test: the morph's coefficient difference (005C73E4's v54) and whether any of it was
/// nonzero.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct RaceSexFaceMorph {
    pub runs: u32,
    pub difference: [FaceGenMatrix; 4],
}

/// IDA: 005C7070 RaceSexMenu_UpdateFace(textures) (cdecl): the RaceSexMenu (TileMenu_GetByMenuID, its containing menu) and
/// the player's two FaceGen nodes (vt+0x130(0), vt+0x134(0)), else return. The process's vt+0x5C (High and MiddleHigh
/// 00629540: +0x15C = 0). The coefficients (00552990 of the race's +0x29C and 00521A10's); the age controls A (geometry,
/// 00553B30(0, 0)) and B (texture, (0, 1)); Y = ((+0x880 - 0.0) / (1.0 - 0.0)) x 50.0 + 15.0 and T = B - (A - Y), each
/// clamped to [15, 65] (`fcom`), into 00555A00(0, 0, Y) and (0, 1, T). With s = 00522230 (2.0, or -2.0 for a male), G = the
/// gender control (1, 0) - s, H = (1, 1) - s, Z = (+0x884 - 0.0) / (1.0 - 0.0) x 4.0 - 2.0 and W = H - (G - Z), each clamped
/// to [-2, 2], then 00555A00(1, 0, s + Z) and (1, 1, s + W). 00552880 into a second set, 005221C0's into a third, and
/// 00552C10 their difference into the second; 00552990 adds it into the NPC's own coefficients (00521A10's, in place). Each
/// of the two nodes' children, as a geometry (vt+0x10): FaceGenHair takes 00550980 with the hair length, the part's
/// BSFaceGenModelExtraData (00550790) has 00558840(model, difference, part, 1.0, null) applied, and FaceGenHair 00550980
/// again. With `textures`, `race_sex_menu_drop_head`. Completion: 100%, f32 for the 24-bit x87.
/// # Errors
/// `textures`, an EGM the host can't reload, a coefficient shape mismatch or a null g_faceGen.
pub fn race_sex_menu_update_face(cx: &mut InterfaceCx, textures: bool) -> Result<(), GameError> {
    let Some(tile) = cx.engine.tile_menu_by_id(MENU_ID_RACE_SEX) else {
        return Ok(());
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(());
    };
    let (player, npc, race, sex) = player_npc(cx)?;
    let (Some(biped), Some(skinned)) = (
        player_face_node(cx, player, b"BSFaceGenNiNodeBiped"),
        player_face_node(cx, player, b"BSFaceGenNiNodeSkinned"),
    ) else {
        return Ok(());
    };
    // 005C7116 the process's vt+0x5C.
    if let Some(process) = cx
        .world
        .get::<crate::tes::actor::MobileObject>(player)
        .and_then(|mobile| mobile.process_58)
        && let Some(mut middle) = cx
            .world
            .get_mut::<crate::tes::actor_process::MiddleHighProcess>(process)
    {
        middle.face_animation_data_15c = None;
    }
    let race_coefficients = cx
        .world
        .get::<TesRace>(race)
        .expect("a race")
        .facegen_29c
        .clone();
    let own = npc_own_coefficients(cx.world, npc)?;
    let mut coefficients: [FaceGenMatrix; 4] = Default::default();
    combine_coefficients(&race_coefficients, &own, &mut coefficients)?;
    let control = crate::tes::face_gen_head::face_gen_age_gender_control;
    let set = crate::tes::face_gen_head::face_gen_set_age_gender_control;
    let geometry_age = control(cx.world, &coefficients, 0, 0)?;
    let texture_age = control(cx.world, &coefficients, 0, 1)?;
    let (age, complexion) = {
        let data = race_sex(cx, menu);
        (data.age_880, data.complexion_884)
    };
    let span = 1.0_f32 - 0.0;
    let year = (age - 0.0) / span * 50.0 + 15.0;
    let texture_year = texture_age - (geometry_age - year);
    let clamp = |value: f32, low: f32, high: f32| {
        if value > low && value >= high {
            high
        } else if value <= low {
            low
        } else {
            value
        }
    };
    let year = clamp(year, 15.0, 65.0);
    let texture_year = clamp(texture_year, 15.0, 65.0);
    set(cx.world, &mut coefficients, 0, 0, year)?;
    set(cx.world, &mut coefficients, 0, 1, texture_year)?;
    let side = if sex != 0 { 2.0_f32 } else { -2.0 };
    let geometry_gender = control(cx.world, &coefficients, 1, 0)? - side;
    let texture_gender = control(cx.world, &coefficients, 1, 1)? - side;
    let gender = (complexion - 0.0) / span * 4.0 - 2.0;
    let texture = texture_gender - (geometry_gender - gender);
    let gender = clamp(gender, -2.0, 2.0);
    let texture = clamp(texture, -2.0, 2.0);
    set(cx.world, &mut coefficients, 1, 0, side + gender)?;
    set(cx.world, &mut coefficients, 1, 1, side + texture)?;
    let mut difference: [FaceGenMatrix; 4] = Default::default();
    crate::tes::face_gen_head::shape_coefficients(&mut difference);
    let current = crate::tes::face_gen_head::npc_face_gen_coefficients(cx.world, npc)?;
    crate::tes::face_gen_head::coefficients_difference(&current, &coefficients, &mut difference);
    let mut updated: [FaceGenMatrix; 4] = Default::default();
    combine_coefficients(&difference, &own, &mut updated)?;
    {
        let vampire = crate::tes::actor_values::base_actor_value(cx.world, npc, 69)? != 0;
        let mut data = cx.world.get_mut::<TesNpc>(npc).expect("an NPC");
        if vampire {
            data.facegen_168 = updated;
        } else {
            data.facegen_108 = updated;
        }
    }
    {
        let mut record = cx.world.get_resource_or_init::<RaceSexFaceMorph>();
        record.runs += 1;
        record.difference = difference.clone();
    }
    let length = cx.world.get::<TesNpc>(npc).expect("an NPC").hair_length_1cc;
    for node in [biped, skinned] {
        for child in child_slots(cx.world, node).into_iter().flatten() {
            let Some(geometry) = as_geometry(cx.world, child) else {
                continue;
            };
            let hair = object_name(cx.world, geometry) == b"FaceGenHair";
            if hair {
                crate::tes::face_gen_nodes::apply_hair_length(cx.world, geometry, length);
            }
            if apply_part_morph(cx, geometry, &difference)? && hair {
                crate::tes::face_gen_nodes::apply_hair_length(cx.world, geometry, length);
            }
        }
    }
    if textures {
        race_sex_menu_drop_head(cx, player, npc)?;
    }
    Ok(())
}

/// IDA: 005C7070's `textures` arm (005C75A4..005C777A), the head dropped for 005C2F20 to build again: with the player's 3D
/// (vt+0x154) a node, the animation data's palette (+0x98 +0x7C), `007C5E70` on the skinned (vt+0x134) then the biped
/// (vt+0x130) node (counted in `Equipment3DBoundaries`), then each of the two with a parent leaves the palette (`00716620`)
/// and its parent (vt+0x88, the pointer released); the process's vt+0x5C (+0x15C = 0); `00405CE0` (the NPC's +0x1D4 and
/// +0x1D8 released); a face description (`0052CD50`) into `005551C0` on vt+0x130's then vt+0x134's node, which are no longer
/// found (both detached and out of the palette), so both calls return at once. Completion: 100%.
/// # Errors
/// A stop in the description.
fn race_sex_menu_drop_head(
    cx: &mut InterfaceCx,
    player: Entity,
    npc: Entity,
) -> Result<(), GameError> {
    let root = cx
        .world
        .get::<crate::tes::reference_3d::Reference3D>(player)
        .map(|loaded| loaded.0)
        .filter(|&node| {
            cx.world
                .get::<crate::gamebryo::scene_node::NiNodeChildren>(node)
                .is_some()
        });
    if root.is_some() {
        let palette = player_object_palette(cx.world, player);
        for _ in 0..2 {
            crate::tes::equipment_3d::count(cx.world, 0x007C_5E70);
        }
        for name in [&b"BSFaceGenNiNodeSkinned"[..], b"BSFaceGenNiNodeBiped"] {
            let Some(node) = player_face_gen_node(cx.world, player, name) else {
                continue;
            };
            let parent = cx
                .world
                .get::<crate::gamebryo::av_object::NiAvParent>(node)
                .and_then(|parent| parent.0);
            if let Some(parent) = parent {
                crate::gamebryo::av_object_palette::remove_scene_objects(
                    cx.world,
                    Some(node),
                    palette,
                );
                crate::gamebryo::scene_node::detach_child(cx.world, parent, node);
            }
        }
    }
    if let Some(process) = crate::tes::actor::process_of(cx.world, player)
        && let Some(mut middle) = cx
            .world
            .get_mut::<crate::tes::actor_process::MiddleHighProcess>(process)
    {
        middle.face_animation_data_15c = None;
    }
    cx.world
        .entity_mut(npc)
        .insert(crate::tes::face_gen_nodes::FaceGenHeadNodes::default());
    let settings = cx
        .world
        .get_resource::<crate::tes::face_gen_tasks::FaceGenHeadSettings>()
        .copied()
        .unwrap_or_default();
    let description = crate::tes::face_gen_head::fill_face_description(
        cx.world,
        npc,
        settings.face_texturing_b10d3c,
        settings.skip_lod_head_b120b0,
    )?;
    let nodes: Vec<Entity> = [&b"BSFaceGenNiNodeBiped"[..], b"BSFaceGenNiNodeSkinned"]
        .into_iter()
        .filter_map(|name| player_face_gen_node(cx.world, player, name))
        .collect();
    apply_face_gen_textures(cx, &nodes, &description)
}

/// IDA: N/A `005551C0` FaceGen_ApplyFaceTexture on each node as the race menu's sites run it (the main thread, 24-bit x87):
/// its hair arm (`apply_hair_tint`) and its face and ears arm (`apply_face_texture`). Completion: those two arms; the eyes'
/// arms are counted at the sites.
fn apply_face_gen_textures(
    cx: &mut InterfaceCx,
    nodes: &[Entity],
    description: &crate::tes::face_gen_head::FaceDescription,
) -> Result<(), GameError> {
    for &node in nodes {
        crate::tes::face_gen_nodes::apply_hair_tint(cx.world, node, description.hair_color_64);
    }
    let InterfaceCx { world, runtime, .. } = cx;
    runtime.with_model_services(world, &mut |world, services| {
        crate::tes::face_gen_tasks::with_face_gen_files(
            world,
            true,
            services,
            &mut |world, files, _| {
                for &node in nodes {
                    crate::tes::face_gen_nodes::apply_face_texture(
                        world,
                        node,
                        description,
                        files,
                    )?;
                }
                Ok(())
            },
        )
    })
}

/// IDA: N/A the race menu's calls counted rather than made: `005C938C` and `005C93AC`, 005C91E0's two `005551C0` calls' face texture and eye arms (D13; their
/// hair arm runs); `005C418E`, 005C3E10's NiTexturingProperty for the eyes (D13; the path is kept).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct RaceSexMenuDeferrals {
    pub calls: std::collections::BTreeMap<u32, u32>,
}

/// IDA: N/A the stack QueuedHead `005C2F20` builds (00437970 on the stack): an id no IO task takes.
const STACK_QUEUED_HEAD: crate::bs::model_loader::IoTaskId =
    crate::bs::model_loader::IoTaskId(usize::MAX);

/// IDA: 005C2F20 (cdecl), the player's head rebuilt in place inside the race menu's frame: `00550240`(the base's form ID)
/// (counted, `EquipmentRebuildFlag`); 00B33D80 = 1; the process's vt+0x5C (+0x15C = 0), vt+0x31C(1) and vt+0x318 (`0063CDC0`
/// with the driver's model services); with neither FaceGen node (vt+0x134, vt+0x130), a QueuedHead on the stack (`00437970`
/// with the actor base, priority 0): `00435300` QueuedHead_Run, its vt+0x28 (`00436F30` QueuedFile_NotifyParent: no parent,
/// nothing), `004353D0` QueuedHead_FinishForActor and `004379A0`; 00B33D80 = 0; the first-person node's (+0x5D0) child 0 takes
/// the race-scaled identity (`004D7C50`, not the inverse); `00434020`(5) with the drain built here (the timed pass through
/// the lent model services, the section progress on this frame's interface with no scene runtime, then the reticle requests).
/// Completion: 100% of the order; `0063CDC0`'s and the drain's stops are theirs.
/// # Errors
/// No model services, a first-person node with no child, or a stop in the rebuild or the drain.
pub fn race_sex_menu_rebuild_head(cx: &mut InterfaceCx) -> Result<(), GameError> {
    use crate::tes::biped_attach::EquipmentRebuildFlag;
    let (player, npc, _, _) = player_npc(cx)?;
    cx.world
        .get_resource_or_init::<EquipmentRebuildFlag>()
        .palette_evictions_550240 += 1;
    cx.world
        .resource_mut::<EquipmentRebuildFlag>()
        .active_b33d80 = true;
    let process = crate::tes::actor::process_of(cx.world, player).ok_or(
        GameError::unclassified(0x005C2F20, "005C2F20: the player with no process faults"),
    )?;
    {
        let mut middle = cx
            .world
            .get_mut::<crate::tes::actor_process::MiddleHighProcess>(process)
            .ok_or(GameError::unclassified(
                0x005C2F20,
                "005C2F20: the player's process with no MiddleHigh part",
            ))?;
        middle.face_animation_data_15c = None;
        middle.equipment_update_pending_16c = 1;
    }
    let inventory_hidden = crate::tes::interface_manager::menu_is_visible(cx, 1002);
    let InterfaceCx { world, runtime, .. } = cx;
    runtime.with_model_services(world, &mut |world, services| {
        crate::tes::equipment_3d::update_equipment_3d(
            world,
            process,
            player,
            inventory_hidden,
            services,
        )?;
        let node = world
            .get::<crate::tes::reference_3d::Reference3D>(player)
            .map(|loaded| loaded.0);
        let face_node = |world: &World, name: &[u8]| {
            node.and_then(|node| {
                crate::gamebryo::object_net::object_by_name_with_palette(world, node, name)
            })
        };
        if face_node(world, b"BSFaceGenNiNodeSkinned").is_none()
            && face_node(world, b"BSFaceGenNiNodeBiped").is_none()
        {
            world
                .get_resource_or_init::<crate::tes::face_gen_tasks::FaceGenTasks>()
                .heads
                .insert(
                    STACK_QUEUED_HEAD,
                    crate::tes::face_gen_tasks::QueuedHead {
                        base_20: npc,
                        biped_24: None,
                        skinned_28: None,
                        parts: Vec::new(),
                        deferred: Vec::new(),
                    },
                );
            let built = crate::tes::face_gen_tasks::run_queued_head_in_walk(
                world,
                STACK_QUEUED_HEAD,
                true,
                services,
            )
            .and_then(|()| {
                let defaults = services.prepare.property_defaults;
                crate::tes::face_gen_nodes::queued_head_finish_for_actor(
                    world,
                    STACK_QUEUED_HEAD,
                    player,
                    &defaults,
                )
            });
            world
                .resource_mut::<crate::tes::face_gen_tasks::FaceGenTasks>()
                .heads
                .remove(&STACK_QUEUED_HEAD);
            built?;
        }
        Ok(())
    })?;
    cx.world
        .resource_mut::<EquipmentRebuildFlag>()
        .active_b33d80 = false;
    // 005C2F20 last: the first-person node's child 0 (a null one faults) takes the race-scaled identity.
    let first_person = cx
        .world
        .get::<crate::tes::player::PlayerCharacter>(player)
        .and_then(|body| body.first_person_node_5d0)
        .ok_or(GameError::unclassified(
            0x005C2F20,
            "005C2F20: a null first-person node faults",
        ))?;
    let child = cx
        .world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(first_person)
        .and_then(|children| children.slots_ac.first().copied().flatten())
        .ok_or(GameError::unclassified(
            0x005C2F20,
            "005C2F20: a first-person node with no child 0 faults",
        ))?;
    let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let scaled =
        crate::tes::biped_attach::race_scaled_rotation(cx.world, player, &identity, false)?;
    if let Some(mut payload) = cx
        .world
        .get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(child)
    {
        payload.local_transform.rotation = scaled;
    }
    race_sex_menu_drain_through_stage(cx, 5)
}

// IDA: 00434020 IOManager_DrainTasksThroughStage(stage) from interface code: the timed pass through the runtime's model
// services, 0057B950's progress on this frame's interface (no scene runtime: the loading menu's progress updates no
// controller), Sleep, then 00578CF0's queued reticle requests (`ReticleRequests`).
fn race_sex_menu_drain_through_stage(cx: &mut InterfaceCx, stage: i32) -> Result<(), GameError> {
    let InterfaceCx {
        world,
        engine,
        settings,
        host,
        runtime,
    } = cx;
    let mut stopped: Option<GameError> = None;
    let mut run_timed = |world: &mut World| -> Result<(), GameError> {
        runtime.with_model_services(world, &mut |world, services| {
            crate::bs::model_loader::ModelDrain::new()
                .run_to_end(world, services)
                .map(|_| ())
        })
    };
    let mut section = |world: &mut World, section: i32, percent: f32| {
        if stopped.is_some() {
            return;
        }
        let mut controllers = crate::gamebryo::scene_update::NodeControllersOnly;
        let mut inner = InterfaceCx {
            world,
            engine,
            settings,
            host: &mut **host,
            runtime: &mut controllers,
        };
        if let Err(error) = super::loading_menu::loading_menu_set_section_progress_if_ready(
            &mut inner, section, percent,
        ) {
            stopped = Some(error);
        }
    };
    let mut save_load = |_: i32, _: f32| {};
    let mut sleep = |ms: u32| std::thread::sleep(std::time::Duration::from_millis(u64::from(ms)));
    let result = crate::bs::model_loader::io_drain_tasks_through_stage(
        world,
        stage,
        &mut crate::bs::model_loader::IoDrainServices {
            run_timed_tasks: &mut run_timed,
            section_progress: &mut section,
            save_load_progress: &mut save_load,
            sleep: &mut sleep,
        },
    );
    if let Some(error) = stopped {
        return Err(error);
    }
    result?;
    super::hud_reticle::run_reticle_requests(cx)
}

/// IDA: 005C4920 (thiscall menu), the hair after a sex change: the race's default hair for the NPC's sex (0052B490), when
/// there is one, becomes the NPC's (+0x1C8). A hair that isn't playable (0051FE80) or allowed for the NPC (0051FFD0) is
/// replaced by the first of the race's +0x8C list that is both, at index i: Main's Hair and Hair's Style take its name
/// (user6), Style takes i (005C2B50) and +0x870 = i; with none the NPC has no hair and Main's Hair names sBald. Then 005C34D0.
/// Completion: 100%.
/// # Errors
/// A null pane child, an unresolved default hair, or a stop in 005C34D0.
pub fn race_sex_menu_apply_gender_hair(
    cx: &mut InterfaceCx,
    menu: MenuId,
) -> Result<(), GameError> {
    let (_, npc, race, sex) = player_npc(cx)?;
    let (default_hair, hairs) = {
        let data = cx.world.get::<TesRace>(race).expect("a race");
        let slot = usize::try_from(sex)
            .ok()
            .and_then(|sex| data.default_hairs_94.get(sex).copied());
        let hair = match slot {
            Some(crate::tes::form_components::FormRef::Resolved(hair)) => hair,
            Some(crate::tes::form_components::FormRef::Raw(0)) | None => None,
            Some(crate::tes::form_components::FormRef::Raw(_)) => {
                return Err(GameError::unclassified(
                    0x005C4920,
                    "005C4920: an unresolved default hair (raw ID)",
                ));
            }
        };
        (hair, data.hairs_8c.clone())
    };
    if let Some(hair) = default_hair {
        cx.world.get_mut::<TesNpc>(npc).expect("an NPC").hair_1c8 = Some(hair);
    }
    let current = cx.world.get::<TesNpc>(npc).expect("an NPC").hair_1c8;
    if let Some(hair) = current
        && (hair_flags(cx.world, hair) & 1 == 0 || !hair_allowed(cx.world, hair, npc)?)
    {
        let mut found = None;
        for (index, &listed) in hairs.iter().enumerate() {
            if hair_flags(cx.world, listed) & 1 != 0 && hair_allowed(cx.world, listed, npc)? {
                found = Some((index, listed));
                break;
            }
        }
        if let Some((index, listed)) = found {
            let name = crate::tes::reference_3d::base_full_name(cx.world, listed);
            let tile = page_child(cx, menu, GMST_MAIN, GMST_HAIR)?;
            tile_set_string_trait(cx, tile, user(6), &name)?;
            let tile = page_child(cx, menu, GMST_HAIR, GMST_STYLE)?;
            tile_set_string_trait(cx, tile, user(6), &name)?;
            toggle_set_value(cx, tile, index as f32)?;
            race_sex_mut(cx, menu).hair_index_870 = index as i32;
            cx.world.get_mut::<TesNpc>(npc).expect("an NPC").hair_1c8 = Some(listed);
        } else {
            cx.world.get_mut::<TesNpc>(npc).expect("an NPC").hair_1c8 = None;
            let bald = text_at_or_empty(cx.world, GMST_BALD);
            let tile = page_child(cx, menu, GMST_MAIN, GMST_HAIR)?;
            tile_set_string_trait(cx, tile, user(6), &bald)?;
        }
    }
    race_sex_menu_update_hair_color_toggles(cx, menu)
}

/// IDA: 005C9770 (thiscall menu), the sex change: Race's Gender user6 against sMale (`_stricmp`; a null side differs): equal
/// keeps user6 sMale and sets the NPC's female flag (+0x28 bit 0), anything else writes sFemale and clears it, each then
/// TESActorBaseData vt+0x50 (004672F0: the form's vt+0x40(0x10), `record_form_change`); 005C4920, 005C7070(1) (the head
/// dropped), 005C2F20 (a new head) and 005C7BA0(0). Completion: 100%.
/// # Errors
/// A null pane child, or a stop in the calls.
pub fn race_sex_menu_change_gender(cx: &mut InterfaceCx, menu: MenuId) -> Result<(), GameError> {
    let (_, npc, _, _) = player_npc(cx)?;
    let tile = page_child(cx, menu, GMST_RACE, GMST_GENDER)?;
    let label = crate::tes::tile_values::tile_string_trait(cx, tile, user(6));
    let male = text_at_or_empty(cx.world, GMST_MALE);
    let equal = label
        .as_deref()
        .is_some_and(|label| !male.is_empty() && oblivion_core::crt::stricmp(label, &male) == 0);
    if equal {
        tile_set_string_trait(cx, tile, user(6), &male)?;
        cx.world
            .get_mut::<crate::tes::form_components::TesActorBaseData>(npc)
            .expect("an NPC has its data")
            .flags_04 |= 1;
    } else {
        let female = text_at_or_empty(cx.world, GMST_FEMALE);
        tile_set_string_trait(cx, tile, user(6), &female)?;
        cx.world
            .get_mut::<crate::tes::form_components::TesActorBaseData>(npc)
            .expect("an NPC has its data")
            .flags_04 &= !1;
    }
    crate::tes::forms_init::record_form_change(cx.world, npc, 0x10)?;
    race_sex_menu_apply_gender_hair(cx, menu)?;
    race_sex_menu_update_face(cx, true)?;
    race_sex_menu_rebuild_head(cx)?;
    race_sex_menu_update_pages(cx, menu, false).map(|_| ())
}

// IDA: N/A the player's FaceGen node by name under its 3D (TESObjectREFR vt+0x130 BSFaceGenNiNodeBiped, vt+0x134
// BSFaceGenNiNodeSkinned: NiObjectNET_GetObjectByNameWithPalette on +0x3C).
fn player_face_gen_node(world: &World, player: Entity, name: &[u8]) -> Option<Entity> {
    world
        .get::<crate::tes::reference_3d::Reference3D>(player)
        .map(|loaded| loaded.0)
        .and_then(|node| {
            crate::gamebryo::object_net::object_by_name_with_palette(world, node, name)
        })
}

// IDA: N/A the player's animation data's object palette (TESObjectREFR_GetAnimData +0x98, its +0x7C).
fn player_object_palette(world: &World, player: Entity) -> Option<Entity> {
    crate::tes::actor_movement_update::player_anim_data(world, player)
        .and_then(|anim| world.get::<crate::tes::actor_animation::ActorAnimData>(anim))
        .and_then(|data| data.manager_98)
        .and_then(|manager| {
            world.get::<crate::gamebryo::controller_sequence::ControllerManager>(manager)
        })
        .and_then(|manager| manager.object_palette_7c)
}

/// IDA: 005C3E10 (thiscall menu), the eyes set from +0x87C: none without the BSFaceGenNiNodeBiped node (vt+0x130). The
/// race's eyes list (+0xA8) walked from a count of -1: each playable eye (`0051ED80`) counts, and the node where the count
/// meets +0x87C is the eye; a null eye there answers 0. With none met, the list's first eye, or none (0). The eye's name
/// (+0x1C) into Main's Eyes user6, the NPC's +0x1D0 = the eye. Then the biped node's children named "FaceGenEyeLeft" and
/// "FaceGenEyeRight" (`strcmp`, the last of each kept): with both, "Textures\%s" of the eye's texture (+0x28), or
/// "Textures\Characters\Eyes\EyeDefault.dds" with none, through TES_CreateTextureImage (`00442890`, which makes a texture even
/// for a missing file) into a new NiTexturingProperty (clamp 3, filter 2) that replaces each eye's type-6 property; then each
/// eye found by name (vt+0x58) takes 007B8940(eye, 1, 1, 1). Answers 1. Completion: 100%, with the texturing property a
/// host deferral (D13): the path is the eye's `FaceGenPartTexture`, as the head build keeps it.
/// # Errors
/// A null pane child, no model services, or a stop in the shader preparation.
fn race_sex_menu_update_eyes(cx: &mut InterfaceCx, menu: MenuId) -> Result<bool, GameError> {
    let (player, npc, race, _) = player_npc(cx)?;
    if player_face_gen_node(cx.world, player, b"BSFaceGenNiNodeBiped").is_none() {
        return Ok(false);
    }
    let list = cx
        .world
        .get::<TesRace>(race)
        .expect("a race")
        .eyes_a8
        .clone();
    let index = race_sex(cx, menu).eyes_index_87c;
    // 005C3E7B..005C3EBF: the inline list's head node always exists; an empty list is that node with a null item.
    let nodes = list.len().max(1);
    let mut counted: i32 = -1;
    let mut at = Some(0_usize);
    while let Some(node) = at {
        if counted == index {
            break;
        }
        if list
            .get(node)
            .is_some_and(|&eyes| eyes_playable(cx.world, eyes))
        {
            counted += 1;
        }
        if counted != index {
            at = (node + 1 < nodes).then_some(node + 1);
        }
    }
    let chosen = match at {
        Some(node) => list.get(node).copied(),
        None => list.first().copied(),
    };
    let Some(chosen) = chosen else {
        return Ok(false);
    };
    let name = crate::tes::reference_3d::base_full_name(cx.world, chosen);
    let tile = page_child(cx, menu, GMST_MAIN, GMST_EYES)?;
    tile_set_string_trait(cx, tile, user(6), &name)?;
    cx.world.get_mut::<TesNpc>(npc).expect("an NPC").eyes_1d0 = Some(chosen);
    let InterfaceCx { world, runtime, .. } = cx;
    runtime.with_model_services(world, &mut |world, services| {
        let biped = player_face_gen_node(world, player, b"BSFaceGenNiNodeBiped");
        let children: Vec<Entity> = biped
            .and_then(|node| world.get::<crate::gamebryo::scene_node::NiNodeChildren>(node))
            .map(|children| children.slots_ac.iter().flatten().copied().collect())
            .unwrap_or_default();
        let named = |world: &World, name: &[u8]| {
            children.iter().copied().rfind(|&child| {
                world
                    .get::<crate::gamebryo::object_net::NiObjectNet>(child)
                    .and_then(|net| net.name.as_deref())
                    .is_some_and(|own| own == name)
            })
        };
        if let (Some(left), Some(right)) = (
            named(world, b"FaceGenEyeLeft"),
            named(world, b"FaceGenEyeRight"),
        ) {
            let path = world
                .get::<crate::tes::data_handler::TesTexture>(chosen)
                .map(|texture| {
                    let mut path = b"Textures\\".to_vec();
                    path.extend_from_slice(&crate::tes::face_gen_head::path_text(&texture.path_04));
                    path
                })
                .unwrap_or_else(|| b"Textures\\Characters\\Eyes\\EyeDefault.dds".to_vec());
            for eye in [left, right] {
                world
                    .entity_mut(eye)
                    .insert(crate::tes::face_gen_nodes::FaceGenPartTexture(path.clone()));
            }
            *world
                .get_resource_or_init::<RaceSexMenuDeferrals>()
                .calls
                .entry(0x005C_418E)
                .or_default() += 1;
        }
        for name in [&b"FaceGenEyeLeft"[..], b"FaceGenEyeRight"] {
            let eye =
                biped.and_then(|node| crate::bs::model_loader::object_by_name(world, node, name));
            if eye.is_some() {
                crate::bs::sky_shader::prepare_object_geometry(
                    world,
                    eye,
                    1,
                    1,
                    1,
                    &mut services.prepare,
                )?;
            }
        }
        Ok(())
    })?;
    Ok(true)
}

/// IDA: 005C91E0 (cdecl), the player's FaceGen head made again: the BSFaceGenNiNodeSkinned (vt+0x134) then the
/// BSFaceGenNiNodeBiped (vt+0x130) node, each with a parent, leaves the animation data's palette (`00716620`) and its parent
/// (vt+0x88 DetachChild, the pointer released); the process's vt+0x5C (+0x15C = 0); `00405CE0` (the NPC's +0x1D4 and +0x1D8
/// released); `00525A70` TESNPC_CreateHead(npc, player, vt+0x168's skin info) with the driver's model services; a face
/// description (`0052CD50`) on the stack, which both `005551C0` calls take (`005C938C` on vt+0x130's node, `005C93AC` on
/// vt+0x134's; run-as: the new hair's Lighting30ShaderProperty holds the NPC's colour / 255.0 after the step, stored at
/// `00555898` from `005C9391`); then `005C2F20`. Completion: 100%, with `005551C0`'s hair arm ported (`apply_hair_tint`) and
/// its face texture and eye arms counted in `RaceSexMenuDeferrals` (D13).
/// # Errors
/// No model services, or a stop in the calls.
fn race_sex_menu_recreate_head(cx: &mut InterfaceCx) -> Result<(), GameError> {
    let (player, npc, _, _) = player_npc(cx)?;
    let palette = player_object_palette(cx.world, player);
    for name in [&b"BSFaceGenNiNodeSkinned"[..], b"BSFaceGenNiNodeBiped"] {
        let Some(node) = player_face_gen_node(cx.world, player, name) else {
            continue;
        };
        let parent = cx
            .world
            .get::<crate::gamebryo::av_object::NiAvParent>(node)
            .and_then(|parent| parent.0);
        if let Some(parent) = parent {
            crate::gamebryo::av_object_palette::remove_scene_objects(cx.world, Some(node), palette);
            crate::gamebryo::scene_node::detach_child(cx.world, parent, node);
        }
    }
    let process = crate::tes::actor::process_of(cx.world, player).ok_or(
        GameError::unclassified(0x005C931F, "005C931F: the player with no process faults"),
    )?;
    cx.world
        .get_mut::<crate::tes::actor_process::MiddleHighProcess>(process)
        .ok_or(GameError::unclassified(
            0x005C9327,
            "005C9327: the player's process with no MiddleHigh part",
        ))?
        .face_animation_data_15c = None;
    cx.world
        .entity_mut(npc)
        .insert(crate::tes::face_gen_nodes::FaceGenHeadNodes::default());
    let skin = crate::tes::equipment_3d::view_skin_info(cx.world, player);
    let InterfaceCx { world, runtime, .. } = cx;
    runtime.with_model_services(world, &mut |world, services| {
        crate::tes::face_gen_nodes::create_head(world, npc, player, skin, services)
    })?;
    let settings = cx
        .world
        .get_resource::<crate::tes::face_gen_tasks::FaceGenHeadSettings>()
        .copied()
        .unwrap_or_default();
    let description = crate::tes::face_gen_head::fill_face_description(
        cx.world,
        npc,
        settings.face_texturing_b10d3c,
        settings.skip_lod_head_b120b0,
    )?;
    for (name, site) in [
        (&b"BSFaceGenNiNodeBiped"[..], 0x005C_938C),
        (b"BSFaceGenNiNodeSkinned", 0x005C_93AC),
    ] {
        if let Some(node) = player_face_gen_node(cx.world, player, name) {
            apply_face_gen_textures(cx, &[node], &description)?;
        }
        *cx.world
            .get_resource_or_init::<RaceSexMenuDeferrals>()
            .calls
            .entry(site)
            .or_default() += 1;
    }
    race_sex_menu_rebuild_head(cx)
}

/// IDA: GMST sNPCModelBeast (00B38B68, "Characters\\_Male\\SkeletonBeast.NIF") and sNPCModel (00B38B70,
/// "Characters\\_Male\\Skeleton.NIF"): the skeleton for a race with a tail and without.
const GMST_NPC_MODEL_BEAST: u32 = 0x00B3_8B68;
const GMST_NPC_MODEL: u32 = 0x00B3_8B70;

/// IDA: GMST sNoRace (00B38B78, "NO RACE").
const GMST_NO_RACE: u32 = 0x00B3_8B78;

/// IDA: 005CDA30 (thiscall menu), the race's change: Race's Race user0 (`ftol`) as the index among DataHandler +0x44's
/// playable races (+0x70 bit 0), walked from -1. Past the list: Race's Race user6 = sNoRace and the Race Description's
/// " " (00A36430); a null race there: the Race Description's sNoRace. A race R: the tail (0052BDB0(race, 0)) of the NPC's
/// race before; R's name (+0x1C) into Main's Race and Race's Race; `00662710` (each spell of the old race's +0x30 list through
/// the player's vt+0x2E0, `player_remove_spell`); `006A2190`(0) and `005E4B00` on the player's magic target (each acts on the
/// active-effect list, empty in the race menu: any effect is a stop); the NPC's race (+0xE8) = R; R's description (vt+0x10(0,
/// DESC), D23: read recorded, the tile takes ""); R's tail; the process's vt+0x384 (`0064B0B0`, the idle pick). Then 005C9980(0),
/// 005C6EA0, 005C7070(1) (the head dropped), and with the tail unchanged 005C2F20 (a new head). A changed tail reloads the
/// player's 3D: the old 3D's parent, `player_reload::player_set_3d_null` (004E0F80), the NPC's skeleton (+0xAC vt+0x18:
/// sNPCModelBeast for a tail, else sNPCModel), 00B33D80 around `player_reload::character_load_3d` (00659F30) and the parent's
/// vt+0x84(node, 1), "Player", 00B3BB10 its Camera01, vt+0x148 (`0065B750`), 005C2F20 and vt+0x384. Then 005C7BA0(0).
/// Completion: 100% of the arms the race menu's player reaches (all ten playable races).
/// # Errors
/// A null pane child, an active effect, or a stop in the calls.
pub fn race_sex_menu_change_race(cx: &mut InterfaceCx, menu: MenuId) -> Result<(), GameError> {
    let (player, npc, old_race, _) = player_npc(cx)?;
    let index = oblivion_core::x87::ftol(page_number(cx, menu, GMST_RACE, GMST_RACE, user(0))?);
    let races = cx
        .world
        .get_resource::<crate::tes::data_handler::DataHandlerLists>()
        .map(|lists| lists.races_44.items().collect::<Vec<_>>())
        .unwrap_or_default();
    let playable = |world: &World, race: Entity| {
        world
            .get::<TesRace>(race)
            .is_some_and(|data| data.data_50[0x20] & 1 != 0)
    };
    let nodes = races.len().max(1);
    let mut counted: i32 = -1;
    let mut at = Some(0_usize);
    while let Some(node) = at {
        if counted == index {
            break;
        }
        if races
            .get(node)
            .is_some_and(|&race| playable(cx.world, race))
        {
            counted += 1;
        }
        if counted != index {
            at = (node + 1 < nodes).then_some(node + 1);
        }
    }
    let mut tail_before = false;
    let mut tail_after = false;
    match at.map(|node| races.get(node).copied()) {
        None => {
            let none = text_at_or_empty(cx.world, GMST_NO_RACE);
            let tile = page_child(cx, menu, GMST_RACE, GMST_RACE)?;
            tile_set_string_trait(cx, tile, user(6), &none)?;
            let tile = page_child(cx, menu, GMST_RACE, GMST_RACE_DESCRIPTION)?;
            tile_set_string_trait(cx, tile, user(6), b" ")?;
        }
        Some(None) => {
            let none = text_at_or_empty(cx.world, GMST_NO_RACE);
            let tile = page_child(cx, menu, GMST_RACE, GMST_RACE_DESCRIPTION)?;
            tile_set_string_trait(cx, tile, user(6), &none)?;
        }
        Some(Some(race)) => {
            tail_before =
                crate::tes::character_3d::race_tail_model(cx.world, old_race, 0).is_some();
            let name = crate::tes::reference_3d::base_full_name(cx.world, race);
            let tile = page_child(cx, menu, GMST_MAIN, GMST_RACE)?;
            tile_set_string_trait(cx, tile, user(6), &name)?;
            let tile = page_child(cx, menu, GMST_RACE, GMST_RACE)?;
            tile_set_string_trait(cx, tile, user(6), &name)?;
            // 00662710: the old race's spells through vt+0x2E0.
            let spells = cx
                .world
                .get::<crate::tes::form_components::TesSpellList>(old_race)
                .map(|list| list.spells_04.clone())
                .unwrap_or_default();
            for spell in spells {
                let crate::tes::form_components::FormRef::Resolved(Some(spell)) = spell else {
                    break;
                };
                crate::tes::script_commands::player_remove_spell(cx.world, player, spell)?;
            }
            // 006A2190(0) and 005E4B00: both act on the player's active effects.
            let effects = crate::tes::actor::process_of(cx.world, player)
                .and_then(|process| {
                    cx.world
                        .get::<crate::tes::magic_cast::ActiveEffectList>(process)
                })
                .is_some_and(|list| !list.0.is_empty());
            if effects {
                return Err(GameError::unclassified(
                    0x005CDD45,
                    "005CDD45: 006A2190 and 005E4B00 with active effects to dispel",
                ));
            }
            cx.world
                .get_mut::<crate::tes::form_components::TesRaceForm>(npc)
                .ok_or(GameError::unclassified(
                    0x005CDD59,
                    "005CDD59: an NPC with no race form",
                ))?
                .race_04 = crate::tes::form_components::FormRef::Resolved(Some(race));
            cx.world
                .get_resource_or_init::<RaceDescriptionReads>()
                .races
                .push(race);
            let tile = page_child(cx, menu, GMST_RACE, GMST_RACE_DESCRIPTION)?;
            tile_set_string_trait(cx, tile, user(6), b"")?;
            tail_after = crate::tes::character_3d::race_tail_model(cx.world, race, 0).is_some();
            let answers = menu_input_answers(cx);
            crate::tes::idle_select::process_play_idle(cx.world, player, answers)?;
        }
    }
    race_sex_menu_change_eyes(cx, menu, false)?;
    race_sex_menu_set_initial_defaults(cx, menu)?;
    race_sex_menu_update_face(cx, true)?;
    if tail_after == tail_before {
        race_sex_menu_rebuild_head(cx)?;
        return race_sex_menu_update_pages(cx, menu, false).map(|_| ());
    }
    // 005CDBFA..005CDEAE: the player's 3D dropped and loaded again with the skeleton for the new tail.
    let skeleton = text_at_or_empty(
        cx.world,
        if tail_after {
            GMST_NPC_MODEL_BEAST
        } else {
            GMST_NPC_MODEL
        },
    );
    let InterfaceCx { world, runtime, .. } = cx;
    runtime.with_model_services(world, &mut |world, services| {
        let parent = crate::tes::reference_3d::player_set_3d_null(world, player)?.ok_or(
            GameError::unclassified(
                0x005CDE6B,
                "005CDE6B: a null parent of the player's 3D faults",
            ),
        )?;
        // 005CDC10 the NPC's TESModel (+0xAC) vt+0x18 SetModelPath.
        world
            .get_mut::<crate::tes::form_components::TesModel>(npc)
            .ok_or(GameError::unclassified(
                0x005CDC20,
                "005CDC20: an NPC with no TESModel",
            ))?
            .path_04
            .set(Some(&skeleton), 0);
        world
            .get_resource_or_init::<crate::tes::biped_attach::EquipmentRebuildFlag>()
            .active_b33d80 = true;
        let loaded = crate::tes::player_reload::character_load_3d(world, player, services);
        let node = match loaded {
            Ok(node) => node,
            Err(error) => {
                world
                    .resource_mut::<crate::tes::biped_attach::EquipmentRebuildFlag>()
                    .active_b33d80 = false;
                return Err(error);
            }
        }
        .ok_or(GameError::unclassified(
            0x005CDE6B,
            "005CDE6B: Character_Load3D answered null",
        ))?;
        let attached = crate::bs::scene_graphs::attach_scene_child(world, parent, Some(node), true);
        world
            .resource_mut::<crate::tes::biped_attach::EquipmentRebuildFlag>()
            .active_b33d80 = false;
        if attached != crate::bs::scene_graphs::AttachChildStatus::Attached {
            return Err(GameError::unclassified(
                0x005CDE79,
                "005CDE79: attaching the player's new 3D",
            ));
        }
        world
            .get_mut::<crate::gamebryo::object_net::NiObjectNet>(node)
            .ok_or(GameError::unclassified(
                0x005CDE88,
                "005CDE88: a 3D with no NiObjectNET",
            ))?
            .name = Some(b"Player".to_vec());
        let camera = crate::bs::model_loader::object_by_name(world, node, b"Camera01");
        world
            .get_resource_or_init::<crate::tes::player_3d::FirstPersonNodes>()
            .third_person_camera_b3bb10 = camera;
        // 005CDEAE vt+0x148 0065B750.
        crate::tes::character_controller::actor_create_character_controller(
            world,
            player,
            crate::tes::character_controller::ControllerInputs::default(),
        )
    })?;
    race_sex_menu_rebuild_head(cx)?;
    let answers = menu_input_answers(cx);
    crate::tes::idle_select::process_play_idle(cx.world, player, answers)?;
    race_sex_menu_update_pages(cx, menu, false).map(|_| ())
}

/// IDA: 005C5B00 (thiscall menu), the hair style's change: 005C50A0(1); when it answers 1, Hair's Style user0 against +0x870 (as
/// a double) takes 005C2B50 with +0x870. Completion: 100%.
/// # Errors
/// A null pane child, or a stop in 005C50A0.
pub fn race_sex_menu_change_hair_style(
    cx: &mut InterfaceCx,
    menu: MenuId,
) -> Result<(), GameError> {
    if !race_sex_menu_update_hair(cx, menu, true)? {
        return Ok(());
    }
    let tile = page_child(cx, menu, GMST_HAIR, GMST_STYLE)?;
    let index = race_sex(cx, menu).hair_index_870;
    if f64::from(tile_numeric_trait(cx, tile, user(0))) != f64::from(index) {
        toggle_set_value(cx, tile, index as f32)?;
    }
    Ok(())
}

/// IDA: 005C9980 (thiscall menu, rebuild): 005C3E10; when it answers 1, Main's Eyes user0 against +0x87C (as a double) takes
/// 005C2B50 with +0x87C, and `rebuild` runs 005C91E0. Completion: 100%.
/// # Errors
/// A null pane child, or a stop in the calls.
pub fn race_sex_menu_change_eyes(
    cx: &mut InterfaceCx,
    menu: MenuId,
    rebuild: bool,
) -> Result<(), GameError> {
    if !race_sex_menu_update_eyes(cx, menu)? {
        return Ok(());
    }
    let tile = page_child(cx, menu, GMST_MAIN, GMST_EYES)?;
    let index = race_sex(cx, menu).eyes_index_87c;
    if f64::from(tile_numeric_trait(cx, tile, user(0))) != f64::from(index) {
        toggle_set_value(cx, tile, index as f32)?;
    }
    if rebuild {
        race_sex_menu_recreate_head(cx)?;
    }
    Ok(())
}

/// IDA: 005C62D0 (thiscall menu): Hair's Length user0 (4014) into the NPC's +0x1CC hair length, then 005C50A0(0). Completion:
/// 100%.
/// # Errors
/// A null pane child, or a stop in 005C50A0.
pub fn race_sex_menu_store_hair_length(
    cx: &mut InterfaceCx,
    menu: MenuId,
) -> Result<bool, GameError> {
    let length = page_number(cx, menu, GMST_HAIR, GMST_LENGTH, user(0))?;
    let (_, npc, _, _) = player_npc(cx)?;
    cx.world
        .get_mut::<TesNpc>(npc)
        .expect("an NPC")
        .hair_length_1cc = length;
    race_sex_menu_update_hair(cx, menu, false)
}

/// IDA: 005CDEF0's colour packing (005CE04B..005CE0DD): each slider's user0 times 255.0 under the main thread's 24-bit
/// precision (a single rounding to f32), truncated by `fistp` under round-to-zero, and its low byte: blue << 16 | green << 8
/// | red. Completion: 100%.
#[must_use]
pub fn pack_slider_color(red: f32, green: f32, blue: f32) -> u32 {
    let byte = |value: f32| (oblivion_core::x87::ftol(value * 255.0) as u32) & 0xFF;
    byte(blue) << 16 | byte(green) << 8 | byte(red)
}

// IDA: N/A the NPC's own coefficients written back as 00552990(difference, own, own) leaves them: +0x168 with vampirism
// (vt+0x128(69)), else +0x108 (00521A10's choice).
fn store_own_coefficients(
    cx: &mut InterfaceCx,
    npc: Entity,
    updated: [FaceGenMatrix; 4],
) -> Result<(), GameError> {
    let vampire = crate::tes::actor_values::base_actor_value(cx.world, npc, 69)? != 0;
    let mut data = cx.world.get_mut::<TesNpc>(npc).expect("an NPC");
    if vampire {
        data.facegen_168 = updated;
    } else {
        data.facegen_108 = updated;
    }
    Ok(())
}

// IDA: N/A the slider range 005C6390 and 005C6AF0 share: General(Face)'s pane (sGeneralFace, 005C3110) whose id trait equals
// the menu tile's (+0x04) user0 takes the race's +0xA4 (3.0 when not above 0.0), any other page its +0xA0 (5.0 likewise).
fn slider_range(cx: &InterfaceCx, menu: MenuId, race: Entity) -> f32 {
    let name = text_at_or_empty(cx.world, GMST_GENERAL_FACE);
    let pane = pane_by_name(cx, menu, &name);
    let page = cx
        .engine
        .menu(menu)
        .tile_04
        .map_or(0.0, |tile| tile_numeric_trait(cx, tile, user(0)));
    let id = pane.map_or(0.0, |pane| {
        tile_numeric_trait(cx, pane, crate::tes::tile_values::TRAIT_ID)
    });
    let data = cx.world.get::<TesRace>(race).expect("a race");
    if f64::from(id) == f64::from(page) {
        if data.facegen_face_a4 <= 0.0 {
            3.0
        } else {
            data.facegen_face_a4
        }
    } else if data.facegen_main_a0 <= 0.0 {
        5.0
    } else {
        data.facegen_main_a0
    }
}

// IDA: N/A a control tile's index into the linear controls: `fistp` of user7 - 1.0 under round-to-zero, its low dword.
fn control_index(cx: &InterfaceCx, tile: TileId) -> u32 {
    let value = f64::from(tile_numeric_trait(cx, tile, user(7))) - 1.0;
    (value.trunc() as i64) as u32
}

// IDA: N/A the menu's +0x94 control tile at `index` (a missing one is a null tile the original reads through: a stop).
fn control_tile(cx: &InterfaceCx, menu: MenuId, index: i32) -> Result<TileId, GameError> {
    race_sex(cx, menu)
        .tiles_94
        .get(&index)
        .copied()
        .ok_or(GameError::unclassified(
            0x005C64E0,
            "005C64E0: a null +0x94 control tile faults",
        ))
}

/// IDA: 005C6390 (thiscall menu, index), a face slider's change: the coefficients (00552990 of the race's +0x29C and
/// 00521A10's); the range (`slider_range`); the control tile +0x94 + 4 x index: with ftol(user8) the value is
/// (user0 - 0.0) / (1.0 - 0.0) x 20.0 - 10.0, else (r - -r) x ((user0 - 0.0) / (1.0 - 0.0)) + -r (24-bit x87, f32 each
/// step), stored as a float; `005539E0`(coefficients, ftol(user8), 0, `fistp`(user7 - 1.0), value); 00552C10 against
/// 005221C0's coefficients and 00552990 into the NPC's own. With user8 0.0, the biped (vt+0x130) then the skinned
/// (vt+0x134) node's geometry children: FaceGenHair takes 00550980 with the hair length, the part's model (00550790)
/// 00558840(model, difference, part, 1.0, null), and FaceGenHair 00550980 again (as 005C7070's loop). Then the control
/// tile's user10 = its user0. Completion: 100%.
/// # Errors
/// A null control tile, a null skinned node, or a stop in the controls or the morph.
fn race_sex_menu_apply_slider(
    cx: &mut InterfaceCx,
    menu: MenuId,
    index: i32,
) -> Result<(), GameError> {
    let (player, npc, race, _) = player_npc(cx)?;
    let race_coefficients = cx
        .world
        .get::<TesRace>(race)
        .expect("a race")
        .facegen_29c
        .clone();
    let own = npc_own_coefficients(cx.world, npc)?;
    let mut coefficients: [FaceGenMatrix; 4] = Default::default();
    combine_coefficients(&race_coefficients, &own, &mut coefficients)?;
    let range = slider_range(cx, menu, race);
    let tile = control_tile(cx, menu, index)?;
    let texture = oblivion_core::x87::ftol(tile_numeric_trait(cx, tile, user(8)));
    let position = tile_numeric_trait(cx, tile, user(0));
    let value = if texture != 0 {
        (position - 0.0) / (1.0 - 0.0) * 20.0 - 10.0
    } else {
        let low = -range;
        (range - low) * ((position - 0.0) / (1.0 - 0.0)) + low
    };
    let control = control_index(cx, tile);
    let kind = usize::try_from(2 * texture).map_err(|_| {
        GameError::unclassified(0x005C65C0, "005C65C0: a negative linear control kind")
    })?;
    crate::tes::face_gen_head::set_linear_control(
        cx.world,
        &mut coefficients,
        kind,
        control,
        value,
    )?;
    let current = crate::tes::face_gen_head::npc_face_gen_coefficients(cx.world, npc)?;
    let mut difference: [FaceGenMatrix; 4] = Default::default();
    crate::tes::face_gen_head::coefficients_difference(&current, &coefficients, &mut difference);
    let own = npc_own_coefficients(cx.world, npc)?;
    let mut updated: [FaceGenMatrix; 4] = Default::default();
    combine_coefficients(&difference, &own, &mut updated)?;
    store_own_coefficients(cx, npc, updated)?;
    if tile_numeric_trait(cx, tile, user(8)) == 0.0
        && let Some(biped) = player_face_node(cx, player, b"BSFaceGenNiNodeBiped")
    {
        let skinned = player_face_node(cx, player, b"BSFaceGenNiNodeSkinned").ok_or(
            GameError::unclassified(0x005C6694, "005C6694: a null skinned FaceGen node faults"),
        )?;
        let length = cx.world.get::<TesNpc>(npc).expect("an NPC").hair_length_1cc;
        for node in [biped, skinned] {
            for child in child_slots(cx.world, node).into_iter().flatten() {
                let Some(geometry) = as_geometry(cx.world, child) else {
                    continue;
                };
                let hair = object_name(cx.world, geometry) == b"FaceGenHair";
                if hair {
                    crate::tes::face_gen_nodes::apply_hair_length(cx.world, geometry, length);
                }
                if apply_part_morph(cx, geometry, &difference)? && hair {
                    crate::tes::face_gen_nodes::apply_hair_length(cx.world, geometry, length);
                }
            }
        }
    }
    let position = tile_numeric_trait(cx, tile, user(0));
    tile_set_numeric_trait(cx, tile, user(10), position)
}

/// IDA: 005C7800 (cdecl), the face texture's controls after a texture slider: the RaceSexMenu and both FaceGen nodes, else
/// return; the process's vt+0x5C (+0x15C = 0); the coefficients (race +0x29C and 00521A10's); d = 0.0 - 0.0; the texture age
/// control (00553B30(0, 1)) less d, clamped to [15, 65], set back (00555A00(0, 1)); the texture gender control (1, 1) less the
/// sign (00522230) less d, clamped to [-2, 2], set as sign + it (f32 each step); 00552C10 against 005221C0's and 00552990
/// into the NPC's own; a face description (`0052CD50`) into `005551C0` on vt+0x130's then vt+0x134's node: their hair arm
/// (`apply_hair_tint`), the face texture and eye arms counted (D13). Completion: 100% but D13.
/// # Errors
/// A stop in the controls or the description.
fn race_sex_menu_update_face_texture(cx: &mut InterfaceCx) -> Result<(), GameError> {
    if cx
        .engine
        .tile_menu_by_id(MENU_ID_RACE_SEX)
        .and_then(|tile| cx.engine.containing_menu(tile))
        .is_none()
    {
        return Ok(());
    }
    let (player, npc, race, sex) = player_npc(cx)?;
    let (Some(biped), Some(skinned)) = (
        player_face_node(cx, player, b"BSFaceGenNiNodeBiped"),
        player_face_node(cx, player, b"BSFaceGenNiNodeSkinned"),
    ) else {
        return Ok(());
    };
    if let Some(process) = crate::tes::actor::process_of(cx.world, player)
        && let Some(mut middle) = cx
            .world
            .get_mut::<crate::tes::actor_process::MiddleHighProcess>(process)
    {
        middle.face_animation_data_15c = None;
    }
    let race_coefficients = cx
        .world
        .get::<TesRace>(race)
        .expect("a race")
        .facegen_29c
        .clone();
    let own = npc_own_coefficients(cx.world, npc)?;
    let mut coefficients: [FaceGenMatrix; 4] = Default::default();
    combine_coefficients(&race_coefficients, &own, &mut coefficients)?;
    let control = crate::tes::face_gen_head::face_gen_age_gender_control;
    let set = crate::tes::face_gen_head::face_gen_set_age_gender_control;
    let clamp = |value: f32, low: f32, high: f32| {
        if value > low && value >= high {
            high
        } else if value <= low {
            low
        } else {
            value
        }
    };
    #[allow(clippy::eq_op, reason = "005C7911: fldz, then fsubr st, st")]
    let zero = 0.0_f32 - 0.0;
    let age = control(cx.world, &coefficients, 0, 1)? - zero;
    set(cx.world, &mut coefficients, 0, 1, clamp(age, 15.0, 65.0))?;
    // 00522230: 2.0, or -2.0 for a male.
    let sign = if sex != 0 { 2.0_f32 } else { -2.0 };
    let gender = control(cx.world, &coefficients, 1, 1)? - sign;
    let gender = clamp(gender - zero, -2.0, 2.0);
    set(cx.world, &mut coefficients, 1, 1, sign + gender)?;
    let mut difference: [FaceGenMatrix; 4] = Default::default();
    crate::tes::face_gen_head::shape_coefficients(&mut difference);
    let current = crate::tes::face_gen_head::npc_face_gen_coefficients(cx.world, npc)?;
    crate::tes::face_gen_head::coefficients_difference(&current, &coefficients, &mut difference);
    let own = npc_own_coefficients(cx.world, npc)?;
    let mut updated: [FaceGenMatrix; 4] = Default::default();
    combine_coefficients(&difference, &own, &mut updated)?;
    store_own_coefficients(cx, npc, updated)?;
    let settings = cx
        .world
        .get_resource::<crate::tes::face_gen_tasks::FaceGenHeadSettings>()
        .copied()
        .unwrap_or_default();
    let description = crate::tes::face_gen_head::fill_face_description(
        cx.world,
        npc,
        settings.face_texturing_b10d3c,
        settings.skip_lod_head_b120b0,
    )?;
    for (node, site) in [(biped, 0x005C_7AE1), (skinned, 0x005C_7B04)] {
        apply_face_gen_textures(cx, &[node], &description)?;
        *cx.world
            .get_resource_or_init::<RaceSexMenuDeferrals>()
            .calls
            .entry(site)
            .or_default() += 1;
    }
    Ok(())
}

/// IDA: 005C9650 RaceSexMenu_ShowPage(allocation, a3, source, a5, sync) (thiscall menu, `retn 14h`): the pane named by
/// `source` (005C3110); Done (+0x3C) user5 = 2.0 (flt_A379B4); the menu tile (+0x04) user0 = the pane's id (0xFA8) and user1
/// = its child count (0xFD0); with `sync`, 005C6AF0(the pane, null); the scroll bar (+0x34) user9 = -999999.0 (flt_A6906C)
/// then 0.0; both strings freed. The page buttons' click arms in 005CC200 (goal 5) call it; a test or host capture can show
/// a pane with it. Completion: 100% of the stores and their order; the two FormHeap frees have no host form (the name is a
/// borrowed slice).
/// # Errors
/// A null pane, menu tile, Done or scroll bar (the original faults), or as `005C6AF0`.
pub fn race_sex_menu_show_page(
    cx: &mut InterfaceCx,
    menu: MenuId,
    pane: &[u8],
    sync: bool,
) -> Result<(), GameError> {
    // 005C967B..005C96A1: the BSStringT copy and 005C3110.
    let pane = pane_by_name(cx, menu, pane).ok_or(GameError::unclassified(
        0x005C96C9,
        "005C96C9: a null pane faults",
    ))?;
    // 005C96A6..005C96BA: Done's user5 = 2.0.
    let done = race_sex(cx, menu).done_3c.ok_or(GameError::unclassified(
        0x005C96BA,
        "005C96BA: a null +0x3C tile faults",
    ))?;
    tile_set_numeric_trait(cx, done, user(5), 2.0)?;
    // 005C96BF..005C96F8: the menu tile's user0 = the pane's id, user1 = its child count.
    let tile = cx.engine.menu(menu).tile_04.ok_or(GameError::unclassified(
        0x005C96D9,
        "005C96D9: a null menu tile faults",
    ))?;
    let id = tile_numeric_trait(cx, pane, crate::tes::tile_values::TRAIT_ID);
    tile_set_numeric_trait(cx, tile, user(0), id)?;
    let count = tile_numeric_trait(cx, pane, TRAIT_CHILD_COUNT);
    tile_set_numeric_trait(cx, tile, user(1), count)?;
    // 005C96FD..005C9709.
    if sync {
        race_sex_menu_sync_pane_sliders(cx, menu, pane, None)?;
    }
    // 005C970E..005C9733: the scroll bar's user9 = -999999.0, then 0.0.
    let bar = race_sex(cx, menu)
        .scroll_bar_34
        .ok_or(GameError::unclassified(
            0x005C9720,
            "005C9720: a null +0x34 tile faults",
        ))?;
    tile_set_numeric_trait(cx, bar, user(9), -999_999.0)?;
    tile_set_numeric_trait(cx, bar, user(9), 0.0)
}

/// IDA: 005C6AF0 (thiscall menu, pane, changed): the coefficients (race +0x29C and 00521A10's); each tile of the pane's list
/// other than `changed`: the range (`slider_range`), its control tile +0x94 + 4 x ftol(user6); with ftol(the control's user8)
/// the value `005538F0`(coefficients, ftol(user8), 0, `fistp`(user7 - 1.0)) maps to (1.0 - 0.0) x ((v - -10.0) / 20.0) + 0.0,
/// else (1.0 - 0.0) x ((v - -r) / (r - -r)) + 0.0; user3 = (float)0x80000000, the value, 0.0; user10 = user0. Completion:
/// 100%.
/// # Errors
/// A null control tile, or a stop in the controls.
fn race_sex_menu_sync_pane_sliders(
    cx: &mut InterfaceCx,
    menu: MenuId,
    pane: TileId,
    changed: Option<TileId>,
) -> Result<(), GameError> {
    let (_, npc, race, _) = player_npc(cx)?;
    let race_coefficients = cx
        .world
        .get::<TesRace>(race)
        .expect("a race")
        .facegen_29c
        .clone();
    let own = npc_own_coefficients(cx.world, npc)?;
    let mut coefficients: [FaceGenMatrix; 4] = Default::default();
    combine_coefficients(&race_coefficients, &own, &mut coefficients)?;
    let children = cx.engine.tile(pane).children_30.clone();
    for child in children {
        if Some(child) == changed {
            continue;
        }
        let range = slider_range(cx, menu, race);
        let index = oblivion_core::x87::ftol(tile_numeric_trait(cx, child, user(6)));
        let control = control_tile(cx, menu, index)?;
        let texture = oblivion_core::x87::ftol(tile_numeric_trait(cx, control, user(8)));
        let kind = usize::try_from(2 * texture).map_err(|_| {
            GameError::unclassified(0x005C6CF7, "005C6CF7: a negative linear control kind")
        })?;
        let value = crate::tes::face_gen_head::linear_control_value(
            cx.world,
            &coefficients,
            kind,
            control_index(cx, control),
        )?;
        let position = if texture != 0 {
            (1.0 - 0.0) * ((value - -10.0) / 20.0) + 0.0
        } else {
            let low = -range;
            (1.0 - 0.0) * ((value - low) / (range - low)) + 0.0
        };
        set_user3(cx, child, position)?;
        let now = tile_numeric_trait(cx, child, user(0));
        tile_set_numeric_trait(cx, child, user(10), now)?;
    }
    Ok(())
}

// IDA: 005C9AB0 RaceSexMenu_ApplyPaneSliderChanges(pane) (thiscall menu): the pane's list in order; each tile with user20 1.0
// and user10 other than user0 takes 005C6390(ftol(user6)) and becomes the last changed, and a user8 of 1.0 marks a texture
// slider. With one marked, 005C7800 and 005C2F20; with any change, 005C6AF0(pane, the last) and 1; else 0.
fn apply_pane_slider_changes(
    cx: &mut InterfaceCx,
    menu: MenuId,
    pane: Option<TileId>,
) -> Result<bool, GameError> {
    let Some(pane) = pane else {
        return Err(GameError::unclassified(
            0x005C9ABA,
            "005C9ABA: a null pane's list faults",
        ));
    };
    let mut changed = None;
    let mut texture = false;
    let children = cx.engine.tile(pane).children_30.clone();
    for child in children {
        if tile_numeric_trait(cx, child, user(20)) == 1.0
            && tile_numeric_trait(cx, child, user(10)) != tile_numeric_trait(cx, child, user(0))
        {
            let index = oblivion_core::x87::ftol(tile_numeric_trait(cx, child, user(6)));
            race_sex_menu_apply_slider(cx, menu, index)?;
            changed = Some(child);
            if tile_numeric_trait(cx, child, user(8)) == 1.0 {
                texture = true;
            }
        }
    }
    if texture {
        race_sex_menu_update_face_texture(cx)?;
        race_sex_menu_rebuild_head(cx)?;
    }
    let Some(changed) = changed else {
        return Ok(false);
    };
    race_sex_menu_sync_pane_sliders(cx, menu, pane, Some(changed))?;
    Ok(true)
}

/// IDA: 005CDEF0 RaceSexMenu_ApplyPageChanges (thiscall menu), each frame from 005CE570: the tiles' values (user0, 4014)
/// against what the menu and the NPC hold, in this order:
/// - Hair's Red, Green and Blue packed (`pack_slider_color`) against the NPC's +0x1E8: 005C5C30(1);
/// - Hair's Length against the NPC's +0x1CC (`fucompp`, unordered differs): 005C62D0;
/// - Hair Color against `fild` +0x88C: 005C5F00, then +0x88C = ftol(its user0);
/// - Race's Race, ftol, against +0x86C: 005CDA30, then the store;
/// - Race's Gender, ftol, against the byte +0x868: 005C9770, then +0x868 = (the value > 0);
/// - Main's Age or Complexion against +0x880 / +0x884: both stored, 005C50A0(0), 005C7070(1) (the head dropped), 005C2F20 (a new head);
/// - Main's Eyes, ftol, against +0x87C: stored first, then 005C9980(1);
/// - Hair's Style, ftol, against +0x870: stored first, then 005C5B00;
/// - 005C9BA0: the sliders of the Main pane (+0x40) and of +0x58..+0x90, each through 005C9AB0.
///
/// Completion: 100% of the order, the compares, the stores and the arms (2026-11-09); the stops left are inside the arms'
/// callees (open-items).
/// # Errors
/// A null pane child, or a stop in an arm's callee.
pub fn race_sex_menu_apply_page_changes(
    cx: &mut InterfaceCx,
    menu: MenuId,
) -> Result<(), GameError> {
    let (_, npc, _, _) = player_npc(cx)?;
    let red = page_number(cx, menu, GMST_HAIR, GMST_RED, user(0))?;
    let green = page_number(cx, menu, GMST_HAIR, GMST_GREEN, user(0))?;
    let blue = page_number(cx, menu, GMST_HAIR, GMST_BLUE, user(0))?;
    let stored = cx.world.get::<TesNpc>(npc).expect("an NPC").hair_color_1e8;
    if pack_slider_color(red, green, blue) != stored {
        race_sex_menu_store_hair_color(cx, menu, true)?;
    }
    let length = cx.world.get::<TesNpc>(npc).expect("an NPC").hair_length_1cc;
    if page_number(cx, menu, GMST_HAIR, GMST_LENGTH, user(0))? != length {
        race_sex_menu_store_hair_length(cx, menu)?;
    }
    let preset = page_number(cx, menu, GMST_HAIR, GMST_HAIR_COLOR, user(0))?;
    if preset != race_sex(cx, menu).hair_color_index_88c as f32 {
        race_sex_menu_apply_hair_color_preset(cx, menu)?;
        let preset = page_number(cx, menu, GMST_HAIR, GMST_HAIR_COLOR, user(0))?;
        race_sex_mut(cx, menu).hair_color_index_88c = oblivion_core::x87::ftol(preset);
    }
    let race = oblivion_core::x87::ftol(page_number(cx, menu, GMST_RACE, GMST_RACE, user(0))?);
    if race != race_sex(cx, menu).race_index_86c {
        race_sex_menu_change_race(cx, menu)?;
        race_sex_mut(cx, menu).race_index_86c = race;
    }
    let female = oblivion_core::x87::ftol(page_number(cx, menu, GMST_RACE, GMST_GENDER, user(0))?);
    if female != i32::from(race_sex(cx, menu).female_868) {
        race_sex_menu_change_gender(cx, menu)?;
        race_sex_mut(cx, menu).female_868 = u8::from(female > 0);
    }
    let age = page_number(cx, menu, GMST_MAIN, GMST_AGE, user(0))?;
    let complexion = page_number(cx, menu, GMST_MAIN, GMST_COMPLEXION, user(0))?;
    let state = race_sex(cx, menu);
    if age != state.age_880 || complexion != state.complexion_884 {
        let state = race_sex_mut(cx, menu);
        state.age_880 = age;
        state.complexion_884 = complexion;
        race_sex_menu_update_hair(cx, menu, false)?;
        race_sex_menu_update_face(cx, true)?;
        race_sex_menu_rebuild_head(cx)?;
    }
    let eyes = oblivion_core::x87::ftol(page_number(cx, menu, GMST_MAIN, GMST_EYES, user(0))?);
    if eyes != race_sex(cx, menu).eyes_index_87c {
        race_sex_mut(cx, menu).eyes_index_87c = eyes;
        race_sex_menu_change_eyes(cx, menu, true)?;
    }
    let style = oblivion_core::x87::ftol(page_number(cx, menu, GMST_HAIR, GMST_STYLE, user(0))?);
    if style != race_sex(cx, menu).hair_index_870 {
        race_sex_mut(cx, menu).hair_index_870 = style;
        race_sex_menu_change_hair_style(cx, menu)?;
    }
    // 005C9BA0: +0x40 (Main) and +0x58..+0x90, the 15 FaceGen sub-panes.
    let panes = race_sex(cx, menu).panes_40;
    for index in std::iter::once(0).chain(6..21) {
        apply_pane_slider_changes(cx, menu, panes[index])?;
    }
    Ok(())
}

/// IDA: 005CA010 RaceSexMenu_Create. An open one answers its tile. Otherwise Menu_GetNextDepth, the XML under the menu root,
/// its menu (none answers null); a null g_thePlayer destroys a menu with a tile and answers null. The player's worn hair
/// (slot 1) else head (slot 0) item at +0x84C, unequipped (a stop: the new game's player wears none); 0065D660 (00B3BB04 =
/// 0, 00B14E4D = 1); Menu_RegisterTile; no +0x28, +0x2C, +0x30 or +0x3C warns and answers null; stacking 6006 or 102 takes
/// the depth. 00552990 into +0x8D8; the 16 hair colours (iHairColorNN) and names ("%s" of sHairColorN), interleaved; the
/// 139 entries; the root's user0 the Main pane's id and user1 its childcount; the name tile's string: the player's name when
/// the setup isn't pending and the name is neither 00B3B4D0's nor "Bendu Olo", else the name is copied to 00B3B4D0 and
/// sEnterName shown; its xdefault 2.0. Then 005C7BA0(1), with the setup pending 005C6EA0 and the flag cleared (the other
/// two arms are stops), the hair colour's user13 = 2.0 arm (the sliders' colour matched against the 16), 005C7070(0), the
/// root visible 1.0 with a
/// MessageMenu shown or Menu_Enable(0), +0x894 = +0x588, 0066C580(0), 005C2BF0(1.0, 0.0) (the camera at the face), the
/// process's vt+0x384 (the idle pick, 11 draws) and +0x890 = g_timeInfo +0x10. Answers the tile. Completion: 100% of the
/// order.
/// # Errors
/// The stops above, or in a callee.
#[allow(clippy::too_many_lines)] // 005CA010's own length.
pub fn race_sex_menu_create(cx: &mut InterfaceCx) -> Result<Option<TileId>, GameError> {
    if let Some(open) = cx.engine.tile_menu_by_id(MENU_ID_RACE_SEX) {
        return Ok(Some(open));
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, RACE_SEX_XML)?;
    let Some(created) = created else {
        return Ok(None);
    };
    let Some(menu) = cx.engine.containing_menu(created) else {
        return Ok(None);
    };
    let Some(player) = the_player(cx.world) else {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    };
    if !matches!(cx.engine.menu(menu).class, MenuClass::RaceSex(_)) {
        return Err(GameError::unclassified(
            0x005CA07C,
            "005CA07C: race_sex_menu.xml made another class",
        ));
    }
    let hair = crate::tes::equipment::equipped_in_slot(cx.world, player, 1)?;
    let head = crate::tes::equipment::equipped_in_slot(cx.world, player, 0)?;
    let unequipped = hair.or(head).map(|(form, _)| form);
    race_sex_mut(cx, menu).unequipped_84c = unequipped;
    if unequipped.is_some() {
        return Err(GameError::unclassified(
            0x005CA0F6,
            "005CA0F6: Actor_UnequipItem on the player's hair or head item",
        ));
    }
    // 0065D660.
    cx.world.resource_mut::<RaceSexMenuStatics>().vanity_b3bb04 = 0;
    cx.world
        .get_resource_or_init::<crate::tes::player::PlayerGlobals>()
        .byte_b14e4d = 1;
    menu_register_tile(cx, menu, Some(created))?;
    {
        let data = race_sex(cx, menu);
        if data.window_28.is_none()
            || data.player_grab_2c.is_none()
            || data.name_30.is_none()
            || data.done_3c.is_none()
        {
            cx.host.warning(
                "Race/Sex Menu Creation Failed... Are your menu and art resources up to date?",
            );
            return Ok(None);
        }
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    // 005CA1B1..005CA1D9: 00552990(race +0x29C, 00521A10(npc), +0x8D8, 0, 0.0).
    let npc = cx
        .world
        .get::<crate::tes::actor::TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
        .ok_or(GameError::unclassified(
            0x005CA1B1,
            "005CA1B1: the player with no base form",
        ))?;
    let race = crate::tes::forms_actors::base_race(cx.world, npc)?.ok_or(
        GameError::unclassified(0x005CA1C3, "005CA1C3: a null race faults"),
    )?;
    let vampire = crate::tes::actor_values::base_actor_value(cx.world, npc, 69)? != 0;
    let race_coefficients = cx
        .world
        .get::<TesRace>(race)
        .expect("a race")
        .facegen_29c
        .clone();
    let own = {
        let data = cx.world.get::<TesNpc>(npc).expect("an NPC");
        if vampire {
            data.facegen_168.clone()
        } else {
            data.facegen_108.clone()
        }
    };
    let mut coefficients: [FaceGenMatrix; 4] = Default::default();
    combine_coefficients(&race_coefficients, &own, &mut coefficients)?;
    race_sex_mut(cx, menu).coefficients_8d8 = coefficients;
    for index in 0..16 {
        let color = cx
            .world
            .get_resource::<GameSettingCollection>()
            .and_then(|settings| settings.raw_at(GMST_HAIR_COLORS + 8 * index as u32))
            .unwrap_or_default();
        let name = text_at_or_empty(cx.world, GMST_HAIR_COLOR_NAMES + 8 * index as u32);
        let data = race_sex_mut(cx, menu);
        data.hair_colors_8f0[index] = color;
        data.hair_color_names_930[index] = name;
    }
    build_entries(cx, menu)?;
    let main = pane_by_name(cx, menu, &text_at_or_empty(cx.world, GMST_MAIN));
    let main_id = main.map_or(0.0, |tile| tile_numeric_trait(cx, tile, TRAIT_ID));
    tile_set_numeric_trait(cx, created, user(0), main_id)?;
    let main_children = main.map_or(0.0, |tile| tile_numeric_trait(cx, tile, TRAIT_CHILD_COUNT));
    tile_set_numeric_trait(cx, created, user(1), main_children)?;
    let full_name = crate::tes::reference_3d::base_full_name(cx.world, npc);
    let pending = cx
        .world
        .resource::<RaceSexMenuStatics>()
        .initial_setup_pending_b14500;
    let remembered = cx
        .world
        .resource::<RaceSexMenuStatics>()
        .name_b3b4d0
        .clone();
    let shown = if pending == 0 && full_name != remembered && full_name != b"Bendu Olo" {
        full_name
    } else {
        cx.world.resource_mut::<RaceSexMenuStatics>().name_b3b4d0 = full_name;
        text_at_or_empty(cx.world, GMST_ENTER_NAME)
    };
    let name_tile = race_sex(cx, menu).name_30.expect("checked");
    tile_set_string_trait(cx, name_tile, TRAIT_STRING, &shown)?;
    tile_set_numeric_trait(cx, name_tile, TRAIT_XDEFAULT, 2.0)?;
    race_sex_menu_update_pages(cx, menu, true)?;
    if pending != 0 {
        race_sex_menu_set_initial_defaults(cx, menu)?;
        cx.world
            .resource_mut::<RaceSexMenuStatics>()
            .initial_setup_pending_b14500 = 0;
    } else {
        return Err(GameError::unclassified(
            0x005CBCB0,
            "005CBCB0: the race menu reopened (00521A10's hair colour arms)",
        ));
    }
    // 005CBDC9: the "Hair Color" toggle's user13, which 005C34D0 (in 005C6EA0) set.
    let hair = text_at_or_empty(cx.world, 0x00B3_8F88);
    let color = text_at_or_empty(cx.world, 0x00B3_9330);
    if let Some(toggle) = pane_child(cx, menu, &hair, &color)?
        && tile_numeric_trait(cx, toggle, user(13)) == 2.0
    {
        // 005CBDEF..005CBF1F: the Red, Green and Blue sliders' user0, each ftol(x * 255.0), packed red | green << 8 |
        // blue << 16; a hair colour equal to it gives the toggle 005C2B50(index): user3 = -user0, the index, 0.0.
        let channel = |cx: &InterfaceCx, name: u32| -> Result<u32, GameError> {
            let name = text_at_or_empty(cx.world, name);
            let slider = pane_child(cx, menu, &hair, &name)?;
            let value = slider.map_or(0.0, |tile| tile_numeric_trait(cx, tile, user(0)));
            Ok(oblivion_core::x87::ftol(value * 255.0) as u32)
        };
        let red = channel(cx, 0x00B3_8FC0)?;
        let green = channel(cx, 0x00B3_8FC8)?;
        let blue = channel(cx, 0x00B3_8FD0)?;
        let packed = red | (((blue & 0xFF) << 8 | (green & 0xFF)) << 8);
        for index in 0..16 {
            if packed == race_sex(cx, menu).hair_colors_8f0[index] {
                let own = -tile_numeric_trait(cx, toggle, user(0));
                tile_set_numeric_trait(cx, toggle, user(3), own)?;
                tile_set_numeric_trait(cx, toggle, user(3), index as f32)?;
                tile_set_numeric_trait(cx, toggle, user(3), 0.0)?;
            }
        }
    }
    race_sex_menu_update_face(cx, false)?;
    if crate::tes::interface_manager::menu_is_visible(cx, 1001) {
        tile_set_numeric_trait(cx, created, TRAIT_VISIBLE, 1.0)?;
    } else {
        let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
        menu_enable(cx, menu, false, frame_time)?;
    }
    let third_person = cx
        .world
        .get::<crate::tes::player::PlayerCharacter>(player)
        .expect("the player")
        .view_flags_588[0];
    race_sex_mut(cx, menu).third_person_894 = third_person;
    let vanity = cx.world.resource::<RaceSexMenuStatics>().vanity_b3bb04;
    crate::tes::player_view::player_set_first_person(cx.world, player, false, vanity)?;
    race_sex_menu_update_camera(cx, menu, 0.0)?;
    let answers = menu_input_answers(cx);
    crate::tes::idle_select::process_play_idle(cx.world, player, answers)?;
    Ok(Some(created))
}

/// IDA: 00B46120 and 00B46124, which 0057B7E0 zeroes.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct NewGameRaceMenuGlobals {
    pub float_b46120: f32,
    pub float_b46124: f32,
}

/// IDA: N/A 0057C140's inputs for the idle walk's conditions, read from the interface (`MenuInputAnswers`).
fn menu_input_answers(cx: &InterfaceCx) -> crate::tes::idle_select::MenuInputAnswers {
    let Some(manager) = cx.world.get_resource::<InterfaceManager>() else {
        return crate::tes::idle_select::MenuInputAnswers::default();
    };
    // 0057C140's head (the manager, cursor, root and the root's user0 at 2.0) and 0057CF60's top stacked mode.
    let ready = crate::tes::interface_player_scene::interface_game_is_running(cx);
    let top_mode = crate::tes::interface_frame::top_stacked_menu_mode(&manager.menu_mode_stack_0e0);
    let shown = |id: u32| (id, crate::tes::interface_manager::menu_is_visible(cx, id));
    crate::tes::idle_select::MenuInputAnswers {
        ready,
        top_mode,
        big_four_shown: [shown(0x3EB), shown(0x3EA), shown(0x3FE), shown(0x3FF)],
    }
}

/// IDA: the static NiCamera 005C2BF0 builds at 00B3B5E0 (00B3B444 + 0x19C) on its first run (guard bit 0 of 00B3B704,
/// NiCamera_Constructor and its atexit `00A24E60`): its local translate (+0x54, 00B3B634) and the rotation `0070C340`
/// stores (+0x30, 00B3B610). It has no parent, so its world is its local. NiCamera_Constructor leaves +0x30 the identity,
/// which a refused `0070C340` keeps and 005C2BF0 then permutes into the scene camera.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct RaceSexCameraStatics {
    pub constructed_b3b704: bool,
    pub translate_b3b634: [f32; 3],
    pub rotate_b3b610: [f32; 9],
}

impl Default for RaceSexCameraStatics {
    /// IDA: N/A the constructor's state: identity rotation, zero translate.
    fn default() -> Self {
        Self {
            constructed_b3b704: false,
            translate_b3b634: [0.0; 3],
            rotate_b3b610: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// IDA: 005C2BF0 RaceSexMenu_UpdateCamera(menu, unused, angle_degrees), `retn 8`, RaceSexMenu_Create's 005CC18F with
/// (1.0, 0.0): only with 00B3B5D8 clear. +0x8A4 = (0, 30, 0) and the look offset (11, 0, 0) (flt_A6BC94, flt_A37CC8); the
/// player's vt+0x134(0) (004DDFD0 TESObjectREFR_GetFaceGenNiNodeSkinned: an actor (vt+0x190) whose vt+0x27C answers
/// nonzero, the player's 2, looks up "BSFaceGenNiNodeSkinned" under +0x3C through 006F94A0) answers null and it returns;
/// else the node's world bound centre (+0x20..+0x28). R = 0070FDD0(GetZRotation (vt+0x1E0, 0065DA60) + (angle + 0.0) x
/// 0.01745329238474369, a float); +0x8A4 = centre + R x (0, 30, 0) and look = centre + R x (11, 0, 0) (007101F0, each sum a
/// float). `0066A5E0`, `0065F080`(player, position +0x8A4, look, 1); the static NiCamera (`RaceSexCameraStatics`) takes the
/// position as its local translate and NiAVObject_Update(0.0, 1) (no parent: its world translate is the position), then
/// `0070C340`(look, g_niPoint3UnitZ (00B258E8)). The World SceneGraph's first child takes the position (+0x54) and the
/// camera's rotation permuted [r2, r0, r1, r5, r3, r4, r8, r6, r7] (+0x30, rep movsd), then NiAVObject_Update(0.0, 0).
/// Precision: 24-bit per step. Completion: 100%, with the callees' stops and `005326B0`'s ray substitution.
/// TODO: [ ] The FaceGen node's world bound is the port's update's; 005C7070's morph (before this) moves vertices by float
/// steps and no NiAVObject_Update of the skinned node follows it here, as in the original.
/// # Errors
/// A stop in a callee, or a World SceneGraph with no first child (a fault in the original).
pub fn race_sex_menu_update_camera(
    cx: &mut InterfaceCx,
    menu: MenuId,
    angle_degrees: f32,
) -> Result<(), GameError> {
    use crate::gamebryo::transform::{camera_look_at_rotation, rotate_vector};
    if cx.world.resource::<RaceSexMenuStatics>().camera_byte_b3b5d8 != 0 {
        return Ok(());
    }
    let offset = [0.0_f32, 30.0, 0.0];
    let look_offset = [11.0_f32, 0.0, 0.0];
    race_sex_mut(cx, menu).camera_position_8a4 = offset;
    let Some(player) = the_player(cx.world) else {
        return Err(GameError::unclassified(
            0x005C2C65,
            "005C2C65: a null g_thePlayer faults",
        ));
    };
    let graph = cx
        .world
        .get_resource::<crate::bs::scene_graphs::SceneGraphGlobals>()
        .and_then(|globals| globals.world);
    let face = cx
        .world
        .get::<crate::tes::reference_3d::Reference3D>(player)
        .and_then(|loaded| {
            crate::gamebryo::object_net::object_by_name_with_palette(
                cx.world,
                loaded.0,
                b"BSFaceGenNiNodeSkinned",
            )
        });
    let Some(face) = face else {
        return Ok(());
    };
    let bound = cx
        .world
        .get::<crate::gamebryo::scene_node::NiAvWorld>(face)
        .map_or([0.0; 4], |state| state.bound);
    let centre = [bound[0], bound[1], bound[2]];
    let heading = crate::tes::player_move::player_heading(cx.world, player);
    let angle = heading + ((f64::from(angle_degrees) + 0.0) * 0.017_453_292_384_743_69) as f32;
    let rotation = crate::gamebryo::transform::rotation_z(angle);
    let turned = rotate_vector(&rotation, &offset);
    let mut position = [
        centre[0] + turned[0],
        turned[1] + centre[1],
        turned[2] + centre[2],
    ];
    let turned = rotate_vector(&rotation, &look_offset);
    let look = [
        turned[0] + centre[0],
        turned[1] + centre[1],
        turned[2] + centre[2],
    ];
    crate::tes::player_view::player_sync_camera_collision_world(cx.world, player)?;
    let race_sex_open = cx.engine.tile_menu_by_id(MENU_ID_RACE_SEX).is_some();
    crate::tes::player_view::player_resolve_camera_collision(
        cx.world,
        player,
        &mut position,
        &look,
        true,
        race_sex_open,
    )?;
    race_sex_mut(cx, menu).camera_position_8a4 = position;
    let rotate = {
        let mut statics = cx.world.get_resource_or_init::<RaceSexCameraStatics>();
        statics.constructed_b3b704 = true;
        statics.translate_b3b634 = position;
        if let Some(rotate) = camera_look_at_rotation(&position, &look, &[0.0, 0.0, 1.0]) {
            statics.rotate_b3b610 = rotate;
        }
        statics.rotate_b3b610
    };
    let permuted = [
        rotate[2], rotate[0], rotate[1], rotate[5], rotate[3], rotate[4], rotate[8], rotate[6],
        rotate[7],
    ];
    let graph = graph.ok_or(GameError::unclassified(
        0x005C2C73,
        "005C2C73: a null g_worldSceneGraph faults",
    ))?;
    let child = cx
        .world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(graph)
        .and_then(|children| children.slots_ac.first().copied().flatten())
        .ok_or(GameError::unclassified(
            0x005C2EA8,
            "005C2EA8: a World SceneGraph with no first child faults",
        ))?;
    {
        let mut payload = cx
            .world
            .get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(child)
            .ok_or(GameError::unclassified(
                0x005C2EA8,
                "005C2EA8: a first child other than a NiNode",
            ))?;
        payload.local_transform.translation = position;
        payload.local_transform.rotation = permuted;
    }
    let mut runtime = crate::gamebryo::scene_update::NoSceneControllers;
    crate::gamebryo::scene_update::update_scene_object(cx.world, child, 0.0, false, &mut runtime)?;
    Ok(())
}

// IDA: the GMSTs 005CC200 compares beyond the panes' names: sFace (00B38F80), sShape, sTone, sRandom, sReset and the Shape and
// Tone pages' sub-panes.
const GMST_FACE: u32 = 0x00B3_8F80;
const GMST_SHAPE: u32 = 0x00B3_8FE0;
const GMST_TONE: u32 = 0x00B3_8FE8;
const GMST_RANDOM: u32 = 0x00B3_8FF0;
const GMST_RESET: u32 = 0x00B3_8FF8;
/// IDA: 005CC2D6..005CC65D, 005CD1E2..005CD612: the Shape page's sub-panes in 005CC200's order (sBrow twice in the back
/// arm, once in the page arms).
const SHAPE_PANES: [u32; 9] = [
    0x00B3_9000,
    0x00B3_9008,
    0x00B3_9010,
    0x00B3_9018,
    0x00B3_8F90,
    0x00B3_9020,
    0x00B3_9028,
    0x00B3_9030,
    0x00B3_9038,
];
/// IDA: 005CC65D..005CC7CC, 005CD698..005CD936: the Tone page's sub-panes.
const TONE_PANES: [u32; 6] = [
    0x00B3_9040,
    0x00B3_9050,
    0x00B3_9320,
    0x00B3_9058,
    0x00B3_9060,
    0x00B3_9328,
];
/// IDA: 0xFF0 (4080), the trait 005CC200 sets to 1.0 on the name tile (+0x30) first.
const TRAIT_FF0: u16 = 0x0FF0;

// IDA: N/A a pane's numeric trait by its GMST name (005C3110, then Tile_GetNumericTraitOrZero); a null pane faults.
fn pane_trait(cx: &InterfaceCx, menu: MenuId, pane: u32, trait_id: u16) -> Result<f32, GameError> {
    let name = text_at_or_empty(cx.world, pane);
    let tile = pane_by_name(cx, menu, &name).ok_or(GameError::unclassified(
        0x005C3110,
        "005C3110: a null pane faults",
    ))?;
    Ok(tile_numeric_trait(cx, tile, trait_id))
}

// IDA: N/A the id of a pane's child (005C3440 then Tile_GetNumericTraitOrZero's id), 0.0 for a missing child.
fn child_id(cx: &InterfaceCx, menu: MenuId, pane: u32, child: u32) -> Result<f32, GameError> {
    let (pane, child) = (
        text_at_or_empty(cx.world, pane),
        text_at_or_empty(cx.world, child),
    );
    Ok(pane_child(cx, menu, &pane, &child)?.map_or(0.0, |tile| {
        tile_numeric_trait(cx, tile, crate::tes::tile_values::TRAIT_ID)
    }))
}

// IDA: N/A 005CC200's back arm: the menu tile's user0 and user1 = the pane's id and child count (0xFD0).
fn race_sex_menu_back_to(cx: &mut InterfaceCx, menu: MenuId, pane: u32) -> Result<(), GameError> {
    let tile = cx.engine.menu(menu).tile_04.ok_or(GameError::unclassified(
        0x005CC849,
        "005CC849: a null menu tile faults",
    ))?;
    let id = pane_trait(cx, menu, pane, crate::tes::tile_values::TRAIT_ID)?;
    tile_set_numeric_trait(cx, tile, user(0), id)?;
    let count = pane_trait(cx, menu, pane, TRAIT_CHILD_COUNT)?;
    tile_set_numeric_trait(cx, tile, user(1), count)
}

/// IDA: 005CC200 RaceSexMenu vt+0x0C HandleClick(id, tile) (thiscall, `retn 8`): +0x8D0 = 0 and the name tile's (+0x30) trait
/// 0xFF0 = 1.0; an enabled name field (+0x8EC) ends (`005C30C0`) and the click goes on to the id-90 test; id 10 starts it
/// (`005C2730`, `005C30C0`) and goes on to the page chain. **Id 90 (Done / back):** on the Main page (the menu tile's user0 =
/// sMain's id) Done's name checks (`005CCA77`); on Shape or Tone back to Face; on a Shape sub-pane back to Shape, on a Tone
/// sub-pane back to Tone; else Done's user5 = 1.0 and back to Main: the menu tile's user0 and user1 = the pane's id and child
/// count. **Other ids,** each against the named child's id: Face/Random and Face/Reset (a confirm box the first time,
/// 00B3B4C8 and 00B3B4C9), id 98 (`00526F00`, `005C91E0`), Main/Age (`005C7070`(0), `005C2F20`), Hair/Red, Green and Blue
/// (`005C5C30`(1)), Hair/Length (`005C62D0`), Race/Gender (`005C9770`), then the pages through `005C9650`: Main's Face, Hair
/// and Race without the sync, Face's Shape and Tone, Shape's and Tone's sub-panes with it. No match returns. After an arm:
/// 00B3B4C9 = 0, 00B3B4C8 = 0 unless the Random arm ran, and `007C5E70`/`007C7EB0` on the player's node (counted). A
/// rejected name returns before the tail; so does Reset, which clears only 00B3B4C8. Completion: 100% of the dispatch;
/// Done's close (`005C28F0 RaceSexMenu_AcceptCharacter`) is Build 5 Target B.
/// # Errors
/// A null pane or tile (the original faults), or a stop in an arm.
pub fn race_sex_menu_handle_click(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    _tile: Option<TileId>,
) -> Result<(), GameError> {
    let name = race_sex(cx, menu).name_30.ok_or(GameError::unclassified(
        0x005CC22E,
        "005CC22E: a null +0x30 tile faults",
    ))?;
    race_sex_mut(cx, menu).pause_8d0 = 0;
    tile_set_numeric_trait(cx, name, TRAIT_FF0, 1.0)?;
    let page_chain = id != 90;
    if race_sex(cx, menu).name_input_8ec.enabled_00 != 0 {
        // 005CC263: the field ends, and its text is stored.
        race_sex_mut(cx, menu).name_input_8ec.set_enabled(false);
        race_sex_menu_store_name(cx, menu)?;
    } else if id == 10 {
        // 005CC8AC: the name tile starts the field; 005CC8BA goes on to the page chain.
        race_sex_menu_begin_name(cx, menu)?;
        race_sex_menu_store_name(cx, menu)?;
    }
    if !page_chain {
        let current = cx
            .engine
            .menu(menu)
            .tile_04
            .map(|tile| tile_numeric_trait(cx, tile, user(0)))
            .ok_or(GameError::unclassified(
                0x005CC2AD,
                "005CC2AD: a null menu tile faults",
            ))?;
        let is = |cx: &InterfaceCx, pane: u32| -> Result<bool, GameError> {
            Ok(pane_trait(cx, menu, pane, crate::tes::tile_values::TRAIT_ID)? == current)
        };
        if is(cx, GMST_MAIN)? {
            // 005CCA77: Done. A rejected name shows its box and returns before the tail.
            if !race_sex_menu_done(cx, menu)? {
                return Ok(());
            }
        } else {
            let parent = if is(cx, GMST_SHAPE)? || is(cx, GMST_TONE)? {
                GMST_FACE
            } else if SHAPE_PANES
                .iter()
                .map(|&pane| is(cx, pane))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .any(|matched| matched)
            {
                GMST_SHAPE
            } else if TONE_PANES
                .iter()
                .map(|&pane| is(cx, pane))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .any(|matched| matched)
            {
                GMST_TONE
            } else {
                let done = race_sex(cx, menu).done_3c.ok_or(GameError::unclassified(
                    0x005CC816,
                    "005CC816: a null +0x3C tile faults",
                ))?;
                tile_set_numeric_trait(cx, done, user(5), 1.0)?;
                GMST_MAIN
            };
            race_sex_menu_back_to(cx, menu, parent)?;
        }
    }
    let mut keep_random = false;
    if page_chain {
        let clicked = id as f32;
        let matches = |cx: &InterfaceCx, pane: u32, child: u32| -> Result<bool, GameError> {
            Ok(child_id(cx, menu, pane, child)? == clicked)
        };
        if matches(cx, GMST_FACE, GMST_RANDOM)? {
            // 005CC93C: 00B3B4C8 set (a Random confirmed before) randomizes at once (005CCBE5); else the confirm box
            // (dword_B39450's text, 005C9FC0, base 1, the buttons [sNo, sYes]) and 00B3B4C8 = 1. Either way the tail keeps
            // 00B3B4C8 ([esp+13h] = 1).
            let confirmed = cx
                .world
                .get_resource_or_init::<RaceSexMenuStatics>()
                .word_b3b4c8
                & 0x00FF
                != 0;
            if confirmed {
                race_sex_menu_random(cx)?;
            } else {
                let text = text_at_or_empty(cx.world, 0x00B3_9450);
                let buttons = [
                    text_at_or_empty(cx.world, 0x00B3_8D00),
                    text_at_or_empty(cx.world, 0x00B3_8CF8),
                ];
                crate::tes::menus::show_message(
                    cx,
                    &text,
                    Some(crate::tes::menus::MessageCallback::RaceSexRandom),
                    1,
                    &buttons,
                )?;
                let mut statics = cx.world.get_resource_or_init::<RaceSexMenuStatics>();
                statics.word_b3b4c8 = (statics.word_b3b4c8 & 0xFF00) | 1;
            }
            keep_random = true;
        } else if id == 98 {
            // 005CCBF9: 00526F00 on the player's base, then 005C91E0; the tail clears both bytes.
            let (_, npc, _, _) = player_npc(cx)?;
            npc_undo_face(cx.world, npc)?;
            race_sex_menu_recreate_head(cx)?;
        } else if matches(cx, GMST_FACE, GMST_RESET)? {
            // 005CCC90: 00B3B4C9 set (a Reset confirmed by the last click) resets at once; else the confirm box
            // (dword_B39448's text, 005C9F70, base 1, the buttons [sNo, sYes]) and 00B3B4C9 = 1. Either way 005CD9D3: only
            // 00B3B4C8 is cleared.
            let confirmed = cx
                .world
                .get_resource_or_init::<RaceSexMenuStatics>()
                .word_b3b4c8
                & 0xFF00
                != 0;
            if confirmed {
                race_sex_menu_reset(cx)?;
            } else {
                let text = text_at_or_empty(cx.world, 0x00B3_9448);
                let buttons = [
                    text_at_or_empty(cx.world, 0x00B3_8D00),
                    text_at_or_empty(cx.world, 0x00B3_8CF8),
                ];
                crate::tes::menus::show_message(
                    cx,
                    &text,
                    Some(crate::tes::menus::MessageCallback::RaceSexReset),
                    1,
                    &buttons,
                )?;
                cx.world
                    .get_resource_or_init::<RaceSexMenuStatics>()
                    .word_b3b4c8 |= 0x0100;
            }
            cx.world
                .get_resource_or_init::<RaceSexMenuStatics>()
                .word_b3b4c8 &= 0xFF00;
            crate::tes::equipment_3d::count(cx.world, 0x007C_5E70);
            crate::tes::equipment_3d::count(cx.world, 0x007C_7EB0);
            return Ok(());
        } else if matches(cx, GMST_MAIN, GMST_AGE)? {
            race_sex_menu_update_face(cx, false)?;
            race_sex_menu_rebuild_head(cx)?;
        } else if matches(cx, GMST_HAIR, GMST_RED)?
            || matches(cx, GMST_HAIR, GMST_GREEN)?
            || matches(cx, GMST_HAIR, GMST_BLUE)?
        {
            race_sex_menu_store_hair_color(cx, menu, true)?;
        } else if matches(cx, GMST_HAIR, GMST_LENGTH)? {
            race_sex_menu_store_hair_length(cx, menu)?;
        } else if matches(cx, GMST_RACE, GMST_GENDER)? {
            race_sex_menu_change_gender(cx, menu)?;
        } else {
            // 005CCF37..005CD993: the page buttons, (parent, page, sync) in order.
            let mut pages: Vec<(u32, u32, bool)> = vec![
                (GMST_MAIN, GMST_FACE, false),
                (GMST_FACE, GMST_SHAPE, true),
                (GMST_FACE, GMST_TONE, true),
                (GMST_MAIN, GMST_HAIR, false),
                (GMST_MAIN, GMST_RACE, false),
            ];
            pages.extend(SHAPE_PANES.iter().map(|&page| (GMST_SHAPE, page, true)));
            pages.extend(TONE_PANES.iter().map(|&page| (GMST_TONE, page, true)));
            let mut shown = false;
            for (parent, page, sync) in pages {
                if matches(cx, parent, page)? {
                    let page = text_at_or_empty(cx.world, page);
                    race_sex_menu_show_page(cx, menu, &page, sync)?;
                    shown = true;
                    break;
                }
            }
            if !shown {
                // 005CD936 jp 005CDA11: no match returns before the tail.
                return Ok(());
            }
        }
    }
    // 005CD9C5..005CDA0C: the confirm boxes' bytes, then the player's node through 007C5E70 and 007C7EB0.
    // 00B3B4C9 (the word's high byte) = 0; 00B3B4C8 = 0 unless the Random arm ran.
    let mut statics = cx.world.get_resource_or_init::<RaceSexMenuStatics>();
    statics.word_b3b4c8 = if keep_random {
        statics.word_b3b4c8 & 0x00FF
    } else {
        0
    };
    crate::tes::equipment_3d::count(cx.world, 0x007C_5E70);
    crate::tes::equipment_3d::count(cx.world, 0x007C_7EB0);
    Ok(())
}

/// IDA: 005C2730 RaceSexMenu_BeginNameInput(menu): on +0x8EC, 0057FF20("") the text, 0057D2D0(__ftol2 of the name tile's
/// font, trait 0xFD3), 00583DD0(__ftol2 of its wrapwidth, 0xFD4) and 0057DD90(1). Completion: 100%.
/// # Errors
/// A null name tile.
pub fn race_sex_menu_begin_name(cx: &mut InterfaceCx, menu: MenuId) -> Result<(), GameError> {
    let name = race_sex(cx, menu).name_30.ok_or(GameError::unclassified(
        0x005C2743,
        "005C2743: a null +0x30 tile faults",
    ))?;
    let font = oblivion_core::x87::ftol(tile_numeric_trait(
        cx,
        name,
        crate::tes::tile_values::TRAIT_FONT,
    ));
    let width = oblivion_core::x87::ftol(tile_numeric_trait(
        cx,
        name,
        crate::tes::tile_values::TRAIT_WRAP_WIDTH,
    ));
    let virtual_width = crate::tes::interface_manager::ui_virtual_width(cx.settings);
    let input = &mut race_sex_mut(cx, menu).name_input_8ec;
    input.set_text(b"");
    input.set_font(font);
    input.set_max_pixel_width(width, virtual_width);
    input.set_enabled(true);
    Ok(())
}

// IDA: N/A the cursor's x and y in the interface's units as 005C2790 and 005C3BE0 read them: the virtual width x 0.5 (dbl
// 00A2FAA0) plus InterfaceManager +0x20, and the virtual height less (height x 0.5 + +0x28), each `__ftol2_sse` truncated; at
// 24-bit precision each step is a float.
fn race_cursor(cx: &InterfaceCx) -> (i32, i32) {
    let (x_offset, y_offset) = {
        let manager = cx
            .world
            .resource::<crate::tes::interface_manager::InterfaceManager>();
        (manager.cursor_state_020[0], manager.cursor_state_020[2])
    };
    let width = crate::tes::interface_manager::ui_virtual_width(cx.settings);
    let height = crate::tes::interface_manager::ui_virtual_height(cx.settings);
    let x = oblivion_core::x87::ftol(width * 0.5 + x_offset);
    let y = oblivion_core::x87::ftol(height - (height * 0.5 + y_offset));
    (x, y)
}

// IDA: N/A InterfaceManager +0x118 bit 2 (4): shift held.
fn shift_held(cx: &InterfaceCx) -> bool {
    cx.world
        .resource::<crate::tes::interface_manager::InterfaceManager>()
        .input_modifier_flags
        & 4
        != 0
}

/// IDA: 005C2790 RaceSexMenu_HandlePress(id, tile) (vt+0x1C, `retn 8`): for an id with a slider tile (+0x94 + 4 x id), +0x898 =
/// (the tile's cumulative x (00588C50) - the cursor's x) - its width (0xFCB); always +0x89C = the scroll bar's (+0x34)
/// cumulative y - the cursor's y; then id 2 (the player, the camera turn) without shift (InterfaceManager +0x118 bit 2):
/// +0x898 = the cursor's x - +0x8A0. Floats at 24-bit precision. Completion: 100%.
/// # Errors
/// A null scroll bar (the original faults).
pub fn race_sex_menu_handle_press(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: TileId,
) -> Result<(), GameError> {
    let (x, y) = race_cursor(cx);
    if race_sex(cx, menu).tiles_94.contains_key(&id) {
        let [left, _] = crate::tes::interface_scene::tile_cumulative_position(cx, tile);
        let width = tile_numeric_trait(cx, tile, crate::tes::tile_values::TRAIT_WIDTH);
        race_sex_mut(cx, menu).drag_origin_x_898 = (left - x as f32) - width;
    }
    let bar = race_sex(cx, menu)
        .scroll_bar_34
        .ok_or(GameError::unclassified(
            0x005C2816,
            "005C2816: a null +0x34 tile faults",
        ))?;
    let [_, top] = crate::tes::interface_scene::tile_cumulative_position(cx, bar);
    race_sex_mut(cx, menu).drag_origin_y_89c = top - y as f32;
    if id == 2 && !shift_held(cx) {
        let turn = race_sex(cx, menu).camera_turn_8a0;
        race_sex_mut(cx, menu).drag_origin_x_898 = x as f32 - turn;
    }
    Ok(())
}

/// IDA: 005C3BE0 RaceSexMenu_HandleHeld(id, tile) (vt+0x20, while the left button is held, `retn 8`): for an id with a slider
/// tile, the held tile's user9 = ((+0x898 + the cursor's x) - the slider's cumulative x) / the held tile's user8, as -1e9
/// (flt_A6B1F0), the value, 0.0; id 21 (the scroll bar) the same on +0x38 from +0x89C, the cursor's y and +0x34's cumulative
/// y, the value `__ftol2_sse` truncated; id 2 with +0x8D0 clear and no shift: `005C2BF0`(1.0, (the cursor's x - +0x898) /
/// 5.0) and +0x8A0 = the cursor's x - +0x898. Completion: 100%.
/// # Errors
/// A null +0x34 or +0x38 tile on id 21, or a stop in the camera.
pub fn race_sex_menu_handle_held(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: TileId,
) -> Result<(), GameError> {
    use crate::tes::tile_values::{TRAIT_USER8, TRAIT_USER9};
    let (x, y) = race_cursor(cx);
    if let Some(&slider) = race_sex(cx, menu).tiles_94.get(&id) {
        let origin = race_sex(cx, menu).drag_origin_x_898 + x as f32;
        let [left, _] = crate::tes::interface_scene::tile_cumulative_position(cx, slider);
        let value = (origin - left) / tile_numeric_trait(cx, tile, TRAIT_USER8);
        tile_set_numeric_trait(cx, tile, TRAIT_USER9, -1_000_000_000.0)?;
        tile_set_numeric_trait(cx, tile, TRAIT_USER9, value)?;
        return tile_set_numeric_trait(cx, tile, TRAIT_USER9, 0.0);
    }
    if id == 21 {
        let (bar, thumb) = {
            let data = race_sex(cx, menu);
            (
                data.scroll_bar_34.ok_or(GameError::unclassified(
                    0x005C3CC9,
                    "005C3CC9: a null +0x34 tile faults",
                ))?,
                data.tile_38.ok_or(GameError::unclassified(
                    0x005C3CE1,
                    "005C3CE1: a null +0x38 tile faults",
                ))?,
            )
        };
        let origin = race_sex(cx, menu).drag_origin_y_89c + y as f32;
        let [_, top] = crate::tes::interface_scene::tile_cumulative_position(cx, bar);
        let value = (origin - top) / tile_numeric_trait(cx, thumb, TRAIT_USER8);
        tile_set_numeric_trait(cx, thumb, TRAIT_USER9, -1_000_000_000.0)?;
        tile_set_numeric_trait(
            cx,
            thumb,
            TRAIT_USER9,
            oblivion_core::x87::ftol(value) as f32,
        )?;
        return tile_set_numeric_trait(cx, thumb, TRAIT_USER9, 0.0);
    }
    if id == 2 && race_sex(cx, menu).pause_8d0 == 0 && !shift_held(cx) {
        let turn = x as f32 - race_sex(cx, menu).drag_origin_x_898;
        race_sex_menu_update_camera(cx, menu, turn / 5.0)?;
        race_sex_mut(cx, menu).camera_turn_8a0 = turn;
    }
    Ok(())
}

/// IDA: 005C3DD0 RaceSexMenu_HandleKeyboardInput(key) (vt+0x30, `retn 4`): with the name field (+0x8EC) enabled, 0057FF50
/// on it (its width test through 0057DE00) and `005C30C0`, answering true; else false. Completion: 100%.
/// # Errors
/// As `race_sex_menu_store_name`.
pub fn race_sex_menu_handle_keyboard(
    cx: &mut InterfaceCx,
    menu: MenuId,
    key: u32,
) -> Result<bool, GameError> {
    if race_sex(cx, menu).name_input_8ec.enabled_00 == 0 {
        return Ok(false);
    }
    let fonts = cx
        .world
        .get_resource::<crate::tes::fonts::FontManager>()
        .cloned();
    race_sex_mut(cx, menu)
        .name_input_8ec
        .handle_key(key, |input, text| input.fits_width(fonts.as_ref(), text));
    race_sex_menu_store_name(cx, menu)?;
    Ok(true)
}

/// IDA: 005C30C0 RaceSexMenu_StoreName(menu): the name tile's string (0xFDE) takes 00580120's display text, and the
/// player's base form's name (TESFullName +0xA4 of the NPC) the editable text (004028D0). Completion: 100%.
/// # Errors
/// A null name tile, or the player with no base form.
pub fn race_sex_menu_store_name(cx: &mut InterfaceCx, menu: MenuId) -> Result<(), GameError> {
    let name = race_sex(cx, menu).name_30.ok_or(GameError::unclassified(
        0x005C30CF,
        "005C30CF: a null +0x30 tile faults",
    ))?;
    let shown = race_sex_mut(cx, menu).name_input_8ec.display_text();
    tile_set_string_trait(cx, name, crate::tes::tile_values::TRAIT_STRING, &shown)?;
    let npc = the_player(cx.world)
        .and_then(|player| cx.world.get::<crate::tes::actor::TesObjectRefr>(player))
        .and_then(|refr| refr.base_form_1c)
        .ok_or(GameError::unclassified(
            0x005C30EB,
            "005C30EB: the player with no base form",
        ))?;
    let text = race_sex(cx, menu).name_input_8ec.editable_text_18.clone();
    let mut full_name = cx
        .world
        .get_mut::<crate::tes::form_components::TesFullName>(npc)
        .ok_or(GameError::unclassified(
            0x005C30FD,
            "005C30FD: a base with no TESFullName",
        ))?;
    full_name.name_04.set(Some(&text), 0);
    Ok(())
}

/// IDA: 005CCA77..005CCBB2, Done's arm of 005CC200 (the back id 90 on Main): the name tile's string (0xFDE) must hold a byte
/// other than ' ' and differ from sEnterName (00B39440, `_mbscmp`); otherwise the sNoNameCharacter box (00B39438, no
/// callback, base 1, [sOk]) and 005CC200 returns at once (`retn 8`, no tail). A name that passes gives "%s %s?" of
/// sConfirmChoice2 (00B38668) when the race's name (TESFullName +0x1C, a null one "") starts with a vowel
/// (aeiouAEIOU), else sConfirmChoice1 (00B38660), and that name; the box (005C2BA0, base 1, [sNo, sYes]); then the tail.
/// Answers whether the tail runs. Completion: 100%.
/// # Errors
/// A null name tile or race, or a stop in the box.
fn race_sex_menu_done(cx: &mut InterfaceCx, menu: MenuId) -> Result<bool, GameError> {
    let name = race_sex(cx, menu).name_30.ok_or(GameError::unclassified(
        0x005CCA77,
        "005CCA77: a null +0x30 tile faults",
    ))?;
    let text =
        crate::tes::tile_values::tile_string_trait(cx, name, crate::tes::tile_values::TRAIT_STRING)
            .unwrap_or_default();
    let blank = text.iter().all(|&byte| byte == b' ');
    if text == text_at_or_empty(cx.world, 0x00B3_9440) || blank {
        let message = text_at_or_empty(cx.world, 0x00B3_9438);
        let ok = [text_at_or_empty(cx.world, 0x00B3_8CF0)];
        crate::tes::menus::show_message(cx, &message, None, 1, &ok)?;
        return Ok(false);
    }
    let race = the_player(cx.world)
        .and_then(|player| cx.world.get::<crate::tes::actor::TesObjectRefr>(player))
        .and_then(|refr| refr.base_form_1c)
        .map(|npc| crate::tes::forms_actors::base_race(cx.world, npc))
        .transpose()?
        .flatten()
        .ok_or(GameError::unclassified(
            0x005CCB2B,
            "005CCB2B: a null race faults",
        ))?;
    let race_name = cx
        .world
        .get::<crate::tes::form_components::TesFullName>(race)
        .and_then(|full_name| full_name.name_04.data_00.clone())
        .unwrap_or_default();
    let vowel = race_name
        .first()
        .is_some_and(|byte| b"aeiouAEIOU".contains(byte));
    let mut message = text_at_or_empty(cx.world, if vowel { 0x00B3_8668 } else { 0x00B3_8660 });
    message.push(b' ');
    message.extend_from_slice(&race_name);
    message.push(b'?');
    let buttons = [
        text_at_or_empty(cx.world, 0x00B3_8D00),
        text_at_or_empty(cx.world, 0x00B3_8CF8),
    ];
    crate::tes::menus::show_message(
        cx,
        &message,
        Some(crate::tes::menus::MessageCallback::RaceSexDone),
        1,
        &buttons,
    )?;
    Ok(true)
}

/// IDA: 005C2BA0 the Done confirm's callback: with the race menu, `button` 2 (sYes) runs 005C28F0 RaceSexMenu_AcceptCharacter, the menu's close (Build
/// 5 Target B: a stop here); any other does nothing. Completion: 100% of the test.
/// # Errors
/// The close, not ported.
pub fn race_sex_menu_done_confirm(cx: &mut InterfaceCx, button: u8) -> Result<(), GameError> {
    let present = cx
        .engine
        .tile_menu_by_id(MENU_ID_RACE_SEX)
        .and_then(|tile| cx.engine.containing_menu(tile))
        .is_some_and(|menu| matches!(cx.engine.menu(menu).class, MenuClass::RaceSex(_)));
    if present && button == 2 {
        return Err(GameError::unclassified(
            0x005C28F0,
            "005C28F0: the race menu's close (Build 5 Target B)",
        ));
    }
    Ok(())
}

/// IDA: 005C9C70 RaceSexMenu_Reset (cdecl): the race menu (TileMenu 1036's containing menu, a RaceSexMenu) else nothing;
/// `00521E40` on the player's base, `005C91E0` the head made again, `005C7BA0`(menu, 0) the pages. Completion: 100%.
/// # Errors
/// A stop in the calls.
pub fn race_sex_menu_reset(cx: &mut InterfaceCx) -> Result<(), GameError> {
    let Some(tile) = cx.engine.tile_menu_by_id(MENU_ID_RACE_SEX) else {
        return Ok(());
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(());
    };
    if !matches!(cx.engine.menu(menu).class, MenuClass::RaceSex(_)) {
        return Ok(());
    }
    let (_, npc, _, _) = player_npc(cx)?;
    npc_reset_face(cx.world, npc)?;
    race_sex_menu_recreate_head(cx)?;
    race_sex_menu_update_pages(cx, menu, false)?;
    Ok(())
}

/// IDA: 005C9F70 the Reset confirm's callback: with the race menu, `button` 2 (sYes, the second button) runs `005C9C70`; any
/// other clears 00B3B4C9. Completion: 100%.
/// # Errors
/// As `race_sex_menu_reset`.
pub fn race_sex_menu_reset_confirm(cx: &mut InterfaceCx, button: u8) -> Result<(), GameError> {
    let present = cx
        .engine
        .tile_menu_by_id(MENU_ID_RACE_SEX)
        .and_then(|tile| cx.engine.containing_menu(tile))
        .is_some_and(|menu| matches!(cx.engine.menu(menu).class, MenuClass::RaceSex(_)));
    if !present {
        return Ok(());
    }
    if button == 2 {
        race_sex_menu_reset(cx)
    } else {
        cx.world
            .get_resource_or_init::<RaceSexMenuStatics>()
            .word_b3b4c8 &= 0x00FF;
        Ok(())
    }
}

/// IDA: 005C9CD0 RaceSexMenu_Randomize (cdecl): the race menu (TileMenu 1036's containing menu, a RaceSexMenu) else
/// nothing; `005273A0`(player's base, 0, 0, 0), `005C91E0` the head made again; then from 005221C0's coefficients +0x880
/// = (1.0 - 0.0) x ((00553B30(0, 0) - 15.0) / 50.0) + 0.0 into Main/Age and, with s the NPC's gender sign (00520FF0),
/// +0x884 = ((00553B30(1, 0) - s) - -2.0) x 0.25 x (1.0 - 0.0) + 0.0 into Main/Complexion (user3 = (float)0x80000000,
/// the value, 0.0). Completion: 100%.
/// # Errors
/// A stop in the calls, or a null slider.
pub fn race_sex_menu_random(cx: &mut InterfaceCx) -> Result<(), GameError> {
    let Some(tile) = cx.engine.tile_menu_by_id(MENU_ID_RACE_SEX) else {
        return Ok(());
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(());
    };
    if !matches!(cx.engine.menu(menu).class, MenuClass::RaceSex(_)) {
        return Ok(());
    }
    let (_, npc, _, sex) = player_npc(cx)?;
    npc_randomize_face(cx.world, npc, false, false, false)?;
    race_sex_menu_recreate_head(cx)?;
    let coefficients = crate::tes::face_gen_head::npc_face_gen_coefficients(cx.world, npc)?;
    let span = 1.0_f32 - 0.0;
    let age =
        crate::tes::face_gen_head::face_gen_age_gender_control(cx.world, &coefficients, 0, 0)?;
    let age = span * ((age - 15.0) / 50.0) + 0.0;
    race_sex_mut(cx, menu).age_880 = age;
    let slider = page_child(cx, menu, GMST_MAIN, GMST_AGE)?;
    set_user3(cx, slider, age)?;
    let side = if sex != 0 { 2.0_f32 } else { -2.0 };
    let gender =
        crate::tes::face_gen_head::face_gen_age_gender_control(cx.world, &coefficients, 1, 0)?
            - side;
    let complexion = (gender - -2.0) * 0.25 * span + 0.0;
    race_sex_mut(cx, menu).complexion_884 = complexion;
    let slider = page_child(cx, menu, GMST_MAIN, GMST_COMPLEXION)?;
    set_user3(cx, slider, complexion)
}

/// IDA: 005C9FC0 the Random confirm's callback: with the race menu, `button` 2 (sYes, the second button) runs `005C9CD0`;
/// any other clears 00B3B4C8. Completion: 100%.
/// # Errors
/// As `race_sex_menu_random`.
pub fn race_sex_menu_random_confirm(cx: &mut InterfaceCx, button: u8) -> Result<(), GameError> {
    let present = cx
        .engine
        .tile_menu_by_id(MENU_ID_RACE_SEX)
        .and_then(|tile| cx.engine.containing_menu(tile))
        .is_some_and(|menu| matches!(cx.engine.menu(menu).class, MenuClass::RaceSex(_)));
    if !present {
        return Ok(());
    }
    if button == 2 {
        race_sex_menu_random(cx)
    } else {
        cx.world
            .get_resource_or_init::<RaceSexMenuStatics>()
            .word_b3b4c8 &= 0xFF00;
        Ok(())
    }
}

/// IDA: 005C50A0's rebuild arm after the hair's pick (005C5452..005C587D), on the biped node and its FaceGenHair geometry
/// `old`: 0070FD80's -90 degree Y rotation; "Meshes\%s" of the hair's model, its .tri and .egm, the model map's step
/// (`00553620` with no EGT, as the hair), the NPC's coefficients (`005221C0`) through 00559870; the clone (`make_hair_clone`
/// with the NPC's length +0x1CC and the hair's texture +0x40); the old geometry's app-culled bit (+0x18 bit 0) onto it; the
/// rotation; the node's vt+0x88 DetachChild(old) and vt+0x84 AttachChild(clone, 1); then the node's "FaceGenHair" (vt+0x58)
/// prepared (007B8940(hair, 1, 1, 1)) and tinted with the NPC's colour (`apply_hair_tint`); NiAVObject_RefreshEffectState
/// and RefreshPropertyState on the node. On the main thread (24-bit x87). No model, or no clone, leaves the old hair.
/// Completion: 100% but the texturing property (D13).
/// # Errors
/// A stop in the model, the clone, the attach or the refreshes.
pub fn swap_hair(
    world: &mut World,
    npc: Entity,
    hair: Entity,
    node: Entity,
    old: Entity,
    services: &mut crate::bs::model_loader::ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let model_path = world
        .get::<crate::tes::form_components::TesModel>(hair)
        .map(|model| crate::tes::facegen::face_gen_head::path_text(&model.path_04))
        .unwrap_or_default();
    let texture = world
        .get::<crate::tes::data_handler::TesTexture>(hair)
        .map(|texture| crate::tes::facegen::face_gen_head::path_text(&texture.path_04))
        .unwrap_or_default();
    let mut nif = b"Meshes\\".to_vec();
    nif.extend_from_slice(&model_path);
    let coefficients = crate::tes::facegen::face_gen_head::npc_face_gen_coefficients(world, npc)?;
    let (length, color) = {
        let data = world
            .get::<crate::tes::forms_actors::TesNpc>(npc)
            .expect("an NPC");
        (data.hair_length_1cc, data.hair_color_1e8)
    };
    let use_heads = world
        .get_resource_or_init::<crate::tes::character_3d::FaceGenSettings>()
        .use_face_gen_heads_b120b4;
    let mut clone = None;
    crate::tes::facegen::face_gen_tasks::with_face_gen_files(
        world,
        true,
        services,
        &mut |world, files, prepare| {
            let Some(model) = crate::tes::facegen::face_gen_tasks::morphed_model(
                world,
                &nif,
                false,
                true,
                &coefficients,
                use_heads,
                files,
            )?
            else {
                return Ok(());
            };
            let made = make_hair_clone(
                world,
                model.geometry,
                &model.positions,
                model.morph_data.as_ref(),
                length,
                &texture,
                prepare,
            )?;
            world.entity_mut(made).insert(model.part);
            clone = Some(made);
            Ok(())
        },
    )?;
    let Some(clone) = clone else {
        return Ok(());
    };
    let culled = world
        .get::<MeshTriShapePayload>(old)
        .is_some_and(|payload| payload.effective_av_flags & 1 != 0);
    let angle = f64::from(-1.570_796_4_f32);
    let (c, s) = (angle.cos() as f32, angle.sin() as f32);
    {
        let mut payload =
            world
                .get_mut::<MeshTriShapePayload>(clone)
                .ok_or(GameError::unclassified(
                    0x005C566D,
                    "005C566D: a hair clone that isn't a geometry",
                ))?;
        if culled {
            payload.effective_av_flags |= 1;
        }
        payload.local_transform.rotation = [c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c];
    }
    crate::gamebryo::scene_node::detach_child(world, node, old);
    if attach_scene_child(world, node, Some(clone), true) != AttachChildStatus::Attached {
        return Err(GameError::unclassified(
            0x005C56CF,
            "005C56CF: attaching the new hair",
        ));
    }
    apply_hair_tint(world, node, color);
    let defaults = services.prepare.property_defaults;
    crate::gamebryo::scene_state::refresh_effect_state(world, node)?;
    crate::gamebryo::scene_state::refresh_property_state(world, node, &defaults)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 005CDEF0 compares the sliders' packed colour with the NPC's every frame; a slider set from a byte (byte / 255.0, as
    // 005C7BA0 and 005C5F00 set them) packs back to that byte, so a colour doesn't rewrite itself. run-aq's start: 00222C31.
    #[test]
    fn a_byte_set_colour_packs_back_to_itself() {
        for byte in 0..=255u32 {
            let value = byte as f32 / 255.0;
            assert_eq!(
                pack_slider_color(value, value, value),
                byte << 16 | byte << 8 | byte,
                "byte {byte}"
            );
        }
        let red = 0x31 as f32 / 255.0;
        let green = 0x2C as f32 / 255.0;
        let blue = 0x22 as f32 / 255.0;
        assert_eq!(pack_slider_color(red, green, blue), 0x0022_2C31);
    }
}
