//! Phase 07: the original tile objects (decision D2 of the Phase 07 handoff). Original x86 sizes: Tile 0x40, TileValue
//! 0x1C,
//! TileExpression 0x18, TileRect 0x44, TileImage 0x4C, TileMenu 0x48. Vtables: Tile 00A690E8, TileRect 00A69138,
//! TileImage
//! 00A69110, TileText 00A6AEC4, TileMenu 00A6AF14. Trait IDs, values, order and failure paths are the original's. The
//! binary
//! layout stays a recorded substitution, as in the C++ port.
//!
//! Tiles are plain data in arenas, not entities. Nothing ever asks for tiles by component, and every order the original
//! can
//! observe (the child list, the value list, the expression and dependency chains, the menu table) is an explicit list
//! here, so
//! a query could only get in the way. The chains link nodes of one tile's value into another tile's value, so an
//! evaluation
//! needs many values at once, which `TileEngine`'s indexes give without borrow trouble. A tile's +0x24 scene node is an
//! `Entity` in the Phase 05 scene graphs. The arenas never free a tile: 0058FAB0 is carried to the menu close work.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::interface_manager::InterfaceSettings;
use super::interface_scene::TileSceneRecord;
use super::loading_menu::LoadingMenu;
use super::tile_xml::{TileParseContext, TileTemplate, TileXmlBuffer};
use crate::bs::file::OpenedFile;
use crate::gamebryo::scene_update::SceneUpdateRuntime;

/// IDA: N/A an index into `TileEngine::tiles`, where the original has a Tile*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileId(pub u32);
/// IDA: N/A an index into `TileEngine::values`, where the original has a Tile::Value*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueId(pub u32);
/// IDA: N/A an index into `TileEngine::expressions`, where the original has a TileExpression*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExprId(pub u32);
/// IDA: N/A an index into `TileEngine::menus`, where the original has a Menu*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuId(pub u32);

/// IDA: TileExpression (0x18): +0x00 prev, +0x04 next, +0x08 operand (a float constant, the cached source number of a
/// reference,
/// the action opcode of a marker, or the owning value of a list head), +0x0C u16 opcode (101 head, 10/15 begin/end
/// markers,
/// 2001..2027 actions), +0x10 previous node of the source value's dependency chain, +0x14 next dependent. Host: the
/// operand union
/// is split by use. Completion: 100%.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileExpression {
    pub prev_00: Option<ExprId>,
    pub next_04: Option<ExprId>,
    /// +0x08 as float.
    pub number_08: f32,
    /// +0x08 as the marker's action opcode.
    pub action_08: u32,
    /// +0x08 as the head's Tile::Value*.
    pub owner_08: Option<ValueId>,
    pub opcode_0c: u16,
    pub high_0e: u16,
    pub dependency_prev_10: Option<ExprId>,
    pub dependency_next_14: Option<ExprId>,
}

/// IDA: Tile::Value (0x1C): +0x00 parent tile, +0x04 number, +0x08 BSStringT, +0x10 expression head, +0x14 dependency
/// head,
/// +0x18 trait, +0x1A isNumber. Text is bytes; an empty text is the original's null BSStringT. Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct TileValue {
    pub parent_tile_00: Option<TileId>,
    pub number_04: f32,
    pub text_08: Vec<u8>,
    pub expression_head_10: ExprId,
    pub dependency_head_14: ExprId,
    pub trait_18: u16,
    pub is_number_1a: bool,
}

/// IDA: Tile type IDs (vt+0x0C GetTypeID): 901 rect, 902 image, 903 text, 904 Tile3D, 905 menu. 907 TileWindow has no
/// host kind: 005902A0 reports it as a failed creation. Completion: 100% for the five kinds, Tile3D with its model a counted boundary.
/// TODO: [ ] TileWindow (M6, Phase 21's menus).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileKind {
    Rect,
    Image,
    Text,
    Model,
    Menu,
}

