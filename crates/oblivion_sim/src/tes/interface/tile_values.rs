//! Phase 07: the original tile token registry, value and expression engine, source resolution, parenting, the TileMenu
//! registry and
//! the scene traversal (D2). x87: main thread after CreateDevice (24-bit precision), so every arithmetic result is an
//! `f32`.
//! Evidence: evidence/phase07-window-2026-10-06 (before/*.c, P07-RUN-001).

use oblivion_core::crt::{atol, stricmp};
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::interface_manager::{
    InterfaceManager, InterfaceMovieReads, menu_create_by_id, menu_register_tile,
    menu_update_cursor_depth,
};
use super::interface_scene::{interface_update_tile_scene, tile_scene_node};
use super::tile::{
    ExprId, InterfaceCx, Menu, TileExpression, TileId, TileKind, TileTokenEntry, TileValue, ValueId,
};
use super::tile_tokens_table::BUILTIN_TOKENS;
use crate::gamebryo::scene_node::SceneNodePayload;
use crate::gamebryo::scene_update::{SceneUpdateStatus, update_scene_object};

/// IDA: Trait IDs (0058A1C0 token table) that this file names.
pub const TRAIT_VISIBLE: u16 = 4001;
pub const TRAIT_CLASS: u16 = 4002;
pub const TRAIT_LIST_CLIP: u16 = 4003;
pub const TRAIT_CLIP_WINDOW: u16 = 4004;
pub const TRAIT_STACKING: u16 = 4005;
pub const TRAIT_LOCUS: u16 = 4006;
pub const TRAIT_ALPHA: u16 = 4007;
pub const TRAIT_ID: u16 = 4008;
pub const TRAIT_DEPTH: u16 = 4011;
pub const TRAIT_Y: u16 = 4012;
pub const TRAIT_X: u16 = 4013;
pub const TRAIT_USER0: u16 = 4014;
/// IDA: 005A491A's trait.
pub const TRAIT_USER1: u16 = 0xfaf;
pub const TRAIT_USER2: u16 = 4016;
pub const TRAIT_USER3: u16 = 4017;
pub const TRAIT_USER4: u16 = 4018;
pub const TRAIT_USER5: u16 = 4019;
pub const TRAIT_USER6: u16 = 4020;
pub const TRAIT_USER7: u16 = 4021;
pub const TRAIT_USER8: u16 = 4022;
pub const TRAIT_USER9: u16 = 4023;
pub const TRAIT_USER10: u16 = 4024;
pub const TRAIT_USER11: u16 = 4025;
pub const TRAIT_USER12: u16 = 4026;
pub const TRAIT_USER13: u16 = 4027;
pub const TRAIT_USER14: u16 = 4028;
pub const TRAIT_USER15: u16 = 4029;
pub const TRAIT_CLIPS: u16 = 4040;
/// IDA: `target`.
pub const TRAIT_TARGET_ID: u16 = 4041;
pub const TRAIT_HEIGHT: u16 = 4042;
pub const TRAIT_WIDTH: u16 = 4043;
pub const TRAIT_RED: u16 = 4044;
pub const TRAIT_GREEN: u16 = 4045;
pub const TRAIT_BLUE: u16 = 4046;
pub const TRAIT_TILE_MODE: u16 = 4047;
pub const TRAIT_CHILD_COUNT: u16 = 4048;
pub const TRAIT_JUSTIFY: u16 = 4049;
pub const TRAIT_ZOOM: u16 = 4050;
pub const TRAIT_FONT: u16 = 4051;
pub const TRAIT_WRAP_WIDTH: u16 = 4052;
pub const TRAIT_WRAP_LIMIT: u16 = 4053;
pub const TRAIT_WRAP_LINES: u16 = 4054;
pub const TRAIT_PAGE_NUM: u16 = 4055;
pub const TRAIT_IS_HTML: u16 = 4056;
pub const TRAIT_CROP_Y: u16 = 4057;
pub const TRAIT_CROP_X: u16 = 4058;
pub const TRAIT_STRING: u16 = 4062;
pub const TRAIT_FILENAME: u16 = 4070;
pub const TRAIT_FILE_WIDTH: u16 = 4071;
pub const TRAIT_FILE_HEIGHT: u16 = 4072;
pub const TRAIT_DEPTH_3D: u16 = 4077;
/// IDA: 00581D70: selectedBigFourMenuID.
pub const TRAIT_SELECTED_BIG_FOUR: u16 = 0x1771;

const TILE_TYPE_RECT: u32 = 901;
const TILE_TYPE_IMAGE: u32 = 902;
const TILE_TYPE_TEXT: u32 = 903;
const TILE_TYPE_3D: u32 = 904;

/// IDA: 0058A0A0 / 00588EF0 / 0058B040 bucket rule: '&' -> 27, letters -> 1..26 (case folded), anything else -> 0. '_'
/// uses the custom
/// array. Completion: 100%.
fn token_bucket(first: u8) -> usize {
    if first == b'&' {
        return 27;
    }
    let mut index = i32::from(first as i8) - 64;
    if index > 32 {
        index -= 32;
    }
    if (index as u32) > 0x1A {
        index = 0;
    }
    index as usize
}

/// IDA: _mbsicmp on the C locale (ASCII case folding). Completion: 100% for ASCII.
fn mbsicmp(left: &[u8], right: &[u8]) -> i32 {
    stricmp(left, right)
}

/// IDA: 0058A1C0, via 0058A0A0 per entry: FormHeap 0x10 entry {id, refcount 0, name}; '&' and letters into the 28 lists
/// (AddTail), '_'
/// into the custom array. Completion: 100%.
pub fn tile_register_builtin_tokens(cx: &mut InterfaceCx) {
    for &(name, id) in &BUILTIN_TOKENS {
        if name.is_empty() {
            continue;
        }
        let entry = TileTokenEntry {
            id: id as u32,
            refcount: 0,
            name: name.as_bytes().to_vec(),
        };
        if name.starts_with('_') {
            cx.engine.custom_tokens.push(entry);
        } else {
            cx.engine.builtin_tokens[token_bucket(name.as_bytes()[0])].push(entry);
        }
    }
    cx.engine.builtins_registered = true;
}

/// IDA: 00588EF0 Tile_LookupTokenID: 0 for null, empty or unknown; a hit bumps the entry's refcount. Completion: 100%.
pub fn tile_lookup_token(cx: &mut InterfaceCx, name: &[u8]) -> u32 {
    let Some(&first) = name.first() else {
        return 0;
    };
    let list = if first == b'_' {
        &mut cx.engine.custom_tokens
    } else {
        &mut cx.engine.builtin_tokens[token_bucket(first)]
    };
    for entry in list {
        if !entry.name.is_empty() && mbsicmp(&entry.name, name) == 0 {
            entry.refcount += 1;
            return entry.id;
        }
    }
    0
}

/// IDA: 0058B040 Tile_GetOrRegisterTokenID(name, requested): a new entry takes 10000 + count when requested is -1. The
/// lookup already
/// counted the reference, like 0058B18D / 0058B173. Completion: 100%.
pub fn tile_get_or_register_token(cx: &mut InterfaceCx, name: &[u8], requested: i32) -> u32 {
    let found = tile_lookup_token(cx, name);
    if found != 0 && !name.is_empty() {
        return found;
    }
    let index = cx.engine.custom_tokens.len() as u32;
    let id = if requested == -1 {
        index + 10000
    } else {
        requested as u32
    };
    cx.engine.custom_tokens.push(TileTokenEntry {
        id,
        refcount: 0,
        name: name.to_vec(),
    });
    id
}

/// IDA: 00589080 token name by ID (null when unknown or empty). Completion: 100%.
#[must_use]
pub fn tile_token_name(cx: &InterfaceCx, id: u32) -> Option<Vec<u8>> {
    if id < 10000 {
        for bucket in &cx.engine.builtin_tokens {
            for entry in bucket {
                if entry.id == id && !entry.name.is_empty() {
                    return Some(entry.name.clone());
                }
            }
        }
        return None;
    }
    let entry = cx.engine.custom_tokens.get((id % 10000) as usize)?;
    if entry.id != id || entry.name.is_empty() {
        return None;
    }
    Some(entry.name.clone())
}

/// IDA: N/A host helper. Completion: 100%.
fn new_expression(cx: &mut InterfaceCx) -> ExprId {
    if let Some(id) = cx.engine.free_expressions.pop() {
        *cx.engine.expression_mut(id) = TileExpression::default();
        return id;
    }
    cx.engine.expressions.push(TileExpression::default());
    ExprId((cx.engine.expressions.len() - 1) as u32)
}

/// IDA: 00589DF0 TileValue_Constructor: text empty, number 0, trait, parent null, the two 0x18 heads (prev and next
/// null, operand
/// self, opcode 101, links null), isNumber 1. Completion: 100%.
fn construct_value(cx: &mut InterfaceCx, trait_id: u16) -> ValueId {
    let expression = new_expression(cx);
    let dependency = new_expression(cx);
    let value = ValueId(cx.engine.values.len() as u32);
    cx.engine.values.push(TileValue {
        parent_tile_00: None,
        number_04: 0.0,
        text_08: Vec::new(),
        expression_head_10: expression,
        dependency_head_14: dependency,
        trait_18: trait_id,
        is_number_1a: true,
    });
    for head in [expression, dependency] {
        let node = cx.engine.expression_mut(head);
        node.owner_08 = Some(value);
        node.opcode_0c = 101;
    }
    value
}