impl TileKind {
    /// IDA: vt+0x0C: TileRect 00581A10 (901), TileImage 005818D0 (902), TileText 0058FE30 (903), Tile3D 0058FFC0 (904),
    /// TileMenu 005900D0 (905).
    /// Completion: 100%.
    #[must_use]
    pub const fn type_id(self) -> u32 {
        match self {
            Self::Rect => 901,
            Self::Image => 902,
            Self::Text => 903,
            Self::Model => 904,
            Self::Menu => 905,
        }
    }
}

/// IDA: Tile (0x40 base) and its kinds' extra fields, which sit together here. Completion: 100% of the fields Phase 07
/// writes.
/// TODO: [ ] 0058FAB0 the destructor (menu close, later phases).
#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    pub kind: TileKind,
    /// +0x04 released byte.
    pub released_04: u8,
    /// +0x05 byte; blocks 0058FBA0 and 0058BEE0 when set.
    pub unknown_05: u8,
    /// +0x06 byte.
    pub unknown_06: u8,
    /// +0x08 BSStringT name.
    pub name_08: Vec<u8>,
    /// Tile +0x10 parent.
    pub parent: Option<TileId>,
    /// +0x14 NiTList<Tile::Value*>, in trait order.
    pub values_14: Vec<ValueId>,
    /// +0x24 NiAVObject*, its substituted geometry and the "Tileptr" extra (D3).
    pub scene_24: Option<TileSceneRecord>,
    /// +0x2C dirty flags: Tile_Initialize stores 2, 0058B2F0 sets 1/2/4/8/0x10/0x20/0x40/0x100/0x200.
    pub flags_2c: u32,
    /// +0x30 NiTList<Tile*>, list head first: index 0 is the last child 0058D1C0 added at the front.
    pub children_30: Vec<TileId>,
    /// TileRect and TileMenu +0x40: 1 when 00591DC0 built a BSScissorTriShape.
    pub scissor_40: u8,
    /// TileMenu +0x44 Menu*, created by vt+0x14 00591C50.
    pub menu_44: Option<MenuId>,
    /// Tile3D +0x50: the path 00590970 formats. Its model isn't loaded: `tile_load_model_counted`.
    pub model_path_50: Option<Vec<u8>>,
    /// TileImage +0x40, 00581810's 1.0.
    pub image_scale_40: f32,
    /// TileImage +0x44 (591A80): what 00591360 takes over the file.
    pub image_override_44: Option<std::sync::Arc<super::interface_scene::SavePreview>>,
    /// TileImage +0x48.
    pub byte_48: u8,
    /// +0x28 the TileTraitTransition list, head first (00588A70 prepends).
    pub transitions_28: Vec<super::interface_frame::TileTraitTransition>,
    /// TileText +0x44..+0x4C: the 00576670 shape's translate.
    pub geometry_translate_44: [f32; 3],
    /// TileText +0x50: the ishtml byte of the HTML build.
    pub html_50: u8,
}

impl Tile {
    /// IDA: N/A host helper. Completion: 100%.
    #[must_use]
    pub const fn type_id(&self) -> u32 {
        self.kind.type_id()
    }
}

/// IDA: 00587D20 Menu_CreateByID's classes. 1007 and 1004 run before MainMenu. `Unported` keeps the original's ID check
/// reachable:
/// a test's host may answer with another class. Completion: 100% of the reached class.
/// TODO: [ ] The cases still unported (`docs/open-items.md`, interface menus), with their menus.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuClass {
    Loading(LoadingMenu),
    Main(super::menus::MainMenu),
    Message(super::menus::MessageMenu),
    Load(super::menus::LoadgameMenu),
    Hud(super::hud_main_menu::HudMainMenu),
    HudInfo(super::hud_main_menu::HudInfoMenu),
    HudSubtitle(super::hud_subtitle_menu::HudSubtitleMenu),
    SleepWait(super::sleep_wait_menu::SleepWaitMenu),
    Pause(super::pause_menu::PauseMenu),
    Options(super::options_menu::OptionsMenu),
    Quantity(super::quantity_menu::QuantityMenu),
    Audio(super::audio_menu::AudioMenu),
    Gameplay(super::gameplay_menu::GameplayMenu),
    Controls(Box<super::controls_menu::ControlsMenu>),
    MagicPopup(super::magic_popup_menu::MagicPopupMenu),
    Breath(super::breath_menu::BreathMenu),
    Recharge(super::recharge_menu::RechargeMenu),
    TextEdit(super::text_edit_menu::TextEditMenu),
    Save(super::save_menu::SaveMenu),
    Stats(super::stats_menu::StatsMenu),
    QuickKeys(super::quick_keys_menu::QuickKeysMenu),
    Map(super::map_menu::MapMenu),
    Magic(super::magic_menu::MagicMenu),
    Inventory(super::inventory_menu::InventoryMenu),
    Unported(u32),
}