/// IDA: 00588930: every node after the expression head is unlinked from its list and its dependency chain and freed.
/// Completion: 100%.
fn clear_expressions(cx: &mut InterfaceCx, value: ValueId) {
    let head = cx.engine.value(value).expression_head_10;
    while let Some(node) = cx.engine.expression(head).next_04 {
        let TileExpression {
            prev_00,
            next_04,
            dependency_prev_10,
            dependency_next_14,
            ..
        } = cx.engine.expression(node).clone();
        if let Some(next) = next_04 {
            cx.engine.expression_mut(next).prev_00 = prev_00;
        }
        if let Some(prev) = prev_00 {
            cx.engine.expression_mut(prev).next_04 = next_04;
        }
        if let Some(prev) = dependency_prev_10 {
            cx.engine.expression_mut(prev).dependency_next_14 = dependency_next_14;
        }
        if let Some(next) = dependency_next_14 {
            cx.engine.expression_mut(next).dependency_prev_10 = dependency_prev_10;
        }
        cx.engine.free_expressions.push(node);
    }
}

/// IDA: N/A the head of a node's expression list (prev walk). Completion: 100%.
fn list_head(cx: &InterfaceCx, mut node: ExprId) -> ExprId {
    while let Some(prev) = cx.engine.expression(node).prev_00 {
        node = prev;
    }
    node
}

/// IDA: 00589770: false only for a numeric, empty-text value that is neither the string (4062) nor the filename (4070)
/// trait.
/// Completion: 100%.
fn stringish(value: &TileValue) -> bool {
    if value.trait_18 == TRAIT_STRING {
        return true;
    }
    !(value.text_08.is_empty() && value.is_number_1a && value.trait_18 != TRAIT_FILENAME)
}

/// IDA: 00517B20 (texts differ): _stricmp when both are non-null; a null side always counts as different. Completion:
/// 100%.
fn texts_differ(left: &[u8], right: &[u8]) -> bool {
    if left.is_empty() || right.is_empty() {
        return true;
    }
    stricmp(left, right) != 0
}

/// IDA: 0058BAD0 evaluation-loop guard: push the value unless it is already on the 100-entry stack; a loop logs the
/// chain and zeroes
/// every opcode of the value's list (returns true without pushing). The original overruns into 00B3B0A0 after 100
/// entries; the host
/// stops. Completion: 100%.
fn guard_evaluation(cx: &mut InterfaceCx, value: ValueId) -> Result<bool, GameError> {
    let mut index = 0;
    while index < cx.engine.evaluation_stack.len() {
        let Some(entry) = cx.engine.evaluation_stack[index] else {
            break;
        };
        if entry == value {
            cx.host.debug_log("\n*****\n");
            cx.host.debug_log("***** ERROR: Loop Detected\n");
            cx.host.debug_log("***** Loop Involves Tiles:");
            for slot in index..cx.engine.evaluation_stack.len() {
                let Some(on_stack) = cx.engine.evaluation_stack[slot] else {
                    break;
                };
                let name = cx
                    .engine
                    .value(on_stack)
                    .parent_tile_00
                    .map_or_else(String::new, |tile| {
                        super::tile::lift_text(&cx.engine.tile(tile).name_08)
                    });
                cx.host.debug_log(&format!("  {name}"));
            }
            let target = cx
                .engine
                .value(value)
                .parent_tile_00
                .map_or_else(String::new, |tile| {
                    super::tile::lift_text(&cx.engine.tile(tile).name_08)
                });
            cx.host.debug_log(&format!(
                "\n***** [ All references targeting {target} have been nullified. ]\n"
            ));
            cx.host.debug_log("*****\n\n");
            let mut node = Some(cx.engine.value(value).expression_head_10);
            while let Some(id) = node {
                let expression = cx.engine.expression_mut(id);
                expression.opcode_0c = 0;
                expression.high_0e = 0;
                node = expression.next_04;
            }
            return Ok(true);
        }
        index += 1;
    }
    // 0058BB27: stack[depth] = value, depth + 1.
    let depth = cx.engine.evaluation_depth as usize;
    if depth >= cx.engine.evaluation_stack.len() {
        return Err(GameError::unclassified(
            0x0058BAD0,
            "0058BAD0: evaluation stack overflow",
        ));
    }
    cx.engine.evaluation_stack[depth] = Some(value);
    cx.engine.evaluation_depth += 1;
    Ok(false)
}

/// IDA: 0058BDD0 propagate to dependents (only when the owning tile's +0x05 is clear): string values copy their text
/// into each
/// dependent's owner (and mark text 0x2 / filename 0x20), numeric values refresh the cached operand; then each distinct
/// dependent owner
/// is evaluated without force. Completion: 100%.
fn propagate(cx: &mut InterfaceCx, value: ValueId) -> Result<(), GameError> {
    let Some(parent) = cx.engine.value(value).parent_tile_00 else {
        return Ok(());
    };
    if cx.engine.tile(parent).unknown_05 != 0 {
        return Ok(());
    }
    let head = cx.engine.value(value).dependency_head_14;
    let mut node = cx.engine.expression(head).dependency_next_14;
    while let Some(id) = node {
        if stringish(cx.engine.value(value)) {
            let list = list_head(cx, id);
            if let Some(owner) = cx.engine.expression(list).owner_08 {
                let text = cx.engine.value(value).text_08.clone();
                if texts_differ(&cx.engine.value(owner).text_08, &text) {
                    let target = cx.engine.value_mut(owner);
                    target.text_08 = text;
                    target.number_04 = 0.0;
                    let trait_id = target.trait_18;
                    if let Some(tile) = target.parent_tile_00 {
                        if cx.engine.tile(tile).type_id() == TILE_TYPE_TEXT {
                            cx.engine.tile_mut(tile).flags_2c |= 2;
                        }
                        if trait_id == TRAIT_FILENAME {
                            cx.engine.tile_mut(tile).flags_2c |= 0x20;
                        }
                    }
                }
            }
        } else {
            let number = cx.engine.value(value).number_04;
            cx.engine.expression_mut(id).number_08 = number;
        }
        node = cx.engine.expression(id).dependency_next_14;
    }
    // 0058BE9B..0058BECF: the next link is read after the evaluation.
    let mut previous = None;
    let mut node = cx.engine.expression(head).dependency_next_14;
    while let Some(id) = node {
        let mut owner = None;
        let expression = cx.engine.expression(id);
        if expression.opcode_0c != 0 || expression.high_0e != 0 {
            let list = list_head(cx, id);
            owner = cx.engine.expression(list).owner_08;
            if let Some(target) = owner
                && owner != previous
            {
                tile_value_evaluate(cx, target, false)?;
            }
        }
        previous = owner;
        node = cx.engine.expression(id).dependency_next_14;
    }
    Ok(())
}

/// IDA: 0058B2F0 generic trait change (when the kind's UpdateField returns 0), and called directly with the last
/// argument 0 by the vt+8
/// builders (0059220D, 0059222B, 00591A17). x/y 1 (+0x100 when clipwindow 4004 is 2), depth (menu +0x18 max over the
/// locus chain), text
/// wrap traits 2, image and rect width, height and crop 0x10 (+0x100), clipwindow 0x100, visible and 4003 4, alpha and
/// RGB 8, clips 4040
/// 0x200, filename 0x20 (image) / 0x40 (3D), id -> Menu vt+4. Completion: 100% of the flags.
#[allow(clippy::assign_op_pattern)] // 0058B562.. the operand order: the ancestor's depth first.
pub fn tile_generic_update(cx: &mut InterfaceCx, tile: TileId, trait_id: u16, number: f32) {
    let type_id = cx.engine.tile(tile).type_id();
    let mark_layout = |cx: &mut InterfaceCx| {
        if tile_numeric_trait(cx, tile, TRAIT_CLIP_WINDOW) == 2.0 {
            cx.engine.tile_mut(tile).flags_2c |= 0x100;
        }
    };
    if trait_id == TRAIT_X || trait_id == TRAIT_Y {
        mark_layout(cx);
        cx.engine.tile_mut(tile).flags_2c |= 1;
        return;
    }
    if trait_id == TRAIT_DEPTH {
        // 0058B562..0058B5EB: add the depth of every locus (4006 == 2.0) ancestor below the root, then raise the
        // containing menu's +0x18.
        let mut depth = number;
        if let Some(mut parent) = cx.engine.tile(tile).parent {
            let mut current = tile;
            while let Some(grand) = cx.engine.tile(parent).parent {
                if find_value(cx, current, TRAIT_LOCUS)
                    .is_some_and(|locus| cx.engine.value(locus).number_04 == 2.0)
                {
                    let own = find_value(cx, current, TRAIT_DEPTH)
                        .map_or(0.0, |own| cx.engine.value(own).number_04);
                    depth = own + depth;
                }
                current = parent;
                parent = grand;
            }
        }
        let menu = cx.engine.containing_menu(tile);
        let mut top = tile;
        while let Some(parent) = cx.engine.tile(top).parent {
            if cx.engine.tile(parent).parent.is_none() {
                break;
            }
            top = parent;
        }
        // 0058B61F..0058B634: fild +0x18 (exact, not rounded to 24 bits), raised to ftol(depth) when below.
        if let Some(menu) = menu
            && top != tile
            && f64::from(cx.engine.menu(menu).depth_18) < f64::from(depth)
        {
            cx.engine.menu_mut(menu).depth_18 = x87::ftol(depth);
        }
        mark_layout(cx);
        cx.engine.tile_mut(tile).flags_2c |= 1;
        return;
    }
    if type_id == TILE_TYPE_TEXT
        && matches!(
            trait_id,
            TRAIT_WRAP_WIDTH
                | TRAIT_WRAP_LIMIT
                | TRAIT_WRAP_LINES
                | TRAIT_PAGE_NUM
                | TRAIT_IS_HTML
                | TRAIT_FONT
        )
    {
        // 0xFD4..0xFD7 wrapwidth..pagenum, 0xFD8 ishtml, 0xFD3 font.
        cx.engine.tile_mut(tile).flags_2c |= 2;
        return;
    }
    if matches!(
        trait_id,
        TRAIT_WIDTH | TRAIT_HEIGHT | TRAIT_CROP_X | TRAIT_CROP_Y
    ) && (type_id == TILE_TYPE_IMAGE || type_id == TILE_TYPE_RECT)
    {
        mark_layout(cx);
        cx.engine.tile_mut(tile).flags_2c |= 0x10;
        return;
    }
    if trait_id == TRAIT_CLIP_WINDOW {
        if type_id == TILE_TYPE_IMAGE || type_id == TILE_TYPE_RECT {
            cx.engine.tile_mut(tile).flags_2c |= 0x100;
        }
        return;
    }
    if trait_id == TRAIT_CLIPS {
        if type_id == TILE_TYPE_IMAGE || type_id == TILE_TYPE_RECT || type_id == TILE_TYPE_TEXT {
            cx.engine.tile_mut(tile).flags_2c |= 0x200;
        }
        return;
    }
    if trait_id == TRAIT_VISIBLE || trait_id == TRAIT_LIST_CLIP {
        cx.engine.tile_mut(tile).flags_2c |= 4;
        return;
    }
    if matches!(trait_id, TRAIT_ALPHA | TRAIT_RED | TRAIT_GREEN | TRAIT_BLUE) {
        cx.engine.tile_mut(tile).flags_2c |= 8;
        return;
    }
    if trait_id == TRAIT_FILENAME {
        let named = tile_string_trait(cx, tile, TRAIT_FILENAME).is_some();
        if type_id == TILE_TYPE_IMAGE && named {
            cx.engine.tile_mut(tile).flags_2c |= 0x20;
        } else if type_id == TILE_TYPE_3D && named {
            cx.engine.tile_mut(tile).flags_2c |= 0x40;
        }
        return;
    }
    if trait_id == TRAIT_ID
        && let Some(menu) = cx.engine.containing_menu(tile)
    {
        // vt+4 SetField: nullsub_1 for Menu and LoadingMenu.
        crate::tes::menus::menu_set_field(cx, menu, x87::ftol(number), tile);
    }
}