/// IDA: 00584640 Menu_Constructor's fields (Menu, 0x28). Completion: 100% of the fields Phase 07 reads and writes.
/// TODO: [ ] vt+0x04 SetField: a nullsub for Menu and LoadingMenu, so 0058B2F0's id trait has nothing to call yet.
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub class: MenuClass,
    /// +0x04 TileMenu*.
    pub tile_04: Option<TileId>,
    /// +0x08 template list; Menu_AddTemplate 00584960 prepends.
    pub templates_08: Vec<TileTemplate>,
    /// +0x10 Tile*, the 0058B800 "&last;" source.
    pub unknown_10: Option<TileId>,
    /// +0x14 menu mode.
    pub menu_mode_14: u32,
    /// +0x18 int32 depth (0058B623 fild, 0058B634 ftol).
    pub depth_18: i32,
    /// +0x1C owns-templates byte.
    pub owns_templates_1c: bool,
    /// +0x20 ID copy (00591C50: vt+0x34).
    pub id_20: u32,
    /// +0x24 visibility state: 00584659 `mov dword ptr [eax+24h], 4`, Hidden. The C++ port leaves it 0.
    pub state_24: u32,
}

impl Menu {
    /// IDA: 00584640. Completion: 100%.
    #[must_use]
    pub const fn new(class: MenuClass) -> Self {
        Self {
            class,
            tile_04: None,
            templates_08: Vec::new(),
            unknown_10: None,
            menu_mode_14: 0,
            depth_18: 0,
            owns_templates_1c: true,
            id_20: 0,
            state_24: 4,
        }
    }

    /// IDA: vt+0x34 GetID (LoadingMenu 005AD370: 1007). Completion: 100%.
    #[must_use]
    pub const fn id(&self) -> u32 {
        match self.class {
            MenuClass::Loading(_) => 1007,
            MenuClass::Main(_) => 1044,
            MenuClass::Message(_) => 1001,
            MenuClass::Load(_) => 1038,
            MenuClass::Hud(_) => 1004,
            MenuClass::HudInfo(_) => 1005,
            MenuClass::HudSubtitle(_) => 1010,
            MenuClass::SleepWait(_) => 1012,
            MenuClass::Pause(_) => 1013,
            MenuClass::Options(_) => 1015,
            MenuClass::Quantity(_) => 1016,
            MenuClass::Audio(_) => 1017,
            MenuClass::Gameplay(_) => 1020,
            MenuClass::Controls(_) => 1021,
            MenuClass::MagicPopup(_) => 1024,
            MenuClass::Breath(_) => 1045,
            MenuClass::Recharge(_) => 1049,
            MenuClass::TextEdit(_) => 1051,
            MenuClass::Save(_) => 1039,
            MenuClass::Stats(_) => 1003,
            MenuClass::QuickKeys(_) => 1046,
            MenuClass::Map(_) => 1023,
            MenuClass::Magic(_) => 1022,
            MenuClass::Inventory(_) => 1002,
            MenuClass::Unported(id) => id,
        }
    }
}

/// IDA: 0058A0A0's 0x10 token entry: +0 ID, +4 refcount, name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileTokenEntry {
    pub id: u32,
    pub refcount: u32,
    pub name: Vec<u8>,
}

/// IDA: 00589B70 / 00587B20's table size (the same slots as `InterfaceMovieReads`).
pub use super::interface_manager::TILE_MENU_SLOTS;

/// IDA: N/A owner of the tile engine's globals and arenas. Completion: 100% of the globals Phase 07 reaches:
/// - 00B3B0B0..00B3B26F the 28 built-in token lists (index 0 other, 1..26 letters, 27 '&'), 00B13BC4 the underscore
///   tokens (IDs
///   10000 + index);
/// - 00B3AF10 the 100-entry evaluation stack and 00B3B0AC its depth (005888A0);
/// - 00B3B0A0 parse failed, 00B3B0A1 unterminated tile (the evaluation gate), 00B3B0A2 in scene update;
/// - 00B06B18 bDisableWarning:MESSAGES (saved and restored by 00590420), the 00B34D88 warning counter: every direct
///   reference in the exe is a read, a save and restore or a store of 0 (004CDAB1, 004CDBB7, 004DF850, 004DFB6A, 0052B0EE,
///   0052B10A), with no increment by address, so nothing in Rust writes it either;
/// - the 00B3554C and 00B3556C XML buffer and parse caches, keyed by the file name after the last '\';
/// - 00B13970 g_tileMenuArray (TileMenu* by menu ID - 1001). Only the main thread touches this table. The MoviePlayer
///   thread's
///   read of it is `InterfaceMovieReads`, stored beside this one by `set_tile_menu`.
#[derive(Resource, Debug)]
pub struct TileEngine {
    pub tiles: Vec<Tile>,
    pub values: Vec<TileValue>,
    pub expressions: Vec<TileExpression>,
    pub free_expressions: Vec<ExprId>,
    pub menus: Vec<Menu>,
    pub builtin_tokens: [Vec<TileTokenEntry>; 28],
    pub custom_tokens: Vec<TileTokenEntry>,
    pub builtins_registered: bool,
    pub evaluation_stack: [Option<ValueId>; 100],
    pub evaluation_depth: u32,
    pub parse_failed: bool,
    pub unterminated_tile: bool,
    pub in_scene_update: bool,
    pub disable_warning_messages: bool,
    pub warning_count: u32,
    pub buffer_cache: Vec<(Vec<u8>, TileXmlBuffer)>,
    pub parse_cache: Vec<(Vec<u8>, TileParseContext)>,
    pub tile_menus: [Option<TileId>; TILE_MENU_SLOTS],
    /// 00B3A708: the 00587B20 one-byte allocation (a host flag).
    pub menu_array_allocated_b3a708: bool,
    /// N/A what stands for a TileTraitTransition's address.
    pub next_transition_id: u32,
    /// dword_B3B3FC[3..].
    pub main_menu_statics: super::menus::MainMenuStatics,
}

impl Default for TileEngine {
    /// IDA: N/A host helper. Completion: 100%.
    fn default() -> Self {
        Self {
            tiles: Vec::new(),
            values: Vec::new(),
            expressions: Vec::new(),
            free_expressions: Vec::new(),
            menus: Vec::new(),
            builtin_tokens: std::array::from_fn(|_| Vec::new()),
            custom_tokens: Vec::new(),
            builtins_registered: false,
            evaluation_stack: [None; 100],
            evaluation_depth: 0,
            parse_failed: false,
            unterminated_tile: false,
            in_scene_update: false,
            disable_warning_messages: false,
            warning_count: 0,
            buffer_cache: Vec::new(),
            parse_cache: Vec::new(),
            tile_menus: [None; TILE_MENU_SLOTS],
            menu_array_allocated_b3a708: false,
            next_transition_id: 0,
            main_menu_statics: super::menus::MainMenuStatics::default(),
        }
    }
}