/// IDA: vt+0x14 UpdateField per kind: TileRect/Tile 00592A60 (0), TileImage 00590F30 (x/y flag 1, width/height flag
/// 0x10, then 0), TileText
/// 005922E0 (string: flag 2 when the stored text is empty or equal; justify against itself), TileMenu 00591C50 (class:
/// Menu_CreateByID and
/// the registry). Returns true where the original returns a non-null tile. Completion: 100%.
fn update_field(
    cx: &mut InterfaceCx,
    tile: TileId,
    trait_id: u16,
    number: f32,
    text: &[u8],
) -> Result<bool, GameError> {
    match cx.engine.tile(tile).kind {
        // 00590720: trait 4062 clears the last animation's name at +0x48, with no model to play it here.
        TileKind::Rect | TileKind::Model => Ok(false),
        TileKind::Image => {
            let flags = &mut cx.engine.tile_mut(tile).flags_2c;
            if (trait_id == TRAIT_X || trait_id == TRAIT_Y) && *flags & 1 == 0 {
                *flags |= 1;
            }
            if (trait_id == TRAIT_WIDTH || trait_id == TRAIT_HEIGHT) && *flags & 0x10 == 0 {
                *flags |= 0x10;
            }
            Ok(false)
        }
        TileKind::Text => {
            if trait_id == TRAIT_STRING {
                let current = tile_string_trait(cx, tile, TRAIT_STRING);
                let incoming = (!text.is_empty()).then_some(text);
                let equal = match (current.as_deref(), incoming) {
                    (None, Some(_)) => true,
                    (Some(stored), Some(new)) => stored == new,
                    // _mbscmp(null, current) crashes in the original; host: unequal.
                    _ => false,
                };
                if equal {
                    cx.engine.tile_mut(tile).flags_2c |= 2;
                    return Ok(true);
                }
                return Ok(false);
            }
            if trait_id == TRAIT_JUSTIFY && number != tile_numeric_trait(cx, tile, TRAIT_JUSTIFY) {
                cx.engine.tile_mut(tile).flags_2c |= 2;
                return Ok(true);
            }
            Ok(false)
        }
        TileKind::Menu => {
            if trait_id != TRAIT_CLASS {
                return Ok(false);
            }
            let menu_id = x87::ftol(number) as u32;
            // 00591C6C 00587B20(1), 00591C76 Menu_CreateByID, +0x44.
            cx.engine.menu_array_allocated_b3a708 = true;
            let Some(class) = menu_create_by_id(cx, menu_id)? else {
                cx.engine.tile_mut(tile).menu_44 = None;
                // 00591C89 ConsoleManager message "ERROR: Cant find menu class!".
                cx.host.debug_log("ERROR: Cant find menu class!");
                return Ok(false);
            };
            let created = super::tile::MenuId(cx.engine.menus.len() as u32);
            cx.engine.menus.push(Menu::new(class));
            cx.engine.tile_mut(tile).menu_44 = Some(created);
            // 00591CA5 Menu_RegisterTile, +0x20 = vt+0x34 GetID. The C++ port stores only +0x04 here: the disassembly
            // calls the whole of 00584880. The stacking trait is not set yet in the shipped loading_menu.xml, so its
            // result is the same.
            menu_register_tile(cx, created, Some(tile))?;
            let menu = cx.engine.menu_mut(created);
            menu.id_20 = menu.id();
            let stacking = tile_numeric_trait(cx, tile, TRAIT_STACKING);
            if stacking == 102.0 || stacking == 103.0 {
                let menu = cx.engine.menu_mut(created);
                menu.menu_mode_14 = menu.id();
                let depth = menu_update_cursor_depth(cx)?;
                tile_set_numeric_trait(cx, tile, TRAIT_DEPTH, depth)?;
            }
            // 00591D24 NiTArray_AddAtIndex(g_tileMenuArray, id - 1001). The movie thread's atomic is stored with the
            // slot.
            let slot = menu_id.wrapping_sub(1001) as usize;
            if slot < cx.engine.tile_menus.len() {
                cx.engine.tile_menus[slot] = Some(tile);
                if let Some(reads) = cx.world.get_resource::<InterfaceMovieReads>() {
                    reads.set_tile_menu(menu_id, true);
                }
            }
            Ok(true)
        }
    }
}

/// IDA: the lookup in 0058B220 and 00588BD0: the tile's value for the trait. Completion: 100%.
#[must_use]
pub fn find_value(cx: &InterfaceCx, tile: TileId, trait_id: u16) -> Option<ValueId> {
    cx.engine.find_value(tile, trait_id)
}

/// IDA: 0058B220 Tile_GetOrCreateValue with 00589DF0 (expression and dependency heads, opcode 101, isNumber 1): the
/// +0x14 list stays in
/// trait order. Completion: 100%.
pub fn tile_get_or_create_value(cx: &mut InterfaceCx, tile: TileId, trait_id: u16) -> ValueId {
    let mut index = 0;
    let count = cx.engine.tile(tile).values_14.len();
    while index < count {
        let value = cx.engine.tile(tile).values_14[index];
        let stored = cx.engine.value(value).trait_18;
        if stored == trait_id {
            return value;
        }
        if stored > trait_id {
            break;
        }
        index += 1;
    }
    let value = construct_value(cx, trait_id);
    cx.engine.value_mut(value).parent_tile_00 = Some(tile);
    cx.engine.tile_mut(tile).values_14.insert(index, value);
    value
}

/// IDA: 00588BD0 Tile_GetNumericTraitOrZero. Completion: 100%.
#[must_use]
pub fn tile_numeric_trait(cx: &InterfaceCx, tile: TileId, trait_id: u16) -> f32 {
    cx.engine.numeric_trait(tile, trait_id)
}

/// IDA: 00588C10: the text of a found value (`None`, the null pointer, when its BSStringT is empty), or the shared
/// empty default 00A5A1D0
/// for a missing value. Completion: 100%.
#[must_use]
pub fn tile_string_trait(cx: &InterfaceCx, tile: TileId, trait_id: u16) -> Option<Vec<u8>> {
    let Some(value) = find_value(cx, tile, trait_id) else {
        return Some(Vec::new());
    };
    let text = &cx.engine.value(value).text_08;
    (!text.is_empty()).then(|| text.clone())
}

/// IDA: 0058CA00 TileValue_SetNumber. Completion: 100%.
pub fn tile_value_set_number(
    cx: &mut InterfaceCx,
    value: ValueId,
    number: f32,
) -> Result<(), GameError> {
    let target = cx.engine.value_mut(value);
    let previous = target.number_04;
    target.is_number_1a = true;
    if number != previous || target.trait_18 == TRAIT_STRING {
        target.text_08.clear();
        target.number_04 = number;
        clear_expressions(cx, value);
        tile_value_evaluate(cx, value, true)?;
    }
    Ok(())
}

/// IDA: 0058CA50 TileValue_SetString (an empty string becomes " "; strcmp is case-sensitive). Completion: 100%.
pub fn tile_value_set_string(
    cx: &mut InterfaceCx,
    value: ValueId,
    text: &[u8],
) -> Result<(), GameError> {
    let target = cx.engine.value_mut(value);
    let was_number = target.is_number_1a;
    target.is_number_1a = false;
    let source: &[u8] = if text.is_empty() { b" " } else { text };
    let differs = target.text_08.is_empty() || target.text_08 != source;
    if was_number || differs {
        target.text_08 = source.to_vec();
        target.number_04 = 0.0;
        clear_expressions(cx, value);
        tile_value_evaluate(cx, value, true)?;
    }
    Ok(())
}

/// IDA: 0058CEB0 Tile_SetNumericTrait. Completion: 100%.
pub fn tile_set_numeric_trait(
    cx: &mut InterfaceCx,
    tile: TileId,
    trait_id: u16,
    number: f32,
) -> Result<(), GameError> {
    let value = tile_get_or_create_value(cx, tile, trait_id);
    tile_value_set_number(cx, value, number)
}

/// IDA: 0058CED0 Tile_SetStringTrait. Completion: 100%.
pub fn tile_set_string_trait(
    cx: &mut InterfaceCx,
    tile: TileId,
    trait_id: u16,
    text: &[u8],
) -> Result<(), GameError> {
    let value = tile_get_or_create_value(cx, tile, trait_id);
    tile_value_set_string(cx, value, text)
}

/// IDA: N/A host helper. Completion: 100%.
fn append_at_tail(cx: &mut InterfaceCx, value: ValueId, node: ExprId) {
    let mut tail = cx.engine.value(value).expression_head_10;
    while let Some(next) = cx.engine.expression(tail).next_04 {
        tail = next;
    }
    cx.engine.expression_mut(node).prev_00 = Some(tail);
    cx.engine.expression_mut(tail).next_04 = Some(node);
}

/// IDA: 0058CB70 TileValue_AppendActionMarker(opcode, marker): a node with operand = the action opcode and opcode = 10
/// (begin) / 15 (end),
/// then 0058BEE0(0). Completion: 100%.
pub fn tile_value_append_marker(
    cx: &mut InterfaceCx,
    value: ValueId,
    action: u32,
    marker: u16,
) -> Result<ExprId, GameError> {
    let node = new_expression(cx);
    let expression = cx.engine.expression_mut(node);
    expression.opcode_0c = marker;
    expression.action_08 = action;
    append_at_tail(cx, value, node);
    tile_value_evaluate(cx, value, false)?;
    Ok(node)
}

/// IDA: 0058CBE0 TileValue_AppendConstantAction. Completion: 100%.
pub fn tile_value_append_constant(
    cx: &mut InterfaceCx,
    value: ValueId,
    operand: f32,
    opcode: u16,
) -> Result<ExprId, GameError> {
    let node = new_expression(cx);
    let expression = cx.engine.expression_mut(node);
    expression.opcode_0c = opcode;
    expression.number_08 = operand;
    append_at_tail(cx, value, node);
    tile_value_evaluate(cx, value, false)?;
    Ok(node)
}

/// IDA: 0058CC60 TileValue_AppendReferenceAction(source, trait, opcode): only a source trait below 4001 is rejected
/// (58cc66, a signed `jl`). The next limit, `58cc76 cmp ebp, ds:0xb13bc0`, is the dword 00B13BC0, 10000 in the file and
/// never written, so `58cc7e cmp ebp, 0x2710; jl` cannot reject: the C++ differs, reading 00B13BD4 (4098) and rejecting
/// 4098..9999. The head is reset to 101; the node caches the source number and joins the source value's dependency chain
/// at its end; then 0058BEE0(0). Completion: 100%.
pub fn tile_value_append_reference(
    cx: &mut InterfaceCx,
    value: ValueId,
    source: TileId,
    source_trait: u32,
    opcode: u16,
) -> Result<Option<ExprId>, GameError> {
    if (source_trait as i32) < 4001 {
        return Ok(None);
    }
    let head = cx.engine.value(value).expression_head_10;
    cx.engine.expression_mut(head).opcode_0c = 101;
    let node = new_expression(cx);
    let cached = tile_numeric_trait(cx, source, source_trait as u16);
    let expression = cx.engine.expression_mut(node);
    expression.number_08 = cached;
    expression.opcode_0c = opcode;
    append_at_tail(cx, value, node);
    let source_value = tile_get_or_create_value(cx, source, source_trait as u16);
    let mut last = cx.engine.value(source_value).dependency_head_14;
    while let Some(next) = cx.engine.expression(last).dependency_next_14 {
        last = next;
    }
    cx.engine.expression_mut(last).dependency_next_14 = Some(node);
    cx.engine.expression_mut(node).dependency_prev_10 = Some(last);
    tile_value_evaluate(cx, value, false)?;
    Ok(Some(node))
}

/// IDA: 0058C063..0058C30F: indexed custom traits ("_name_" or a numeric suffix that differs from the current number)
/// redirect the node to
/// "<prefix><int(number)>" on the source tile. Returns the value and operand the action then uses. Completion: 100%.
fn redirect_indexed_trait(
    cx: &mut InterfaceCx,
    self_value: ValueId,
    node: ExprId,
    value: ValueId,
    source_tile: Option<TileId>,
    operand: f32,
) -> Result<(ValueId, f32), GameError> {
    let mut name = Vec::new();
    let mut suffix = 0;
    let trait_id = cx.engine.value(value).trait_18;
    if trait_id >= 10000 {
        if let Some(found) = tile_token_name(cx, u32::from(trait_id)) {
            name = found;
        }
        if !name.is_empty() {
            let mut k = name.len() - 1;
            let mut digits = vec![b' '; name.len()];
            while k != 0 && name[k] != b'_' {
                digits[k] = name[k];
                k -= 1;
            }
            if k != 0 {
                suffix = atol(&digits);
            }
        }
    }
    let number = cx.engine.value(self_value).number_04;
    if name.is_empty()
        || number == 0.0
        || !(name.last() == Some(&b'_') || (suffix != 0 && number != suffix as f32))
    {
        return Ok((value, operand));
    }
    let mut prefix = name.clone();
    let mut cut = 2;
    if name.len() > 2 {
        while cut < name.len() && name[cut - 1] != b'_' {
            cut += 1;
        }
    }
    if name.get(cut - 1) == Some(&b'_') {
        prefix.truncate(cut);
    }
    prefix.extend_from_slice(x87::ftol(number).to_string().as_bytes());
    cx.engine.value_mut(self_value).number_04 = 0.0;
    let redirected = tile_get_or_register_token(cx, &prefix, -1) as u16;
    let mut target = source_tile.and_then(|tile| find_value(cx, tile, redirected));
    if target.is_none()
        && let Some(tile) = source_tile
    {
        let created = tile_get_or_create_value(cx, tile, redirected);
        tile_value_set_number(cx, created, 0.0)?;
        target = find_value(cx, tile, redirected);
    }
    let Some(target) = target else {
        return Ok((value, operand));
    };
    let links = cx.engine.expression(node);
    let (prev, next) = (links.dependency_prev_10, links.dependency_next_14);
    if let Some(prev) = prev {
        if let Some(next) = next {
            cx.engine.expression_mut(next).dependency_prev_10 = Some(prev);
            cx.engine.expression_mut(prev).dependency_next_14 = Some(next);
        } else {
            cx.engine.expression_mut(prev).dependency_next_14 = None;
        }
        let expression = cx.engine.expression_mut(node);
        expression.dependency_prev_10 = None;
        expression.dependency_next_14 = None;
    } else if let Some(next) = next {
        cx.engine.expression_mut(next).dependency_prev_10 = None;
        cx.engine.expression_mut(node).dependency_next_14 = None;
    }
    let mut last = cx.engine.value(target).dependency_head_14;
    while let Some(following) = cx.engine.expression(last).dependency_next_14 {
        last = following;
    }
    cx.engine.expression_mut(last).dependency_next_14 = Some(node);
    let number = cx.engine.value(target).number_04;
    let expression = cx.engine.expression_mut(node);
    expression.dependency_prev_10 = Some(last);
    expression.number_08 = number;
    Ok((target, number))
}