impl TileEngine {
    /// IDA: N/A host helper. Completion: 100%.
    #[must_use]
    pub fn tile(&self, id: TileId) -> &Tile {
        &self.tiles[id.0 as usize]
    }
    /// IDA: N/A host helper. Completion: 100%.
    pub fn tile_mut(&mut self, id: TileId) -> &mut Tile {
        &mut self.tiles[id.0 as usize]
    }
    /// IDA: N/A host helper. Completion: 100%.
    #[must_use]
    pub fn value(&self, id: ValueId) -> &TileValue {
        &self.values[id.0 as usize]
    }
    /// IDA: N/A host helper. Completion: 100%.
    pub fn value_mut(&mut self, id: ValueId) -> &mut TileValue {
        &mut self.values[id.0 as usize]
    }
    /// IDA: N/A host helper. Completion: 100%.
    #[must_use]
    pub fn expression(&self, id: ExprId) -> &TileExpression {
        &self.expressions[id.0 as usize]
    }
    /// IDA: N/A host helper. Completion: 100%.
    pub fn expression_mut(&mut self, id: ExprId) -> &mut TileExpression {
        &mut self.expressions[id.0 as usize]
    }
    /// IDA: N/A host helper. Completion: 100%.
    #[must_use]
    pub fn menu(&self, id: MenuId) -> &Menu {
        &self.menus[id.0 as usize]
    }
    /// IDA: N/A host helper. Completion: 100%.
    pub fn menu_mut(&mut self, id: MenuId) -> &mut Menu {
        &mut self.menus[id.0 as usize]
    }

    /// IDA: 00589B70 TileMenu_GetByMenuID: `lea ecx, [eax-3E9h]`, the slot when below the array size, else null.
    /// Completion: 100%.
    #[must_use]
    pub fn tile_menu_by_id(&self, menu_id: u32) -> Option<TileId> {
        let slot = menu_id.wrapping_sub(1001) as usize;
        self.tile_menus.get(slot).copied().flatten()
    }

    /// IDA: 005898F0 Tile_GetContainingMenu: walks up to the tile whose parent has no parent, or a parentless tile
    /// (005898F7..00589908), and returns +0x44 only if that top tile is type 0x389 (0058991B), else null. It is not the
    /// nearest TileMenu: a menu below the top is not found, and the C++ differs (its walk stops at the first menu).
    /// Completion: 100%.
    #[must_use]
    pub fn containing_menu(&self, tile: TileId) -> Option<MenuId> {
        let mut top = tile;
        while let Some(parent) = self.tile(top).parent {
            if self.tile(parent).parent.is_none() {
                break;
            }
            top = parent;
        }
        let top = self.tile(top);
        if top.kind == TileKind::Menu {
            top.menu_44
        } else {
            None
        }
    }
}

/// IDA: N/A what the interface code reaches outside the game: the C++ `InterfaceServices` and `TileEngineServices`
/// functions. Files
/// go through the Phase 03 file finder and archives, device and movie state through the host, in the same place as the
/// other
/// phases' `*Platform` traits. `site` style arguments are not needed, since each method is one original call.
pub trait InterfaceHost {
    /// IDA: 00431130(path, 0, 0x2800, 0x80), BSFile open for 005744E0's font and texture files. `None` is a null
    /// result. Completion: 100%.
    fn open_font_file(&mut self, path: &str) -> Option<OpenedFile>;
    /// IDA: 00431130(path, 0, 0x2800, 4) and the read of 00584F10: the file bytes, `None` when it is not good. Completion: 100%.
    fn read_tile_file(&mut self, path: &str) -> Option<Vec<u8>>;
    /// IDA: the FileFinder FindFile (g_FileFinder 00B33A04) existence test of 00591030. Completion: 100%.
    fn file_exists(&mut self, path: &str) -> bool;
    /// IDA: 00591360 NiSourceTexture_CreateFromFile: the loaded texture's width and height (vt 00780D10 / 0076DDF0),
    /// `None` when it
    /// cannot be created. Completion: 100%.
    fn texture_size(&mut self, path: &str) -> Option<[u32; 2]>;
    /// IDA: 0040FDA0 MoviePlayer_IsRunning. Completion: 100%.
    fn movie_player_is_running(&mut self) -> bool;
    /// IDA: 0057F170 the interface render, a host substitution (D3): M6 draws the menu here. Completion: 100%.
    fn render_interface(&mut self, argument: u32);
    /// IDA: 00403190 OSInputGlobals mouse axis query (1, 2). Completion: 100%.
    fn mouse_axis(&mut self, axis: i32) -> i32;
    /// IDA: 0047DF80 Random_Next(0), the main-thread CRT rand of the 2006 "rand" tile action. Completion: 100%.
    fn random_next(&mut self) -> i32;
    /// IDA: 00578E00 -> LoadingMenu_UpdateDisplay(0), from 0058E92E outside the LoadingMenu. Completion: 100%.
    fn loading_update_display(&mut self);
    /// IDA: DebugMessage_Warning 004A7A60 and 00533D30 -> DebugLog_Dispatch: the host log. Completion: 100%.
    fn warning(&mut self, text: &str);
    /// IDA: OutputDebugStringA (00584FC1). Completion: 100%.
    fn output_debug_string(&mut self, text: &str);
    /// IDA: DebugLog_Level2Printf (0040FEC0). Completion: 100%.
    fn debug_log(&mut self, text: &str);
    /// IDA: GetTickCount, for 005AD0D0 and 005AD980. Completion: 100%.
    fn tick_count(&mut self) -> u32;
    /// IDA: 005AD158 OSSoundGlobals_PauseGameSounds, when g_osGlobals has its sound object. Completion: 100%.
    fn pause_game_sounds(&mut self);
    /// IDA: 0057DE50 InterfaceManager_PlayUISound(id): the sound agent's. Completion: 100% of the seam.
    fn play_ui_sound(&mut self, _sound: u32) {}
    /// IDA: 0045D450 TESSaveLoadGame_RebuildSaveFileList's result: the saves, newest first. Completion: 100% of the seam.
    fn save_file_list(&mut self) -> Vec<super::save_files::SaveFile> {
        Vec::new()
    }
    /// IDA: 005AE190 and 005B5D60's TESSaveLoadGame_LoadGame, 005B5DF0's new game: a later phase's. A host stop by return: the menu stays. Completion: 100% of the seam.
    fn start_game(&mut self, _request: super::menus::GameStartRequest) {}
    /// IDA: 005B59B0's g_osGlobals->quitGame = 1. Completion: 100% of the seam.
    fn quit_game(&mut self) {}
    /// IDA: 005ADC76 006A9C00 OSSoundGlobals_ResumeGameSounds, with g_osGlobals' sound and no movie, or one in state 0 or 2. Completion: 100% of the seam.
    fn resume_game_sounds_after_loading(&mut self) {}
    /// IDA: 00B1436C sCharGenQuest:General, by 005B5E35's _strtol. The image's value is 0002466E.
    fn chargen_quest_id(&mut self) -> u32 {
        0x0002_466E
    }
    /// IDA: 005B5EB0..005B5EDC: 00B3C0EC = 0, then with sound 006AB160(-1, 0, 0), 006AB160(2, 0, 0) and 006AB420.
    fn new_game_music(&mut self, _world: &mut World) {}
    /// IDA: 005B58A4..005B590E MainMenu_Destructor's media: 00B3C0EC = 0, then with sound 006A8D50, 006AB160(0xFFFF, null, 0) and on true 006AB420; a current movie whose state is neither 0 nor 2 then 006A9B40 and 006A8D00.
    fn main_menu_destructor_media(&mut self, _world: &mut World) {}
    /// IDA: 005B5F20 MainMenu_ctor's movie and music, which need the driver's context. Returns 00B3B408. Completion: 100% of the seam.
    fn main_menu_ctor_media(&mut self, _world: &mut World) -> bool {
        false
    }
    /// IDA: 00587D20 Menu_CreateByID for a class other than 1007. `Ok(None)` is its null result. The default stops,
    /// since those
    /// classes are carried. Completion: 100%.
    fn create_unported_menu(&mut self, _menu_id: u32) -> Result<Option<MenuClass>, StreamError> {
        Err(StreamError(
            "00587D20: this menu class is carried to its phase",
        ))
    }
}