/// IDA: 0058BEE0 TileValue_Evaluate's action switch. Unordered compares keep the C++ port's arms. Completion: 100% of
/// the action set.
/// TODO: [ ] 2023 and 2024: the last bit of the host's `ln` and `log10` against 00996E80 and 009970F0 (the CRT's SSE2 paths,
/// taken at this control word, 986ba8 `and ax, 0x7F`); no Phase 07 tile uses them.
#[allow(clippy::too_many_lines, clippy::assign_op_pattern)] // 0058BEE0 action 2018: the multiple first.
fn apply_action(
    cx: &mut InterfaceCx,
    self_value: ValueId,
    node: ExprId,
    action: u32,
    value: ValueId,
    operand: f32,
) {
    let p = cx.engine.value(self_value).number_04;
    let result = match action {
        // Copy, only after the head.
        2001 => {
            if cx.engine.expression(node).prev_00.is_none() {
                return;
            }
            let Some(tile) = cx.engine.value(self_value).parent_tile_00 else {
                return;
            };
            let source_text = cx.engine.value(value).text_08.clone();
            let own = cx.engine.value(self_value);
            let differs = texts_differ(&own.text_08, &source_text);
            let trait_id = own.trait_18;
            let type_id = cx.engine.tile(tile).type_id();
            if trait_id == TRAIT_STRING && type_id == TILE_TYPE_TEXT && differs {
                cx.engine.tile_mut(tile).flags_2c |= 2;
            }
            if trait_id == TRAIT_FILENAME && type_id == TILE_TYPE_IMAGE && differs {
                cx.engine.tile_mut(tile).flags_2c |= 0x20;
            }
            cx.engine.value_mut(self_value).text_08 = source_text;
            operand
        }
        2002 => operand + p,
        2003 => p - operand,
        2004 => operand * p,
        2005 => {
            if operand == 0.0 {
                p
            } else {
                p / operand
            }
        }
        2006 => {
            // fild, fdiv 32767.0 (A3D5A8), fmul: float results (24-bit).
            let random = cx.host.random_next();
            let scaled = random as f32 / 32767.0 * operand;
            scaled.floor() + 1.0
        }
        2026 => {
            if operand == 2.0 {
                1.0
            } else {
                2.0
            }
        }
        2008 => {
            if operand < p {
                2.0
            } else {
                1.0
            }
        }
        2009 => {
            if operand <= p {
                2.0
            } else {
                1.0
            }
        }
        2010 => {
            if operand == p {
                2.0
            } else {
                1.0
            }
        }
        2011 => {
            if operand < p || operand.is_nan() || p.is_nan() {
                1.0
            } else {
                2.0
            }
        }
        2012 => {
            if operand > p {
                2.0
            } else {
                1.0
            }
        }
        2013 => {
            if operand <= p || operand.is_nan() || p.is_nan() {
                operand
            } else {
                p
            }
        }
        2014 => {
            if operand < p {
                p
            } else {
                operand
            }
        }
        2015 => {
            if p == 2.0 && operand == 2.0 {
                2.0
            } else {
                1.0
            }
        }
        2016 => {
            if p == 2.0 || operand == 2.0 {
                2.0
            } else {
                1.0
            }
        }
        2017 => {
            if operand == p {
                1.0
            } else {
                2.0
            }
        }
        2018 => {
            let mut wrapped = p;
            if wrapped < 0.0 {
                let magnitude = wrapped.abs();
                let ratio = magnitude / operand;
                let multiple = operand * (x87::ftol(ratio).wrapping_add(1)) as f32;
                wrapped = multiple + wrapped;
            }
            x87::float_fmod(wrapped, operand)
        }
        2019 => (operand + p).floor(),
        2025 => (operand + p).ceil(),
        2020 => {
            if p < 0.0 {
                (operand + p).abs()
            } else {
                p
            }
        }
        2021 => {
            if operand == 2.0 {
                p
            } else {
                0.0
            }
        }
        2022 => {
            if operand == 1.0 {
                p
            } else {
                0.0
            }
        }
        // 58c7e3 00986B80 is `fldln2; fyl2x`, 58c802 00986CD0 `fldlg2; fyl2x`: the table at 0058C998 sends 2023 to the
        // first. Their double result is stored as a float.
        2023 => f64::from(p).ln() as f32,
        2024 => f64::from(p).log10() as f32,
        2027 | 0 | 35 | 101 => return,
        _ => {
            cx.host
                .warning(&format!("ERROR: Unknown action enumeration {action} \n"));
            return;
        }
    };
    cx.engine.value_mut(self_value).number_04 = result;
}

/// IDA: 0058BEE0 TileValue_Evaluate(forceUpdate). Action semantics from the disassembly (the Hex-Rays view drops the
/// ceil store and the
/// ftol). Completion: 100% of the action set.
pub fn tile_value_evaluate(
    cx: &mut InterfaceCx,
    self_value: ValueId,
    force_update: bool,
) -> Result<(), GameError> {
    let own = cx.engine.value(self_value);
    let (trait_id, parent) = (own.trait_18, own.parent_tile_00);
    if cx.engine.unterminated_tile && trait_id != TRAIT_CLASS {
        return Ok(());
    }
    let Some(parent) = parent else {
        return Ok(());
    };
    if cx.engine.tile(parent).unknown_05 != 0 {
        return Ok(());
    }
    if guard_evaluation(cx, self_value)? {
        return Ok(());
    }
    let before = cx.engine.value(self_value).number_04;
    // NiTList<float>: push and pop at the head.
    let mut stack: Vec<f32> = Vec::new();
    let head = cx.engine.value(self_value).expression_head_10;
    let mut node = cx.engine.expression(head).next_04;
    while let Some(id) = node {
        let expression = cx.engine.expression(id);
        let mut action = u32::from(expression.opcode_0c) | (u32::from(expression.high_0e) << 16);
        let mut operand = expression.number_08;
        let marker_action = expression.action_08;
        // Source value: the dependency chain's head owner, else the list head's.
        let mut chain = id;
        while let Some(link) = cx.engine.expression(chain).dependency_prev_10 {
            chain = link;
        }
        let list = list_head(cx, chain);
        let value = cx.engine.expression(list).owner_08;
        let source_tile = value.and_then(|value| cx.engine.value(value).parent_tile_00);
        let mut apply = true;
        if action == 15 && !stack.is_empty() {
            action = marker_action;
            operand = cx.engine.value(self_value).number_04;
            cx.engine.value_mut(self_value).number_04 = stack.pop().expect("checked non-empty");
        } else if action == 10 {
            stack.push(cx.engine.value(self_value).number_04);
            apply = false;
        }
        if apply && let Some(value) = value {
            let (value, operand) =
                redirect_indexed_trait(cx, self_value, id, value, source_tile, operand)?;
            apply_action(cx, self_value, id, action, value, operand);
        }
        node = cx.engine.expression(id).next_04;
    }
    while let Some(saved) = stack.pop() {
        cx.engine.value_mut(self_value).number_04 = saved;
    }
    // 0058C898: a string trait with actions whose number changed is reprinted.
    let number = cx.engine.value(self_value).number_04;
    if cx.engine.expression(head).next_04.is_some() && trait_id == TRAIT_STRING && before != number
    {
        cx.engine.value_mut(self_value).text_08 = x87::ftol(number).to_string().into_bytes();
    }
    if before != number || stringish(cx.engine.value(self_value)) || force_update {
        propagate(cx, self_value)?;
        if let Some(tile) = cx.engine.value(self_value).parent_tile_00
            && cx.engine.tile(tile).released_04 == 0
        {
            let own = cx.engine.value(self_value);
            let (number, text) = (own.number_04, own.text_08.clone());
            if !update_field(cx, tile, trait_id, number, &text)? {
                tile_generic_update(cx, tile, trait_id, number);
            }
        }
    }
    cx.engine.evaluation_depth -= 1;
    let depth = cx.engine.evaluation_depth as usize;
    cx.engine.evaluation_stack[depth] = None;
    Ok(())
}

/// IDA: 00582160(0, 1) +0x68 menuRoot. Completion: 100%.
#[must_use]
pub fn interface_menu_root(cx: &InterfaceCx) -> Option<TileId> {
    cx.world
        .get_resource::<InterfaceManager>()
        .and_then(|manager| manager.menu_root_068)
}

/// IDA: 00589B10: the case-insensitive named search, depth first. Completion: 100%.
fn search_named(cx: &InterfaceCx, tile: TileId, target: &[u8]) -> Option<TileId> {
    let own = cx.engine.tile(tile);
    if !own.name_08.is_empty() && mbsicmp(&own.name_08, target) == 0 {
        return Some(tile);
    }
    own.children_30
        .iter()
        .find_map(|&child| search_named(cx, child, target))
}

/// IDA: 00589930 Tile_GetChildByName: the child by name (case-sensitive), depth first. Completion: 100%.
#[must_use]
pub fn tile_child_named(cx: &InterfaceCx, tile: TileId, target: &[u8]) -> Option<TileId> {
    child_named(cx, tile, target)
}

// IDA: 00589930, as `tile_child_named`.
fn child_named(cx: &InterfaceCx, tile: TileId, target: &[u8]) -> Option<TileId> {
    for &child in &cx.engine.tile(tile).children_30 {
        if cx.engine.tile(child).name_08 == target {
            return Some(child);
        }
        if let Some(found) = child_named(cx, child, target) {
            return Some(found);
        }
    }
    None
}

/// IDA: 0058B800 Tile_ResolveSource with 00588FC0 (name and argument split), 00589B10 and 00589930. Completion: 100%.
pub fn tile_resolve_source(
    cx: &mut InterfaceCx,
    context: Option<TileId>,
    source: &[u8],
) -> Option<TileId> {
    // 00588FC0: up to 255 characters; '(' starts the argument, ')' ends.
    let mut name = Vec::new();
    let mut argument: Vec<u8> = Vec::new();
    // 588ff3 esi: -1, and 0 again at each '(' (589015), which writes over the earlier argument from its start.
    let mut argument_at: Option<usize> = None;
    for &c in source.iter().take(255) {
        if c == 0 || c == b')' {
            break;
        }
        if c == b'(' {
            argument_at = Some(0);
        } else if let Some(at) = &mut argument_at {
            if *at < argument.len() {
                argument[*at] = c;
            } else {
                argument.push(c);
            }
            *at += 1;
        } else {
            name.push(c);
        }
    }
    let token = tile_lookup_token(cx, &name);
    match token {
        5001 => return context.and_then(|tile| cx.engine.tile(tile).parent),
        5002 => return context,
        5004 => {
            let parent = context.and_then(|tile| cx.engine.tile(tile).parent)?;
            let siblings = &cx.engine.tile(parent).children_30;
            let first = *siblings.first()?;
            if argument.is_empty() {
                let position = siblings.iter().position(|&tile| Some(tile) == context);
                return Some(position.map_or(first, |index| {
                    siblings.get(index + 1).copied().unwrap_or(first)
                }));
            }
            return siblings.iter().copied().find(|&sibling| {
                let name = &cx.engine.tile(sibling).name_08;
                !name.is_empty() && mbsicmp(name, &argument) == 0
            });
        }
        5005 => {
            let context = context?;
            let first = *cx.engine.tile(context).children_30.first()?;
            if argument.is_empty() {
                return Some(first);
            }
            return child_named(cx, context, &argument);
        }
        5006 => return interface_menu_root(cx),
        5007 => {
            return cx
                .world
                .get_resource::<InterfaceManager>()
                .and_then(|manager| manager.strings_06c);
        }
        107 => {
            let menu = context.and_then(|tile| cx.engine.containing_menu(tile))?;
            return cx.engine.menu(menu).unknown_10;
        }
        _ => {}
    }
    if let Some(context) = context {
        let mut top = context;
        while let Some(parent) = cx.engine.tile(top).parent {
            if cx.engine.tile(parent).parent.is_none() {
                break;
            }
            top = parent;
        }
        return search_named(cx, top, source);
    }
    interface_menu_root(cx).and_then(|root| search_named(cx, root, source))
}

/// IDA: 0058D1C0 (parent, after). A released old parent (58d1cc `cmp byte [edi+4], 0; jne 58d21b`) skips both the 0xFD0
/// count and the list removal: the tile stays in that parent's list. Otherwise the count minus 1, then 00776690 removes
/// the tile. 58d221 stores +0x10 next, before the new parent's 0xFD0 count plus 1 at 58d25a, so that evaluation already
/// sees the new parent: the C++ differs, storing after. The children list (+0x30) is walked from +0x34 through each
/// node's +0: an `after` that is a child removes the tile from the list (00776690) and inserts after it (005986D0);
/// otherwise 006AA320 adds at the front. Completion: 100% of the count updates, the store order and list order.
pub fn tile_set_parent(
    cx: &mut InterfaceCx,
    tile: TileId,
    parent: Option<TileId>,
    after: Option<TileId>,
) -> Result<(), GameError> {
    if let Some(old_parent) = cx.engine.tile(tile).parent
        && cx.engine.tile(old_parent).released_04 == 0
    {
        let count = tile_numeric_trait(cx, old_parent, TRAIT_CHILD_COUNT) - 1.0;
        let value = tile_get_or_create_value(cx, old_parent, TRAIT_CHILD_COUNT);
        tile_value_set_number(cx, value, count)?;
        cx.engine
            .tile_mut(old_parent)
            .children_30
            .retain(|&child| child != tile);
    }
    // 58d221.
    cx.engine.tile_mut(tile).parent = parent;
    let Some(parent) = parent else {
        return Ok(());
    };
    let count = tile_numeric_trait(cx, parent, TRAIT_CHILD_COUNT) + 1.0;
    let value = tile_get_or_create_value(cx, parent, TRAIT_CHILD_COUNT);
    tile_value_set_number(cx, value, count)?;
    let is_child = after.is_some_and(|after| cx.engine.tile(parent).children_30.contains(&after));
    let siblings = &mut cx.engine.tile_mut(parent).children_30;
    if let (Some(after), true) = (after, is_child) {
        // 58d29a 00776690, then 58d2bb 005986D0.
        siblings.retain(|&child| child != tile);
        let index = siblings.iter().position(|&child| child == after);
        siblings.insert(index.map_or(0, |index| index + 1), tile);
    } else {
        // 58d28e / 58d2d6 006AA320.
        siblings.insert(0, tile);
    }
    Ok(())
}

/// IDA: 0058DA00 Tile_Initialize(parent, name, after) and the kind's vt+4: TileRect 00591D40 (RGB 255, alpha 0),
/// TileImage 00590F80 (RGB 255,
/// alpha 255, +0x40 1.0, +0x44 null, +0x48 0), TileText 00592250 (string " ", RGB 117/59/33, alpha 255), TileMenu
/// 00590110 (+0x44 null, after
/// 0). The tokens are registered at the end of the first Tile_Initialize. Completion: 100%.
pub fn tile_initialize(
    cx: &mut InterfaceCx,
    kind: TileKind,
    parent: Option<TileId>,
    name: Option<&[u8]>,
    after: Option<TileId>,
) -> Result<TileId, GameError> {
    let tile = TileId(cx.engine.tiles.len() as u32);
    cx.engine.tiles.push(super::tile::Tile {
        kind,
        released_04: 0,
        unknown_05: 0,
        unknown_06: 0,
        name_08: Vec::new(),
        parent: None,
        values_14: Vec::new(),
        scene_24: None,
        model_path_50: None,
        flags_2c: 2,
        children_30: Vec::new(),
        scissor_40: 0,
        menu_44: None,
        image_scale_40: 1.0,
        image_override_44: None,
        byte_48: 0,
        transitions_28: Vec::new(),
        geometry_translate_44: [0.0; 3],
        html_50: 0,
    });
    let after = if kind == TileKind::Menu { None } else { after };
    if parent.is_some() {
        tile_set_parent(cx, tile, parent, after)?;
    }
    if let Some(name) = name {
        cx.engine.tile_mut(tile).name_08 = name.to_vec();
    }
    if !cx.engine.builtins_registered {
        tile_register_builtin_tokens(cx);
    }
    match kind {
        TileKind::Rect => {
            tile_set_numeric_trait(cx, tile, TRAIT_RED, 255.0)?;
            tile_set_numeric_trait(cx, tile, TRAIT_GREEN, 255.0)?;
            tile_set_numeric_trait(cx, tile, TRAIT_BLUE, 255.0)?;
            tile_set_numeric_trait(cx, tile, TRAIT_ALPHA, 0.0)?;
        }
        // 005905F0 for Tile3D stores the same four as TileImage's.
        TileKind::Image | TileKind::Model => {
            tile_set_numeric_trait(cx, tile, TRAIT_RED, 255.0)?;
            tile_set_numeric_trait(cx, tile, TRAIT_GREEN, 255.0)?;
            tile_set_numeric_trait(cx, tile, TRAIT_BLUE, 255.0)?;
            tile_set_numeric_trait(cx, tile, TRAIT_ALPHA, 255.0)?;
        }
        TileKind::Text => {
            tile_set_string_trait(cx, tile, TRAIT_STRING, b" ")?;
            tile_set_numeric_trait(cx, tile, TRAIT_RED, 117.0)?;
            tile_set_numeric_trait(cx, tile, TRAIT_GREEN, 59.0)?;
            tile_set_numeric_trait(cx, tile, TRAIT_BLUE, 33.0)?;
            tile_set_numeric_trait(cx, tile, TRAIT_ALPHA, 255.0)?;
        }
        TileKind::Menu => {}
    }
    Ok(tile)
}

/// IDA: the 0058FCE3 test through the tile's +0x24: the node exists and has the app-cull bit (+0x18 & 1). Completion: 100%.
fn scene_node_culled(cx: &InterfaceCx, tile: TileId) -> bool {
    tile_scene_node(cx, tile).is_some_and(|node| {
        cx.world
            .get::<SceneNodePayload>(node)
            .is_some_and(|payload| payload.effective_av_flags & 1 != 0)
    })
}