/// IDA: N/A everything an interface function reaches: the World with the scene graphs and the interface resources, the
/// tile engine
/// (taken out of the World so that both can be borrowed), the settings and the host.
pub struct InterfaceCx<'a> {
    pub world: &'a mut World,
    pub engine: &'a mut TileEngine,
    pub settings: &'a InterfaceSettings,
    pub host: &'a mut dyn InterfaceHost,
    pub runtime: &'a mut dyn SceneUpdateRuntime,
}

/// IDA: N/A runs `body` with the `TileEngine` resource out of the World. Later phases' callers of the interface code
/// enter here. Completion: 100%.
pub fn with_interface_cx<R>(
    world: &mut World,
    settings: &InterfaceSettings,
    host: &mut dyn InterfaceHost,
    runtime: &mut dyn SceneUpdateRuntime,
    body: impl FnOnce(&mut InterfaceCx) -> R,
) -> R {
    world.init_resource::<TileEngine>();
    world.resource_scope(|world, mut engine: Mut<TileEngine>| {
        let mut cx = InterfaceCx {
            world,
            engine: &mut engine,
            settings,
            host,
            runtime,
        };
        body(&mut cx)
    })
}

/// IDA: N/A a path or name lifted from original bytes to the host's `&str` file API, byte for char, as elsewhere in the
/// port. Completion: 100%.
#[must_use]
pub fn lift_text(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

/// IDA: N/A a host for tests that reach no file or device: warnings and debug lines are kept, everything else answers nothing.
#[cfg(test)]
#[derive(Default)]
pub struct NullInterfaceHost {
    pub warnings: Vec<String>,
    pub debug: Vec<String>,
    /// What `tick_count` answers, and what each call adds afterwards.
    pub tick: u32,
    pub tick_step: u32,
}

#[cfg(test)]
impl InterfaceHost for NullInterfaceHost {
    /// IDA: N/A host helper. Completion: 100%.
    fn open_font_file(&mut self, _: &str) -> Option<OpenedFile> {
        None
    }
    /// IDA: N/A host helper. Completion: 100%.
    fn read_tile_file(&mut self, _: &str) -> Option<Vec<u8>> {
        None
    }
    /// IDA: N/A host helper. Completion: 100%.
    fn file_exists(&mut self, _: &str) -> bool {
        false
    }
    /// IDA: N/A host helper. Completion: 100%.
    fn texture_size(&mut self, _: &str) -> Option<[u32; 2]> {
        None
    }
    /// IDA: N/A host helper. Completion: 100%.
    fn movie_player_is_running(&mut self) -> bool {
        false
    }
    /// IDA: N/A host helper. Completion: 100%.
    fn render_interface(&mut self, _: u32) {}
    /// IDA: N/A host helper. Completion: 100%.
    fn mouse_axis(&mut self, _: i32) -> i32 {
        0
    }
    /// IDA: N/A host helper. Completion: 100%.
    fn random_next(&mut self) -> i32 {
        16383
    }
    /// IDA: N/A host helper. Completion: 100%.
    fn loading_update_display(&mut self) {}
    /// IDA: N/A host helper. Completion: 100%.
    fn warning(&mut self, text: &str) {
        self.warnings.push(text.to_owned());
    }
    /// IDA: N/A host helper. Completion: 100%.
    fn output_debug_string(&mut self, _: &str) {}
    /// IDA: N/A host helper. Completion: 100%.
    fn debug_log(&mut self, text: &str) {
        self.debug.push(text.to_owned());
    }
    /// IDA: N/A host helper. Completion: 100%.
    fn tick_count(&mut self) -> u32 {
        let tick = self.tick;
        self.tick = tick.wrapping_add(self.tick_step);
        tick
    }
    /// IDA: N/A host helper. Completion: 100%.
    fn pause_game_sounds(&mut self) {}
}