/// IDA: 0058FBA0 Tile_UpdateSceneGraph(tile, ancestorChanged). 0058FD43: Tile3D (904) children run 00590D20; there are
/// no host 3D tiles.
/// Completion: 100% of the traversal.
pub fn tile_update_scene_graph(
    cx: &mut InterfaceCx,
    tile: TileId,
    ancestor_changed: bool,
) -> Result<(), GameError> {
    if cx.engine.tile(tile).unknown_05 != 0 {
        return Ok(());
    }
    if cx.engine.tile(tile).parent == interface_menu_root(cx) {
        // 0058FBD0..0058FC2A: stacking 102 -> 6006, 103 -> 6007.
        let stacking = tile_numeric_trait(cx, tile, TRAIT_STACKING);
        if stacking == 102.0 {
            let value = tile_get_or_create_value(cx, tile, TRAIT_STACKING);
            tile_value_set_number(cx, value, 6006.0)?;
        } else if stacking == 103.0 {
            let value = tile_get_or_create_value(cx, tile, TRAIT_STACKING);
            tile_value_set_number(cx, value, 6007.0)?;
        }
    }
    let saved = cx.engine.in_scene_update;
    cx.engine.in_scene_update = true;
    let changed = interface_update_tile_scene(cx, tile)?;
    cx.engine.in_scene_update = saved;
    let children = cx.engine.tile(tile).children_30.clone();
    for child in children {
        let visible = find_value(cx, child, TRAIT_VISIBLE);
        let clip = find_value(cx, child, TRAIT_LIST_CLIP);
        let check_hidden = visible.is_some_and(|value| cx.engine.value(value).number_04 == 1.0)
            || clip.is_some_and(|value| cx.engine.value(value).number_04 == 2.0);
        let mut recurse = true;
        if check_hidden {
            // 0058FCE3: the child's or any ancestor's scene node app-culled.
            let mut current = Some(child);
            while let Some(id) = current {
                if scene_node_culled(cx, id) {
                    recurse = false;
                }
                current = cx.engine.tile(id).parent;
            }
        }
        if recurse {
            tile_update_scene_graph(cx, child, ancestor_changed || changed)?;
        }
    }
    if !ancestor_changed
        && changed
        && let Some(node) = tile_scene_node(cx, tile)
    {
        // 00707370 NiAVObject_Update(tile +0x24, 0.0, 1).
        let result = update_scene_object(cx.world, node, 0.0, true, cx.runtime)?;
        if result.status != SceneUpdateStatus::Updated {
            return Err(GameError::unclassified(
                0x0058FD7E,
                "0058FD7E: NiAVObject_Update reported an unsupported state",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamebryo::scene_update::NoSceneControllers;
    use crate::tes::interface_manager::{InterfaceSettings, interface_settings_from};
    use crate::tes::settings::{SettingCollections, register_static_settings};
    use crate::tes::tile::{NullInterfaceHost, with_interface_cx};
    use bevy_ecs::prelude::*;

    fn settings() -> InterfaceSettings {
        let mut collections = SettingCollections::default();
        register_static_settings(&mut collections);
        interface_settings_from(&collections.ini, 1920, 1080, 8, Default::default())
    }

    fn run<R>(
        world: &mut World,
        host: &mut NullInterfaceHost,
        body: impl FnOnce(&mut InterfaceCx) -> R,
    ) -> R {
        with_interface_cx(world, &settings(), host, &mut NoSceneControllers, body)
    }

    fn rect(cx: &mut InterfaceCx, parent: Option<TileId>, name: &[u8]) -> TileId {
        tile_initialize(cx, TileKind::Rect, parent, Some(name), None).unwrap()
    }

    #[test]
    fn tokens() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        run(&mut world, &mut host, |cx| {
            tile_register_builtin_tokens(cx);
            let count: usize = cx.engine.builtin_tokens.iter().map(Vec::len).sum();
            assert_eq!(count + cx.engine.custom_tokens.len(), 223);
            assert_eq!(tile_lookup_token(cx, b"&true;"), 2);
            assert_eq!(tile_lookup_token(cx, b"RECT"), 901);
            assert_eq!(tile_lookup_token(cx, b"&LoadingMenu;"), 1007);
            assert_eq!(tile_lookup_token(cx, b""), 0);
            assert_eq!(tile_lookup_token(cx, b"nothing"), 0);
            // A hit counts the reference.
            let bucket = &cx.engine.builtin_tokens[token_bucket(b'r')];
            let entry = bucket.iter().find(|entry| entry.name == b"rect").unwrap();
            assert_eq!(entry.refcount, 1);
            // 0058B040: a new name takes 10000 + count; the same name finds it, in any case.
            let base = cx.engine.custom_tokens.len() as u32 + 10000;
            assert_eq!(tile_get_or_register_token(cx, b"_mine", -1), base);
            assert_eq!(tile_get_or_register_token(cx, b"_MINE", -1), base);
            assert_eq!(tile_get_or_register_token(cx, b"_other", 77), 77);
            assert_eq!(tile_token_name(cx, base).as_deref(), Some(&b"_mine"[..]));
            assert_eq!(tile_token_name(cx, 901).as_deref(), Some(&b"rect"[..]));
            assert_eq!(tile_token_name(cx, 12), None);
        });
    }

    #[test]
    fn value_stores() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        run(&mut world, &mut host, |cx| {
            let tile = rect(cx, None, b"a");
            // 0058B220 keeps the list in trait order.
            for trait_id in [TRAIT_WIDTH, TRAIT_X, TRAIT_Y] {
                tile_set_numeric_trait(cx, tile, trait_id, 1.0).unwrap();
            }
            let traits: Vec<u16> = cx
                .engine
                .tile(tile)
                .values_14
                .iter()
                .map(|&value| cx.engine.value(value).trait_18)
                .collect();
            let mut sorted = traits.clone();
            sorted.sort_unstable();
            assert_eq!(traits, sorted);
            // 0058CA50: an empty string becomes " "; a number clears the text.
            tile_set_string_trait(cx, tile, TRAIT_USER5, b"").unwrap();
            assert_eq!(
                tile_string_trait(cx, tile, TRAIT_USER5).as_deref(),
                Some(&b" "[..])
            );
            tile_set_numeric_trait(cx, tile, TRAIT_USER5, 2.0).unwrap();
            assert_eq!(tile_string_trait(cx, tile, TRAIT_USER5), None);
            // 00588C10: a missing value reads as the shared empty default, 00588BD0 as 0.
            assert_eq!(tile_string_trait(cx, tile, 4100).as_deref(), Some(&b""[..]));
            assert_eq!(tile_numeric_trait(cx, tile, 4100), 0.0);
            // The kinds' vt+4 defaults.
            assert_eq!(tile_numeric_trait(cx, tile, TRAIT_RED), 255.0);
            assert_eq!(tile_numeric_trait(cx, tile, TRAIT_ALPHA), 0.0);
            assert_eq!(cx.engine.tile(tile).flags_2c & 2, 2);
        });
    }

    /// The action switch of 0058BEE0, against what the C++ port computes (no original run reaches most of these).
    #[test]
    fn actions() {
        let cases: &[(u16, f32, f32, f32)] = &[
            (2002, 5.0, 2.0, 7.0),
            (2003, 5.0, 2.0, 3.0),
            (2004, 5.0, 2.0, 10.0),
            (2005, 5.0, 2.0, 2.5),
            (2005, 5.0, 0.0, 5.0),
            (2008, 5.0, 2.0, 2.0),
            (2008, 2.0, 5.0, 1.0),
            (2009, 2.0, 2.0, 2.0),
            (2010, 2.0, 2.0, 2.0),
            (2010, 2.0, 3.0, 1.0),
            (2011, 2.0, 3.0, 2.0),
            (2011, 3.0, 2.0, 1.0),
            (2012, 2.0, 3.0, 2.0),
            (2013, 5.0, 2.0, 2.0),
            (2013, 2.0, 5.0, 2.0),
            (2014, 5.0, 2.0, 5.0),
            (2014, 2.0, 5.0, 5.0),
            (2015, 2.0, 2.0, 2.0),
            (2015, 2.0, 1.0, 1.0),
            (2016, 1.0, 2.0, 2.0),
            (2016, 1.0, 1.0, 1.0),
            (2017, 2.0, 2.0, 1.0),
            (2017, 2.0, 3.0, 2.0),
            (2018, 7.0, 4.0, 3.0),
            (2018, -1.0, 4.0, 3.0),
            (2019, 1.25, 1.0, 2.0),
            (2025, 1.25, 1.0, 3.0),
            (2020, -3.0, 1.0, 2.0),
            (2020, 3.0, 1.0, 3.0),
            (2021, 3.0, 2.0, 3.0),
            (2021, 3.0, 1.0, 0.0),
            (2022, 3.0, 1.0, 3.0),
            (2022, 3.0, 2.0, 0.0),
            (2026, 3.0, 2.0, 1.0),
            (2026, 3.0, 1.0, 2.0),
            (2027, 3.0, 1.0, 3.0),
        ];
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        run(&mut world, &mut host, |cx| {
            let tile = rect(cx, None, b"a");
            for &(opcode, start, operand, expected) in cases {
                tile_set_numeric_trait(cx, tile, TRAIT_X, start).unwrap();
                let value = find_value(cx, tile, TRAIT_X).unwrap();
                tile_value_append_constant(cx, value, operand, opcode).unwrap();
                assert_eq!(
                    cx.engine.value(value).number_04,
                    expected,
                    "{opcode} {start} {operand}"
                );
                // 0058CA00 clears the list, unless the number is unchanged.
                tile_set_numeric_trait(cx, tile, TRAIT_X, f32::MIN).unwrap();
            }
            // 2023 is ln and 2024 log10 (0058C998: 00986B80 has fldln2, 00986CD0 fldlg2).
            tile_set_numeric_trait(cx, tile, TRAIT_X, 100.0).unwrap();
            let value = find_value(cx, tile, TRAIT_X).unwrap();
            tile_value_append_constant(cx, value, 0.0, 2024).unwrap();
            assert_eq!(cx.engine.value(value).number_04, 2.0);
            tile_set_numeric_trait(cx, tile, TRAIT_X, 1.0).unwrap();
            let value = find_value(cx, tile, TRAIT_X).unwrap();
            tile_value_append_constant(cx, value, 0.0, 2023).unwrap();
            assert_eq!(cx.engine.value(value).number_04, 0.0);
            // rand: 16383 / 32767 * 2, floored, plus 1 (fild, fdiv A3D5A8, fmul; 24-bit).
            tile_set_numeric_trait(cx, tile, TRAIT_X, 0.0).unwrap();
            let value = find_value(cx, tile, TRAIT_X).unwrap();
            tile_value_append_constant(cx, value, 2.0, 2006).unwrap();
            assert_eq!(cx.engine.value(value).number_04, 1.0);
        });
        assert!(host.warnings.is_empty());
    }

    #[test]
    fn an_unknown_action_warns() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        run(&mut world, &mut host, |cx| {
            let tile = rect(cx, None, b"a");
            let value = tile_get_or_create_value(cx, tile, TRAIT_X);
            tile_value_append_constant(cx, value, 1.0, 2500).unwrap();
        });
        assert_eq!(host.warnings, ["ERROR: Unknown action enumeration 2500 \n"]);
    }

    /// 0058CB70: a begin marker saves the number, the end marker restores it and applies its action with the inner result.
    #[test]
    fn markers_nest() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        run(&mut world, &mut host, |cx| {
            let tile = rect(cx, None, b"a");
            tile_set_numeric_trait(cx, tile, TRAIT_X, 10.0).unwrap();
            let value = find_value(cx, tile, TRAIT_X).unwrap();
            tile_value_append_marker(cx, value, 2002, 10).unwrap();
            tile_value_append_constant(cx, value, 3.0, 2004).unwrap();
            // Inside: 10 * 3 = 30. Before the end marker, the unclosed marker restores 10.
            assert_eq!(cx.engine.value(value).number_04, 10.0);
            tile_value_append_marker(cx, value, 2002, 15).unwrap();
            // 10 + 30.
            assert_eq!(cx.engine.value(value).number_04, 40.0);
        });
    }

    /// 0058CC60: a reference caches the source, joins its chain, and follows it through 0058BDD0.
    #[test]
    fn references_follow_their_source() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        run(&mut world, &mut host, |cx| {
            let source = rect(cx, None, b"source");
            let target = rect(cx, None, b"target");
            tile_set_numeric_trait(cx, source, TRAIT_WIDTH, 4.0).unwrap();
            let value = tile_get_or_create_value(cx, target, TRAIT_WIDTH);
            assert_eq!(
                tile_value_append_reference(cx, value, source, 100, 2001).unwrap(),
                None,
                "a trait below 4001 is rejected"
            );
            // 58cc66 is a signed `jl`, so a negative ID is below 4001 too.
            assert_eq!(
                tile_value_append_reference(cx, value, source, u32::MAX, 2001).unwrap(),
                None,
            );
            // 58cc76: 00B13BC0 is 10000 in the file, so 5000 is accepted (the C++ rejected 4098..9999).
            assert!(
                tile_value_append_reference(cx, value, source, 5000, 2001)
                    .unwrap()
                    .is_some()
            );
            tile_set_numeric_trait(cx, target, TRAIT_WIDTH, 0.0).unwrap();
            tile_value_append_reference(cx, value, source, u32::from(TRAIT_WIDTH), 2001)
                .unwrap()
                .unwrap();
            assert_eq!(tile_numeric_trait(cx, target, TRAIT_WIDTH), 4.0);
            tile_set_numeric_trait(cx, source, TRAIT_WIDTH, 9.0).unwrap();
            assert_eq!(tile_numeric_trait(cx, target, TRAIT_WIDTH), 9.0);
            // A string source copies its text down.
            tile_set_string_trait(cx, source, TRAIT_USER5, b"text").unwrap();
            let linked = tile_get_or_create_value(cx, target, TRAIT_USER5);
            tile_value_append_reference(cx, linked, source, u32::from(TRAIT_USER5), 2001)
                .unwrap()
                .unwrap();
            tile_set_string_trait(cx, source, TRAIT_USER5, b"other").unwrap();
            assert_eq!(
                tile_string_trait(cx, target, TRAIT_USER5).as_deref(),
                Some(&b"other"[..])
            );
        });
    }

    /// 0058BAD0: a loop logs its chain, zeroes the opcodes and does not hang.
    #[test]
    fn an_evaluation_loop_is_cut() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        run(&mut world, &mut host, |cx| {
            let a = rect(cx, None, b"a");
            let b = rect(cx, None, b"b");
            tile_set_numeric_trait(cx, a, TRAIT_X, 1.0).unwrap();
            tile_set_numeric_trait(cx, b, TRAIT_X, 2.0).unwrap();
            let on_a = find_value(cx, a, TRAIT_X).unwrap();
            let on_b = find_value(cx, b, TRAIT_X).unwrap();
            tile_value_append_reference(cx, on_a, b, u32::from(TRAIT_X), 2001)
                .unwrap()
                .unwrap();
            tile_value_append_reference(cx, on_b, a, u32::from(TRAIT_X), 2001)
                .unwrap()
                .unwrap();
            // Equal numbers stop the propagation, so a changing term closes the loop.
            tile_value_append_constant(cx, on_b, 1.0, 2002).unwrap();
            assert_eq!(cx.engine.evaluation_depth, 0, "the stack unwinds");
        });
        assert!(
            host.debug
                .iter()
                .any(|line| line == "***** ERROR: Loop Detected\n")
        );
    }

    /// 0058D1C0: the front of the list without `after`, the place after it with one, and the +0xFD0 counts.
    #[test]
    fn parenting() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        run(&mut world, &mut host, |cx| {
            let parent = rect(cx, None, b"parent");
            let first = rect(cx, Some(parent), b"first");
            let second = rect(cx, Some(parent), b"second");
            assert_eq!(cx.engine.tile(parent).children_30, [second, first]);
            assert_eq!(tile_numeric_trait(cx, parent, TRAIT_CHILD_COUNT), 2.0);
            let third =
                tile_initialize(cx, TileKind::Rect, Some(parent), None, Some(second)).unwrap();
            assert_eq!(cx.engine.tile(parent).children_30, [second, third, first]);
            tile_set_parent(cx, second, None, None).unwrap();
            assert_eq!(cx.engine.tile(parent).children_30, [third, first]);
            assert_eq!(cx.engine.tile(second).parent, None);
            assert_eq!(tile_numeric_trait(cx, parent, TRAIT_CHILD_COUNT), 2.0);
        });
    }

    /// 58d1cc: a released old parent keeps its count and its list entry, and the tile still moves.
    #[test]
    fn a_released_parent_keeps_its_child() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        run(&mut world, &mut host, |cx| {
            let old = rect(cx, None, b"old");
            let new = rect(cx, None, b"new");
            let child = rect(cx, Some(old), b"child");
            cx.engine.tile_mut(old).released_04 = 1;
            tile_set_parent(cx, child, Some(new), None).unwrap();
            assert_eq!(cx.engine.tile(old).children_30, [child]);
            assert_eq!(tile_numeric_trait(cx, old, TRAIT_CHILD_COUNT), 1.0);
            assert_eq!(cx.engine.tile(child).parent, Some(new));
            assert_eq!(cx.engine.tile(new).children_30, [child]);
        });
    }

    /// 58d221 stores +0x10 before the count at 58d25a: a depth change that the count's reference chain evaluates finds
    /// the child already under the new parent, so 0058B2F0's walk sees the menu.
    #[test]
    fn the_parent_is_stored_before_the_new_count() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        run(&mut world, &mut host, |cx| {
            let root = rect(cx, None, b"root");
            let parent = rect(cx, Some(root), b"parent");
            let child = rect(cx, None, b"child");
            // The child's depth follows the parent's count, so the count's evaluation runs 0058B2F0 on the child.
            let depth = tile_get_or_create_value(cx, child, TRAIT_DEPTH);
            tile_value_append_reference(cx, depth, parent, u32::from(TRAIT_CHILD_COUNT), 2001)
                .unwrap()
                .unwrap();
            let menu = cx.engine.menus.len() as u32;
            cx.engine
                .menus
                .push(Menu::new(crate::tes::tile::MenuClass::Unported(1001)));
            cx.engine.tile_mut(parent).kind = TileKind::Menu;
            cx.engine.tile_mut(parent).menu_44 = Some(crate::tes::tile::MenuId(menu));
            tile_set_parent(cx, child, Some(parent), None).unwrap();
            // 0058B5F3: the parent's child, one level under the top, raises the menu's +0x18 only with +0x10 stored.
            assert_eq!(cx.engine.menu(crate::tes::tile::MenuId(menu)).depth_18, 1);
        });
    }

    /// 0058B800's tokens and the named search.
    #[test]
    fn sources() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        run(&mut world, &mut host, |cx| {
            let top = rect(cx, None, b"top");
            let a = rect(cx, Some(top), b"a");
            let b = rect(cx, Some(top), b"b");
            let c = rect(cx, Some(b), b"c");
            // The list head is b, as 0058D1C0 adds at the front.
            assert_eq!(tile_resolve_source(cx, Some(c), b"parent()"), Some(b));
            assert_eq!(tile_resolve_source(cx, Some(c), b"me()"), Some(c));
            assert_eq!(tile_resolve_source(cx, Some(b), b"sibling()"), Some(a));
            assert_eq!(
                tile_resolve_source(cx, Some(a), b"sibling()"),
                Some(b),
                "wraps to the head"
            );
            assert_eq!(
                tile_resolve_source(cx, Some(a), b"sibling(B)"),
                Some(b),
                "case-insensitive"
            );
            // 589015: a second '(' writes over the argument from its start, so the name in "sibling(b(a" is "a".
            assert_eq!(tile_resolve_source(cx, Some(a), b"sibling(b(a"), Some(a));
            assert_eq!(tile_resolve_source(cx, Some(top), b"child()"), Some(b));
            assert_eq!(tile_resolve_source(cx, Some(top), b"child(c)"), Some(c));
            assert_eq!(
                tile_resolve_source(cx, Some(top), b"child(C)"),
                None,
                "case-sensitive"
            );
            assert_eq!(tile_resolve_source(cx, None, b"screen()"), None);
            // The default arm searches from below the root, by the whole source text.
            assert_eq!(tile_resolve_source(cx, Some(c), b"C"), Some(c));
            assert_eq!(tile_resolve_source(cx, Some(c), b"none"), None);
        });
    }

    /// 0058B2F0's flags per trait and kind.
    #[test]
    fn dirty_flags() {
        let mut world = World::new();
        let mut host = NullInterfaceHost::default();
        run(&mut world, &mut host, |cx| {
            let tile = rect(cx, None, b"a");
            for (trait_id, flag) in [
                (TRAIT_X, 1),
                (TRAIT_Y, 1),
                (TRAIT_DEPTH, 1),
                (TRAIT_VISIBLE, 4),
                (TRAIT_LIST_CLIP, 4),
                (TRAIT_ALPHA, 8),
                (TRAIT_WIDTH, 0x10),
                (TRAIT_CROP_X, 0x10),
                (TRAIT_CLIP_WINDOW, 0x100),
                (TRAIT_CLIPS, 0x200),
            ] {
                cx.engine.tile_mut(tile).flags_2c = 0;
                tile_generic_update(cx, tile, trait_id, 1.0);
                assert_eq!(cx.engine.tile(tile).flags_2c, flag, "{trait_id}");
            }
            let text = tile_initialize(cx, TileKind::Text, None, None, None).unwrap();
            for trait_id in [TRAIT_WRAP_WIDTH, TRAIT_FONT, TRAIT_IS_HTML] {
                cx.engine.tile_mut(text).flags_2c = 0;
                tile_generic_update(cx, text, trait_id, 1.0);
                assert_eq!(cx.engine.tile(text).flags_2c, 2, "{trait_id}");
            }
            cx.engine.tile_mut(text).flags_2c = 0;
            tile_generic_update(cx, text, TRAIT_WIDTH, 1.0);
            assert_eq!(
                cx.engine.tile(text).flags_2c,
                0,
                "a text's width is not 0x10"
            );
            let image = tile_initialize(cx, TileKind::Image, None, None, None).unwrap();
            cx.engine.tile_mut(image).flags_2c = 0;
            tile_set_string_trait(cx, image, TRAIT_FILENAME, b"x.dds").unwrap();
            assert_eq!(cx.engine.tile(image).flags_2c & 0x20, 0x20);
        });
    }
}
