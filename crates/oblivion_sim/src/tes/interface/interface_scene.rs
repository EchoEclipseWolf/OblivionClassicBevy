//! Phase 07 (D3): the per-tile scene update 0058E870 and the kinds' vt+8 node builders (00591DC0 rect, 00591C20 menu,
//! 00591600 image, 00592390 text) over the Phase 05 scene graphs. Node state
//! (names, parents, flags, translates, properties) is ported; the leaf geometry is a `TileSceneGeometry` record for
//! M6's host quad projection (a recorded substitution). Main thread after
//! CreateDevice: 24-bit x87, so `f32` arithmetic in the original order. Evidence: evidence/phase07-window-2026-10-06
//! (before/0x58E870 and the builders, the P07-RUN-001 node dump).

use bevy_ecs::prelude::*;
use oblivion_core::crt::stricmp;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::fonts::{FontManager, font_data_float};
use super::interface_manager::{InterfaceManager, ui_virtual_height, ui_virtual_width};
use super::interface_text::font_layout_text;
use super::tile::{InterfaceCx, TileId, TileKind, lift_text};
use super::tile_values::*;
use crate::bs::scene_graphs::{AttachChildStatus, attach_scene_child, detach_scene_child};
use crate::gamebryo::av_object::NiAvParent;
use crate::gamebryo::mesh_geometry::allocate_geometry_data_id;
use crate::gamebryo::object_net::NiObjectNet;
use crate::gamebryo::object_net::set_object_name;
use crate::gamebryo::properties::new_material_property;
use crate::gamebryo::scene_node::{SceneNodePayload, spawn_scene_node};
use crate::tes::game_settings::GameSettingCollection;

/// IDA: NiTriShape A7ED5C (7174B0, 0xC0) and BSScissorTriShape A3FC04 (4A1780, 0xD0) vtables of the tile geometry.
pub const TILE_TRI_SHAPE_VTABLE: u32 = 0x00A7_ED5C;
pub const TILE_SCISSOR_TRI_SHAPE_VTABLE: u32 = 0x00A3_FC04;

/// IDA: the D3 substitution record for one leaf geometry under a tile node (the vt+8 builders 00591DC0 and 00591600 and
/// the 00576670 text shape): the values the Phase 07 window writes into the
/// shape, its NiMaterialProperty (0x5C), NiTexturingProperty (0x30) and vertex data. The render objects and 0057F170
/// are M6's. Completion: 100% of the values the window writes.
#[derive(Clone, Debug, PartialEq)]
pub struct TileSceneGeometry {
    pub original_vtable: u32,
    /// +0x54.
    pub translate_54: [f32; 3],
    /// Four corner vertices (x, y, z), UVs and the two triangles.
    pub vertices: [f32; 12],
    pub uvs: [f32; 8],
    pub has_uvs: bool,
    pub triangles: [u16; 6],
    /// BSScissorTriShape: the 4A1780 virtual extent.
    pub scissor_extent: [f32; 2],
    /// Vertex data +0x2E flags (|= 9 resize, & 0xFFF | 0x8000, |= 4 colors).
    pub data_flags_2e: u16,
    /// Vertex data +0x0A: the 00728670 word_B27504 ID its constructor takes (P08-RUN-002: IDs 1..0xF over the 15 Phase
    /// 07 shapes).
    pub data_id_0a: u16,
    pub has_material: bool,
    /// NiMaterialProperty +0x18: the 007095A0 00B25AF4 ID (P08-RUN-002: 0x13..0x1D over the 11 rect and image
    /// materials).
    pub material_id_18: u32,
    /// NiMaterialProperty +0x54 revision.
    pub material_revision_54: u32,
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
    pub specular: [f32; 3],
    pub emissive: [f32; 3],
    pub alpha: f32,
    pub has_texturing: bool,
    /// NiTexturingProperty: the base texture path (empty: none), clamp mode, flags.
    pub texture: Vec<u8>,
    pub texture_size: [u32; 2],
    pub clamp: u32,
    pub texturing_flags: u16,
    /// The text shape's per-vertex colour (00574200; 0058FA2A rewrites it).
    pub has_vertex_colors: bool,
    pub vertex_color: [f32; 4],
    /// 00573F10: the text shape's glyph quads. Empty for a rect or an image.
    pub glyphs: Vec<super::interface_text::TextGlyph>,
    /// +0x0C slot 0 of the FontInfo, for a text shape.
    pub font_texture: Option<Entity>,
    /// BSScissorTriShape +0xC0..+0xCC (4A17F0): left, top, right, bottom in renderer pixels. All zero means no scissor (00581390).
    pub scissor_c0: [f32; 4],
    /// TileImage +0x44 (591A80): the save screenshot, which 00591360 takes over the file.
    pub preview: Option<std::sync::Arc<SavePreview>>,
}

/// IDA: the RGB24 NiPixelData 0045D5F0 builds from a save's screenshot: width, height, and 3 bytes a pixel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavePreview {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl Default for TileSceneGeometry {
    /// IDA: N/A host helper. Completion: 100%.
    fn default() -> Self {
        Self {
            original_vtable: TILE_TRI_SHAPE_VTABLE,
            translate_54: [0.0; 3],
            vertices: [0.0; 12],
            uvs: [0.0; 8],
            has_uvs: false,
            triangles: [0, 1, 2, 2, 1, 3],
            scissor_extent: [0.0; 2],
            data_flags_2e: 0,
            data_id_0a: 0,
            has_material: false,
            material_id_18: 0,
            material_revision_54: 0,
            ambient: [0.5; 3],
            diffuse: [0.5; 3],
            specular: [0.0; 3],
            emissive: [0.0; 3],
            alpha: 1.0,
            has_texturing: false,
            texture: Vec::new(),
            texture_size: [0; 2],
            clamp: 0,
            texturing_flags: 0,
            has_vertex_colors: false,
            vertex_color: [0.0; 4],
            glyphs: Vec::new(),
            font_texture: None,
            scissor_c0: [0.0; 4],
            preview: None,
        }
    }
}

/// IDA: tile +0x24 NiNode (0xDC) as a Phase 05 scene node, its substituted geometry and the 00590580 "Tileptr" extra
/// data (+0xC tile back reference, cleared by 00589890 when the node is released).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileSceneRecord {
    pub node: Option<Entity>,
    pub geometry: Vec<TileSceneGeometry>,
    pub extra_tile: Option<TileId>,
}

/// IDA: the tile's +0x24. Completion: 100%.
#[must_use]
pub fn tile_scene_node(cx: &InterfaceCx, tile: TileId) -> Option<Entity> {
    cx.engine
        .tile(tile)
        .scene_24
        .as_ref()
        .and_then(|record| record.node)
}

/// IDA: N/A host helper. Completion: 100%.
fn trait_of(cx: &InterfaceCx, tile: TileId, trait_id: u16) -> f32 {
    tile_numeric_trait(cx, tile, trait_id)
}

/// IDA: N/A host helper. Completion: 100%.
fn record_mut<'a>(cx: &'a mut InterfaceCx, tile: TileId) -> &'a mut TileSceneRecord {
    cx.engine
        .tile_mut(tile)
        .scene_24
        .get_or_insert_with(TileSceneRecord::default)
}

/// IDA: 005894D0: the node of the tile or its nearest ancestor with one. Completion: 100%.
fn nearest_node(cx: &InterfaceCx, mut tile: Option<TileId>) -> Option<Entity> {
    while let Some(id) = tile {
        if let Some(node) = tile_scene_node(cx, id) {
            return Some(node);
        }
        tile = cx.engine.tile(id).parent;
    }
    None
}

/// IDA: 00589AA0: 00589890 clears the "Tileptr" extra's tile, the parent's DetachChild (vt+0x88) drops the node, then
/// +0x24 = 0. Completion: 100%. The entity stays spawned, as references are not modelled.
pub(in crate::tes) fn release_node(cx: &mut InterfaceCx, tile: TileId) {
    let Some(node) = tile_scene_node(cx, tile) else {
        return;
    };
    let parent = cx.world.get::<NiAvParent>(node).and_then(|link| link.0);
    if let Some(parent) = parent {
        detach_scene_child(cx.world, parent, node);
    }
    let record = record_mut(cx, tile);
    record.extra_tile = None;
    record.node = None;
    record.geometry.clear();
}

/// IDA: N/A host helper. Completion: 100%.
fn require_attached(status: AttachChildStatus, message: &'static str) -> Result<(), GameError> {
    if status != AttachChildStatus::Attached {
        return Err(GameError::from_legacy_message(message));
    }
    Ok(())
}

/// IDA: N/A host helper. Completion: 100%.
fn main_root(cx: &InterfaceCx) -> Option<Entity> {
    cx.world
        .get_resource::<InterfaceManager>()
        .and_then(|manager| manager.main_root_054)
}

/// IDA: the node publication shared by the builders: NiNode (0xDC) at +0x24, parent (or Main Root) AttachChild(node,
/// 1), SetName. Completion: 100% of the order.
fn publish_node(
    cx: &mut InterfaceCx,
    tile: TileId,
    parent_node: Option<Entity>,
    message: &'static str,
) -> Result<Entity, GameError> {
    let node = spawn_scene_node(cx.world, SceneNodePayload::default());
    record_mut(cx, tile).node = Some(node);
    if let Some(parent) = parent_node.or_else(|| main_root(cx)) {
        require_attached(
            attach_scene_child(cx.world, parent, Some(node), true),
            message,
        )?;
    }
    let name = cx.engine.tile(tile).name_08.clone();
    set_object_name(
        &mut cx
            .world
            .get_mut::<NiObjectNet>(node)
            .expect("just spawned")
            .name,
        Some(&name),
    );
    Ok(node)
}

/// IDA: RefreshEffectState and RefreshPropertyState keep no host state here; +0x7C = 1; 00590580 "Tileptr" extra data
/// (0x14) with the tile back reference. Completion: 100%.
fn finish_node(cx: &mut InterfaceCx, tile: TileId) {
    if let Some(mut manager) = cx.world.get_resource_mut::<InterfaceManager>() {
        manager.geometry_changed_07c = 1;
    }
    record_mut(cx, tile).extra_tile = Some(tile);
}

/// IDA: N/A host helper. Completion: 100%.
fn set_app_culled(cx: &mut InterfaceCx, node: Entity, culled: bool) {
    let mut payload = cx
        .world
        .get_mut::<SceneNodePayload>(node)
        .expect("a tile node has a payload");
    if culled {
        payload.effective_av_flags |= 1;
    } else {
        payload.effective_av_flags &= !1;
    }
}

/// IDA: 00591DC0 the rect node (also 00591C20's first step). Completion: 100% of the order.
fn build_rect(cx: &mut InterfaceCx, tile: TileId) -> Result<bool, GameError> {
    let parent_node = nearest_node(cx, cx.engine.tile(tile).parent);
    let x = trait_of(cx, tile, TRAIT_X);
    let z = -trait_of(cx, tile, TRAIT_Y);
    let width = trait_of(cx, tile, TRAIT_WIDTH);
    let height = trait_of(cx, tile, TRAIT_HEIGHT);
    let red = trait_of(cx, tile, TRAIT_RED) * 0.007_812_5;
    let green = trait_of(cx, tile, TRAIT_GREEN) * 0.007_812_5;
    let blue = trait_of(cx, tile, TRAIT_BLUE) * 0.007_812_5;
    let alpha = trait_of(cx, tile, TRAIT_ALPHA) * 0.007_812_5;
    let mut geometry = TileSceneGeometry {
        // 00592004 (00591FC6 clipped) the shape constructor's 00732D70 data takes the next word_B27504 ID before the
        // material (P08-RUN-002 order).
        data_id_0a: allocate_geometry_data_id(cx.world),
        ..TileSceneGeometry::default()
    };
    let bottom = -height;
    geometry.vertices = [
        0.0, 0.0, 0.0, 0.0, 0.0, bottom, width, 0.0, 0.0, width, 0.0, bottom,
    ];
    if trait_of(cx, tile, TRAIT_CLIPS) == 2.0 {
        // 00591F8C BSScissorTriShape (0xD0) with the virtual extent.
        geometry.original_vtable = TILE_SCISSOR_TRI_SHAPE_VTABLE;
        geometry.scissor_extent = [
            ui_virtual_width(cx.settings),
            ui_virtual_height(cx.settings),
        ];
        cx.engine.tile_mut(tile).scissor_40 = 1;
    } else {
        geometry.original_vtable = TILE_TRI_SHAPE_VTABLE;
        cx.engine.tile_mut(tile).scissor_40 = 0;
    }
    // 0059201D NiMaterialProperty: +0x54 += 3, alpha, ambient = diffuse = RGB, specular white, +0x54 += 1. 00592038
    // takes the 00B25AF4 ID.
    geometry.has_material = true;
    geometry.material_id_18 = new_material_property(cx.world).material_id_18;
    geometry.material_revision_54 += 3;
    geometry.alpha = alpha;
    geometry.ambient = [red, green, blue];
    geometry.diffuse = [red, green, blue];
    geometry.specular = [1.0, 1.0, 1.0];
    geometry.material_revision_54 += 1;
    geometry.translate_54 = [x, trait_of(cx, tile, TRAIT_DEPTH) * -0.008, z];
    let record = record_mut(cx, tile);
    record.geometry.clear();
    record.geometry.push(geometry);
    let node = publish_node(
        cx,
        tile,
        parent_node,
        "0059215F: the tile node was not attached",
    )?;
    let culled = trait_of(cx, tile, TRAIT_VISIBLE) == 1.0;
    set_app_culled(cx, node, culled);
    finish_node(cx, tile);
    // 00592203 / 00592221 0058B2F0(x) and (y).
    let value = trait_of(cx, tile, TRAIT_X);
    tile_generic_update(cx, tile, TRAIT_X, value);
    let value = trait_of(cx, tile, TRAIT_Y);
    tile_generic_update(cx, tile, TRAIT_Y, value);
    Ok(true)
}

/// IDA: 00591600 the image node. Completion: 100% of the order.
fn build_image(cx: &mut InterfaceCx, tile: TileId) -> Result<bool, GameError> {
    let parent_node = nearest_node(cx, cx.engine.tile(tile).parent);
    let mut geometry = TileSceneGeometry {
        // 0059182C (005917CF clipped) the shape constructor's 00732D70 data takes the next word_B27504 ID first.
        data_id_0a: allocate_geometry_data_id(cx.world),
        vertices: [0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 1.0, 0.0, 0.0, 1.0, 0.0, -1.0],
        uvs: [0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0],
        has_uvs: true,
        ..TileSceneGeometry::default()
    };
    if trait_of(cx, tile, TRAIT_CLIPS) == 2.0 {
        geometry.original_vtable = TILE_SCISSOR_TRI_SHAPE_VTABLE;
        geometry.scissor_extent = [
            ui_virtual_width(cx.settings),
            ui_virtual_height(cx.settings),
        ];
        cx.engine.tile_mut(tile).byte_48 = 1;
    } else {
        geometry.original_vtable = TILE_TRI_SHAPE_VTABLE;
        cx.engine.tile_mut(tile).byte_48 = 0;
    }
    // 00591856 NiTexturingProperty: no base texture, clamp 3 when tile == 2, flags (flags & 0xFFF1) | 4.
    geometry.has_texturing = true;
    geometry.texture.clear();
    geometry.clamp = if trait_of(cx, tile, TRAIT_TILE_MODE) == 2.0 {
        3
    } else {
        0
    };
    geometry.texturing_flags = (geometry.texturing_flags & 0xFFF1) | 4;
    // 005918C7 NiMaterialProperty: +0x54 += 2, alpha 0, emissive white. 005918E3 takes the 00B25AF4 ID.
    geometry.has_material = true;
    geometry.material_id_18 = new_material_property(cx.world).material_id_18;
    geometry.material_revision_54 += 2;
    geometry.alpha = 0.0;
    geometry.emissive = [1.0, 1.0, 1.0];
    let record = record_mut(cx, tile);
    record.geometry.clear();
    record.geometry.push(geometry);
    let node = publish_node(
        cx,
        tile,
        parent_node,
        "00591986: the tile node was not attached",
    )?;
    // 005919A7 the manager's alpha property (+0x78) on the node.
    if let Some(alpha) = cx
        .world
        .get_resource::<InterfaceManager>()
        .and_then(|manager| manager.alpha_property_078)
    {
        crate::gamebryo::scene_state::attach_object_property(cx.world, node, Some(alpha))?;
    }
    finish_node(cx, tile);
    // 00591A0D 0058B2F0(depth); clips 2 -> 0x200; flags |= 0x3D.
    let value = trait_of(cx, tile, TRAIT_DEPTH);
    tile_generic_update(cx, tile, TRAIT_DEPTH, value);
    if trait_of(cx, tile, TRAIT_CLIPS) == 2.0 {
        cx.engine.tile_mut(tile).flags_2c |= 0x200;
    }
    cx.engine.tile_mut(tile).flags_2c |= 0x3D;
    Ok(true)
}

/// IDA: 00592390 the text node (the non-HTML build through 00576670). Completion: 100% of the order.
/// TODO: [ ] 00578C10 the HTML build (later menus): a host stop.
fn build_text(cx: &mut InterfaceCx, tile: TileId) -> Result<bool, GameError> {
    let parent_node = nearest_node(cx, cx.engine.tile(tile).parent);
    let x = trait_of(cx, tile, TRAIT_X);
    let z = -trait_of(cx, tile, TRAIT_Y);
    let mut wrap_width = x87::ftol(trait_of(cx, tile, TRAIT_WRAP_WIDTH));
    let mut wrap_lines = x87::ftol(trait_of(cx, tile, TRAIT_WRAP_LINES));
    let justify = x87::ftol(trait_of(cx, tile, TRAIT_JUSTIFY));
    let font_trait = trait_of(cx, tile, TRAIT_FONT);
    let font_trait = if font_trait > 1.0 { font_trait } else { 1.0 };
    let font_index = x87::ftol(font_trait - 1.0);
    let html = trait_of(cx, tile, TRAIT_IS_HTML) > 0.0;
    // 005924C2..00592613 the colour: each channel / 255 when present and non-zero; alpha 1 without the trait.
    let channel = |cx: &InterfaceCx, trait_id| {
        let raw = if find_value(cx, tile, trait_id).is_some() {
            trait_of(cx, tile, trait_id)
        } else {
            0.0
        };
        if raw == 0.0 { 0.0 } else { raw / 255.0 }
    };
    let red = channel(cx, TRAIT_RED);
    let green = channel(cx, TRAIT_GREEN);
    let blue = channel(cx, TRAIT_BLUE);
    let alpha = if find_value(cx, tile, TRAIT_ALPHA).is_some() {
        channel(cx, TRAIT_ALPHA)
    } else {
        1.0
    };
    // 00592617..00592678 the string trait, else "%0.f" of the number.
    let text = match tile_string_trait(cx, tile, TRAIT_STRING) {
        Some(stored) if !stored.is_empty() => stored,
        _ => format!("{:.0}", f64::from(trait_of(cx, tile, TRAIT_STRING))).into_bytes(),
    };
    if wrap_width < 1 {
        wrap_width = x87::ftol(ui_virtual_width(cx.settings));
    }
    if wrap_lines < 1 {
        wrap_lines = i32::MAX;
    }
    // 0059247F: the FontInfo for the trait.
    let fonts = cx
        .world
        .get_resource::<FontManager>()
        .ok_or(GameError::unclassified(
            0x0059247F,
            "0059247F: no FontManager for the text tile",
        ))?;
    if html || fonts.unknown_14 != 0 {
        return Err(GameError::unclassified(
            0x005926B3,
            "005926B3: the HTML text build (00578C10) is carried",
        ));
    }
    let font = usize::try_from(font_index)
        .ok()
        .and_then(|index| fonts.font_infos_00.get(index))
        .and_then(Option::as_ref)
        .ok_or(GameError::unclassified(
            0x0059247F,
            "0059247F: no FontInfo for the text tile",
        ))?;
    // 00592714 00576670(font, text, &width, &limit, 0, lines, justify, 10, colour, 1): the shape's translate is (0, 0,
    // -(2 * (+0x2C - line height) + extra)) with extra 6 for font id 3. 00575B91
    // reads the wrap limit (trait 4053) but nothing uses it.
    let default_settings;
    let settings = if let Some(settings) = cx.world.get_resource::<GameSettingCollection>() {
        settings
    } else {
        default_settings = GameSettingCollection::default();
        &default_settings
    };
    let layout = font_layout_text(
        font, settings, &text, wrap_width, wrap_lines, b'\n', justify,
    )?;
    let line_height = font_data_float(&font.font_data_38, 0);
    let extra = if font.id_08 == 3 { 6.0 } else { 0.0 };
    let lift = (font.top_extent_2c - line_height) * 2.0 + extra;
    let font_path = font.path_04.clone().unwrap_or_default();
    let font_texture = font.textures_0c[0];
    let glyphs = layout.glyphs.clone();
    let record = record_mut(cx, tile);
    record.geometry.clear();
    let node = spawn_scene_node(cx.world, SceneNodePayload::default());
    record_mut(cx, tile).node = Some(node);
    // 00574200: a BSScissorTriShape (4A1780, extent 0, 0) with per-vertex colours and the FontInfo's first texturing
    // property (+0x0C). 00574493 the 4A1780 shape's data takes the next word_B27504 ID; the
    // text shape has no NiMaterialProperty (P08-RUN-002).
    let mut geometry = TileSceneGeometry {
        data_id_0a: allocate_geometry_data_id(cx.world),
        original_vtable: TILE_SCISSOR_TRI_SHAPE_VTABLE,
        translate_54: [0.0, 0.0, 0.0 - lift],
        has_texturing: true,
        texture: font_path,
        has_vertex_colors: true,
        vertex_color: [red, green, blue, alpha],
        glyphs,
        font_texture,
        ..TileSceneGeometry::default()
    };
    // 0059271E..00592731 tile +0x44..+0x4C, then the shape translate (x, depth * -0.008, -y) and the node's
    // AttachChild(shape, 0).
    cx.engine.tile_mut(tile).geometry_translate_44 = geometry.translate_54;
    geometry.translate_54 = [x, trait_of(cx, tile, TRAIT_DEPTH) * -0.008, z];
    record_mut(cx, tile).geometry.push(geometry);
    // 0059293A..00592956 width and height from the layout (fild).
    tile_set_numeric_trait(cx, tile, TRAIT_WIDTH, layout.width as f32)?;
    tile_set_numeric_trait(cx, tile, TRAIT_HEIGHT, layout.height as f32)?;
    cx.engine.tile_mut(tile).flags_2c |= 1;
    if trait_of(cx, tile, TRAIT_CLIPS) == 2.0 {
        cx.engine.tile_mut(tile).flags_2c |= 0x200;
    }
    if let Some(parent) = parent_node.or_else(|| main_root(cx)) {
        require_attached(
            attach_scene_child(cx.world, parent, Some(node), true),
            "005929AB: the tile node was not attached",
        )?;
    }
    let culled = trait_of(cx, tile, TRAIT_VISIBLE) == 1.0;
    set_app_culled(cx, node, culled);
    finish_node(cx, tile);
    // The 00576670 SetName is absent: the text node keeps no name.
    Ok(true)
}

/// IDA: vt+8 of the reached kinds. Returns the original non-null node test. Completion: 100% for the four kinds.
fn build_node(cx: &mut InterfaceCx, tile: TileId) -> Result<bool, GameError> {
    match cx.engine.tile(tile).kind {
        TileKind::Rect => build_rect(cx, tile),
        // 00591C20: the rect node, then 00707530(2) on the node (591c2a, its +0x9C list) and, when found, +0x54 += 1 and
        // alpha 0. 00591DC0 attaches its material to the shape (5920b0) and none to the node, so nothing is found and
        // nothing changes. The C++ differs: it writes the shape's material.
        TileKind::Menu => build_rect(cx, tile),
        TileKind::Image => build_image(cx, tile),
        TileKind::Text => build_text(cx, tile),
        TileKind::Model => build_model(cx, tile),
    }
}

/// IDA: 00590C20 Tile3D vt+8: a NiNode under the nearest node or Main Root, 00590970, the "Tileptr" extra. It names nothing. Completion: 100% of the order.
fn build_model(cx: &mut InterfaceCx, tile: TileId) -> Result<bool, GameError> {
    let node = spawn_scene_node(cx.world, SceneNodePayload::default());
    record_mut(cx, tile).node = Some(node);
    let parent = cx.engine.tile(tile).parent;
    if let Some(parent) = nearest_node(cx, parent).or_else(|| main_root(cx)) {
        require_attached(
            attach_scene_child(cx.world, parent, Some(node), true),
            "00590C8B: the child was not attached",
        )?;
    }
    tile_load_model_counted(cx, tile, node)?;
    record_mut(cx, tile).extra_tile = Some(tile);
    Ok(true)
}

/// IDA: 00590970: the path at +0x50 from the filename trait, "%s" with "\\Data" in it, else "Meshes\\Menus\\%s"; then locus = 2.0, the node's flag 1 by visible 1.0, the flags `^ 0x40 | 9`, and +0x7C = 1. **A counted boundary:** 00439EB0's load, its clone (00480820, 004430C0, 00700610), +0x40's controller, 00590810, the 0x5C material and alpha property, and the attach aren't run, so the node has no child where `run-h-phase17` has one. It stays culled until a game runs, and no main thread draw comes from it, since the port reaches the original's rand state without it. Completion: all but the model.
/// TODO: [ ] The model, with Tile3D's menus (Phase 21).
fn tile_load_model_counted(
    cx: &mut InterfaceCx,
    tile: TileId,
    node: Entity,
) -> Result<(), GameError> {
    let file = super::tile_values::tile_string_trait(cx, tile, super::tile_values::TRAIT_FILENAME)
        .unwrap_or_default();
    let path = if file.windows(5).any(|part| part == b"\\Data") {
        file
    } else {
        [b"Meshes\\Menus\\".as_slice(), &file].concat()
    };
    cx.engine.tile_mut(tile).model_path_50 = Some(path);
    super::tile_values::tile_set_numeric_trait(cx, tile, TRAIT_LOCUS, 2.0)?;
    let culled = trait_of(cx, tile, TRAIT_VISIBLE) == 1.0;
    set_app_culled(cx, node, culled);
    let flags = &mut cx.engine.tile_mut(tile).flags_2c;
    *flags = (*flags ^ 0x40) | 9;
    if let Some(mut manager) = cx.world.get_resource_mut::<InterfaceManager>() {
        manager.geometry_changed_07c = 1;
    }
    Ok(())
}

/// IDA: 00591030 the texture path and scale for a filename: "Data..." as is, "\Textures" / "Textures" / "\Menus" /
/// "Menus" / "\" / other prefixes completed to Data\Textures\Menus..., the Menus80 and
/// Menus50 variants when H / h_v <= 0.8 (with zoom >= 0) / 0.5, FindFile, and a fallback to the plain path with
/// scale 1. Returns the path and the scale, which is stored even when the file is not found.
/// Completion: 100%.
fn resolve_texture_path(
    cx: &mut InterfaceCx,
    source: &[u8],
    zoom: f32,
    scale: &mut f32,
) -> Vec<u8> {
    if source.is_empty() || source[0] == b' ' {
        return Vec::new();
    }
    let concat = |prefix: &[u8]| [prefix, source].concat();
    let buffer = if source.starts_with(b"Data") {
        source.to_vec()
    } else if source.starts_with(b"\\Textures") {
        concat(b"Data")
    } else if source.starts_with(b"Textures") {
        concat(b"Data\\")
    } else if source.starts_with(b"\\Menus") {
        concat(b"Data\\Textures")
    } else if source.starts_with(b"Menus") {
        concat(b"Data\\Textures\\")
    } else if source.starts_with(b"\\") {
        concat(b"Data\\Textures\\Menus")
    } else {
        concat(b"Data\\Textures\\Menus\\")
    };
    let mut menus80 = buffer.clone();
    let mut menus50 = buffer.clone();
    if let Some(at) = buffer.windows(7).position(|window| window == b"\\Menus\\") {
        let rest = &buffer[at + 7..];
        menus80 = [&b"Data\\Textures\\Menus80\\"[..], rest].concat();
        menus50 = [&b"Data\\Textures\\Menus50\\"[..], rest].concat();
    }
    let ratio = cx.settings.renderer_height as f32 / ui_virtual_height(cx.settings);
    *scale = 1.0;
    let mut chosen = &buffer;
    if f64::from(ratio) <= 0.5 {
        chosen = &menus50;
        *scale = 0.5;
    } else if ratio <= 0.8 && zoom >= 0.0 {
        chosen = &menus80;
        *scale = 0.8;
    }
    if cx.host.file_exists(&lift_text(chosen)) {
        return chosen.clone();
    }
    if chosen != &buffer && cx.host.file_exists(&lift_text(&buffer)) {
        *scale = 1.0;
        return buffer;
    }
    Vec::new()
}

/// IDA: 58ebfa..58ec26 and 58f038..58f06c: the filename trait is non-empty and not " " (_stricmp with A36430), which is
/// the empty text 0058CA50 stores. Completion: 100%.
fn stored_file_name(cx: &InterfaceCx, tile: TileId) -> bool {
    tile_string_trait(cx, tile, TRAIT_FILENAME)
        .is_some_and(|text| !text.is_empty() && stricmp(&text, b" ") != 0)
}

/// IDA: 0058EAAF..0058EE51 the image texture load (flag 0x20). 0058EB53 00591360: the palette is a host omission (no
/// texture reuse). Completion: 100% of the traits and flags.
fn load_image_texture(
    cx: &mut InterfaceCx,
    tile: TileId,
    changed: &mut bool,
) -> Result<(), GameError> {
    let textured = cx
        .engine
        .tile(tile)
        .scene_24
        .as_ref()
        .and_then(|record| record.geometry.first())
        .is_some_and(|geometry| geometry.has_texturing);
    if !textured {
        return Ok(());
    }
    let Some(filename) = tile_string_trait(cx, tile, TRAIT_FILENAME) else {
        return Ok(());
    };
    let zoom = trait_of(cx, tile, TRAIT_ZOOM);
    let mut scale = cx.engine.tile(tile).image_scale_40;
    let path = resolve_texture_path(cx, &filename, zoom, &mut scale);
    cx.engine.tile_mut(tile).image_scale_40 = scale;
    // 0058EB53 00591360's first arm: a screenshot set by 591A80 is taken as it is.
    let preview = cx.engine.tile(tile).image_override_44.clone();
    let size = if let Some(preview) = &preview {
        Some([preview.width, preview.height])
    } else if path.is_empty() {
        None
    } else {
        let world = &mut *cx.world;
        let lifted = lift_text(&path);
        cx.runtime
            .tile_texture_size(world, &lifted)
            .unwrap_or_else(|| cx.host.texture_size(&lifted))
    };
    let Some(size) = size else {
        if !path.is_empty() {
            cx.host.warning(&format!(
                "*** ERROR: Could not create ptexture '{}'!",
                lift_text(&path)
            ));
        }
        return Ok(());
    };
    let clamp = if trait_of(cx, tile, TRAIT_TILE_MODE) == 2.0 {
        3
    } else {
        0
    };
    let geometry = &mut record_mut(cx, tile).geometry[0];
    geometry.texture = path;
    geometry.preview = preview;
    geometry.texture_size = size;
    geometry.clamp = clamp;
    let mut zoom = trait_of(cx, tile, TRAIT_ZOOM);
    let named = stored_file_name(cx, tile);
    let width = size[0] as f32;
    let height = size[1] as f32;
    let image_scale = cx.engine.tile(tile).image_scale_40;
    if trait_of(cx, tile, TRAIT_TILE_MODE) == -1.0 {
        if named {
            tile_set_numeric_trait(cx, tile, TRAIT_FILE_WIDTH, width / image_scale)?;
            tile_set_numeric_trait(cx, tile, TRAIT_FILE_HEIGHT, height / image_scale)?;
        } else {
            tile_set_numeric_trait(cx, tile, TRAIT_FILE_WIDTH, width)?;
            tile_set_numeric_trait(cx, tile, TRAIT_FILE_HEIGHT, height)?;
        }
        cx.engine.tile_mut(tile).flags_2c |= 0x10;
    } else if zoom >= 0.0 {
        if zoom == 0.0 {
            zoom = 100.0;
        }
        let factor = zoom / 100.0;
        if named {
            tile_set_numeric_trait(cx, tile, TRAIT_FILE_WIDTH, width * factor / image_scale)?;
            tile_set_numeric_trait(cx, tile, TRAIT_FILE_HEIGHT, height * factor / image_scale)?;
        } else {
            tile_set_numeric_trait(cx, tile, TRAIT_FILE_WIDTH, width * factor)?;
            tile_set_numeric_trait(cx, tile, TRAIT_FILE_HEIGHT, height * factor)?;
        }
        cx.engine.tile_mut(tile).flags_2c |= 0x10;
    }
    *changed = true;
    cx.engine.tile_mut(tile).flags_2c &= !0x20;
    Ok(())
}

/// IDA: 0058EE59..0058F2A9 the geometry resize (flag 0x10). Completion: 100% of the values the D3 record holds.
fn resize_geometry(cx: &mut InterfaceCx, tile: TileId, is_image: bool, changed: &mut bool) {
    let width = trait_of(cx, tile, TRAIT_WIDTH);
    let height = trait_of(cx, tile, TRAIT_HEIGHT);
    let crop_x = trait_of(cx, tile, TRAIT_CROP_X);
    let crop_y = trait_of(cx, tile, TRAIT_CROP_Y);
    let tile_mode = trait_of(cx, tile, TRAIT_TILE_MODE);
    let mut zoom = trait_of(cx, tile, TRAIT_ZOOM);
    let stored = stored_file_name(cx, tile);
    let image_scale = cx.engine.tile(tile).image_scale_40;
    let record = record_mut(cx, tile);
    let Some(geometry) = record.geometry.first_mut() else {
        return;
    };
    let bottom = -height;
    geometry.vertices = [
        0.0, 0.0, 0.0, 0.0, 0.0, bottom, width, 0.0, 0.0, width, 0.0, bottom,
    ];
    let mut span_u = width;
    let mut span_v = height;
    if geometry.has_texturing
        && is_image
        && (!geometry.texture.is_empty() || geometry.preview.is_some())
    {
        geometry.clamp = if tile_mode == 2.0 { 3 } else { 0 };
        // 58f006 `test ah, 1`: below 0, or a NaN, keeps the tile's size.
        if tile_mode == -1.0 || zoom.lt(&0.0) || zoom.is_nan() {
            span_u = width;
            span_v = height;
        } else {
            if zoom == 0.0 {
                zoom = 100.0;
            }
            let factor = zoom / 100.0;
            let texture_width = geometry.texture_size[0] as f32;
            let texture_height = geometry.texture_size[1] as f32;
            if stored {
                span_u = texture_width * factor / image_scale;
                span_v = texture_height * factor / image_scale;
            } else {
                span_u = texture_width * factor;
                span_v = texture_height * factor;
            }
        }
    }
    if geometry.has_uvs {
        let u0 = crop_x / span_u;
        let v0 = crop_y / span_v;
        let du = width / span_u;
        let dv = height / span_v;
        geometry.uvs = [u0, v0, u0, dv + v0, u0 + du, v0, u0 + du, v0 + dv];
    }
    // 0058F263 data +0x2E |= 9, the bound, then (& 0xFFF) | 0x8000.
    geometry.data_flags_2e |= 9;
    geometry.data_flags_2e = (geometry.data_flags_2e & 0xFFF) | 0x8000;
    *changed = true;
    cx.engine.tile_mut(tile).flags_2c &= !0x10;
}

/// IDA: 00589430: descendants with clipwindow 2 get flag 0x100; others recurse. Completion: 100%.
fn mark_clip_descendants(cx: &mut InterfaceCx, tile: TileId) {
    for child in cx.engine.tile(tile).children_30.clone() {
        let clip = find_value(cx, child, TRAIT_CLIP_WINDOW);
        if clip.is_some_and(|value| cx.engine.value(value).number_04 == 2.0) {
            cx.engine.tile_mut(child).flags_2c |= 0x100;
        } else {
            mark_clip_descendants(cx, child);
        }
    }
}

/// IDA: 0058F2EA..0058F5BF the translate update (flag 1). Completion: 100%.
#[allow(clippy::assign_op_pattern)] // 0058F2EA's operand order: the trait first.
fn update_position(cx: &mut InterfaceCx, tile: TileId, node: Entity) {
    let own = cx.engine.tile(tile);
    let root_child = own.parent.is_some() && own.parent == interface_menu_root(cx);
    let (type_id, unknown_06) = (own.type_id(), own.unknown_06);
    if trait_of(cx, tile, TRAIT_LOCUS) == 2.0 || root_child {
        let mut point = [
            trait_of(cx, tile, TRAIT_X),
            0.0,
            -trait_of(cx, tile, TRAIT_Y),
        ];
        point[1] = trait_of(cx, tile, TRAIT_DEPTH) * -0.008;
        if type_id == 904 {
            point[1] -= trait_of(cx, tile, TRAIT_DEPTH_3D);
        }
        if root_child {
            point[0] -= ui_virtual_width(cx.settings) * 0.5;
            let lifted = ui_virtual_height(cx.settings) + point[2];
            point[2] = lifted - ui_virtual_height(cx.settings) * 0.5;
        }
        set_node_translation(cx, node, point);
        if unknown_06 == 0 {
            for geometry in &mut record_mut(cx, tile).geometry {
                geometry.translate_54 = [0.0; 3];
            }
        }
        mark_clip_descendants(cx, tile);
    } else {
        let depth = trait_of(cx, tile, TRAIT_DEPTH);
        set_node_translation(cx, node, [0.0; 3]);
        let mut point = [0.0f32; 3];
        point[0] = trait_of(cx, tile, TRAIT_X) + point[0];
        point[2] -= trait_of(cx, tile, TRAIT_Y);
        point[1] = depth * -0.008 + point[1];
        if type_id == 904 {
            point[1] -= trait_of(cx, tile, TRAIT_DEPTH_3D);
        }
        for geometry in &mut record_mut(cx, tile).geometry {
            geometry.translate_54 = point;
        }
    }
}

/// IDA: N/A host helper. Completion: 100%.
fn set_node_translation(cx: &mut InterfaceCx, node: Entity, point: [f32; 3]) {
    cx.world
        .get_mut::<SceneNodePayload>(node)
        .expect("a tile node has a payload")
        .local_transform
        .translation = point;
}

/// IDA: 0058F8A5..0058FA68 the colour update (flag 8): materials get +0x54 += 2, alpha and emissive RGB (/ 255); a
/// material-less shape with vertex colours (unless an HTML text) gets the colour on every vertex.
/// Completion: 100%.
fn update_color(cx: &mut InterfaceCx, tile: TileId, changed: &mut bool) {
    let alpha = trait_of(cx, tile, TRAIT_ALPHA) / 255.0;
    let red = trait_of(cx, tile, TRAIT_RED) / 255.0;
    let green = trait_of(cx, tile, TRAIT_GREEN) / 255.0;
    let blue = trait_of(cx, tile, TRAIT_BLUE) / 255.0;
    let own = cx.engine.tile(tile);
    let html = own.kind == TileKind::Text && own.html_50 != 0;
    let mut clear = false;
    for geometry in &mut record_mut(cx, tile).geometry {
        if geometry.has_material {
            geometry.material_revision_54 += 2;
            geometry.alpha = alpha;
            geometry.emissive = [red, green, blue];
        } else if geometry.has_vertex_colors {
            if !html {
                geometry.vertex_color = [red, green, blue, alpha];
                geometry.data_flags_2e |= 4;
            }
        } else {
            continue;
        }
        *changed = true;
        clear = true;
    }
    if clear {
        cx.engine.tile_mut(tile).flags_2c &= !8;
    }
}

/// IDA: 00588C50 Tile_GetCumulativeXFromLocusAncestors and 00588CF0's Y: the trait plus every ancestor's whose locus (4006) isn't 0, the ancestor's first. Completion: 100%.
#[must_use]
#[allow(clippy::assign_op_pattern)] // The operand order: the ancestor's first.
pub fn tile_cumulative_position(cx: &InterfaceCx, tile: TileId) -> [f32; 2] {
    let mut x = trait_of(cx, tile, TRAIT_X);
    let mut y = trait_of(cx, tile, TRAIT_Y);
    let mut parent = cx.engine.tile(tile).parent;
    while let Some(id) = parent {
        if find_value(cx, id, TRAIT_LOCUS)
            .is_some_and(|value| cx.engine.value(value).number_04 != 0.0)
        {
            x = trait_of(cx, id, TRAIT_X) + x;
            y = trait_of(cx, id, TRAIT_Y) + y;
        }
        parent = cx.engine.tile(id).parent;
    }
    [x, y]
}

/// IDA: 0058E870's 0x100 arm: the tile's rectangle in renderer pixels: (x, y, x + width, y + height) times renderer over virtual size, each product rounding at 24 bits. Completion: 100%.
fn clip_rect(cx: &InterfaceCx, tile: TileId) -> [f32; 4] {
    let scale_x = cx.settings.renderer_width as f32 / ui_virtual_width(cx.settings);
    let scale_y = cx.settings.renderer_height as f32 / ui_virtual_height(cx.settings);
    let [x, y] = tile_cumulative_position(cx, tile);
    let bottom = (trait_of(cx, tile, TRAIT_HEIGHT) + y) * scale_y;
    let right = (trait_of(cx, tile, TRAIT_WIDTH) + x) * scale_x;
    [scale_x * x, y * scale_y, right, bottom]
}

/// IDA: 58B773..58B79D and 58F83E..58F866: four __ftol2_sse before 4A17F0: +0xC0..+0xCC are ints, so a rectangle truncates to whole pixels. They're kept as floats here for the draw's arithmetic. Completion: 100%.
fn scissor_words(rect: [f32; 4]) -> [f32; 4] {
    rect.map(|value| x87::ftol(value) as f32)
}

/// IDA: 4A17F0 BSScissorTriShape's store on each of the node's scissor shapes. Completion: 100%.
fn set_scissor(cx: &mut InterfaceCx, tile: TileId, rect: [f32; 4]) {
    let words = scissor_words(rect);
    for geometry in &mut record_mut(cx, tile).geometry {
        if geometry.original_vtable == TILE_SCISSOR_TRI_SHAPE_VTABLE {
            geometry.scissor_c0 = words;
        }
    }
}

/// IDA: 0058B670: over all descendants, those whose clips (4040) is 2 take the rectangle. Completion: 100%.
fn set_descendant_scissors(cx: &mut InterfaceCx, tile: TileId, rect: [f32; 4]) {
    for child in cx.engine.tile(tile).children_30.clone() {
        if find_value(cx, child, TRAIT_CLIPS)
            .is_some_and(|value| cx.engine.value(value).number_04 == 2.0)
        {
            set_scissor(cx, child, rect);
        }
        set_descendant_scissors(cx, child, rect);
    }
}

/// IDA: 0058E870 Tile_UpdateSceneNode. Geometry per the D3 substitution. Completion: 100% of the flag order and
/// clearing rules.
pub fn interface_update_tile_scene(cx: &mut InterfaceCx, tile: TileId) -> Result<bool, GameError> {
    if !cx.world.contains_resource::<InterfaceManager>() {
        return Err(GameError::unclassified(
            0x0058E870,
            "0058E870: no InterfaceManager",
        ));
    }
    if cx.engine.tile(tile).flags_2c == 0 {
        return Ok(false);
    }
    let mut changed = false;
    let is_image = cx.engine.tile(tile).type_id() == 902;
    // 0058E8B2..0058E902: a clips change of an image with a node rebuilds it when the scissor byte disagrees.
    if tile_scene_node(cx, tile).is_some() && cx.engine.tile(tile).flags_2c & 0x200 != 0 && is_image
    {
        let clips = trait_of(cx, tile, TRAIT_CLIP_WINDOW) == 2.0;
        let scissor = cx.engine.tile(tile).byte_48 != 0;
        if clips != scissor {
            cx.engine.tile_mut(tile).flags_2c |= 2;
        }
    }
    // 0058E90A..0058E956 rebuild (flag 2).
    if cx.engine.tile(tile).flags_2c & 2 != 0 {
        if let Some(menu) = cx.engine.containing_menu(tile)
            && cx.engine.menu(menu).id() != 1007
        {
            cx.host.loading_update_display();
        }
        release_node(cx, tile);
        if build_node(cx, tile)? {
            changed = true;
            cx.engine.tile_mut(tile).flags_2c &= !2;
        }
    }
    // 0058E959 `test byte [esi+0x2C], 0x80` and a non-null +0x28: the trait transitions, from the list's head again after each that ends. The C++ tests bit 31, which is not the exe's bit.
    if cx.engine.tile(tile).flags_2c & 0x80 != 0 {
        super::interface_frame::tile_run_trait_transitions(cx, tile)?;
    }
    let node = tile_scene_node(cx, tile);
    // 0058E98C visibility (flag 4).
    if cx.engine.tile(tile).flags_2c & 4 != 0
        && let Some(node) = node
    {
        let culled =
            trait_of(cx, tile, TRAIT_VISIBLE) == 1.0 || trait_of(cx, tile, TRAIT_LIST_CLIP) == 2.0;
        set_app_culled(cx, node, culled);
        changed = true;
        cx.engine.tile_mut(tile).flags_2c &= !4;
    }
    // 0058E9F0..0058EA25 culled: the node's or an ancestor node's bit 0.
    let mut culled = true;
    if let Some(node) = node {
        let mut current = node;
        loop {
            culled = cx
                .world
                .get::<SceneNodePayload>(current)
                .is_some_and(|payload| payload.effective_av_flags & 1 != 0);
            if culled {
                break;
            }
            let Some(parent) = cx.world.get::<NiAvParent>(current).and_then(|link| link.0) else {
                break;
            };
            current = parent;
        }
    }
    // 0058EA44 the texture purge (+0xB8) is 0 at startup.
    let purge = cx
        .world
        .get_resource::<InterfaceManager>()
        .is_some_and(|manager| manager.purge_textures_0b8 != 0);
    if purge
        && culled
        && is_image
        && let Some(geometry) = record_mut(cx, tile).geometry.first_mut()
    {
        geometry.texture.clear();
        cx.engine.tile_mut(tile).flags_2c |= 0x20;
    }
    // 0058E* flag 0x40: a filename set after the build reloads Tile3D's model, unless culled.
    if cx.engine.tile(tile).flags_2c & 0x40 != 0
        && !culled
        && let Some(node) = node
    {
        if cx.engine.tile(tile).type_id() == 904 {
            tile_load_model_counted(cx, tile, node)?;
        }
        changed = true;
        cx.engine.tile_mut(tile).flags_2c &= !0x40;
    }
    // 0058EAA5 image texture (flag 0x20).
    if is_image
        && cx.engine.tile(tile).flags_2c & 0x20 != 0
        && node.is_some()
        && (!culled || cx.engine.in_scene_update)
    {
        load_image_texture(cx, tile, &mut changed)?;
    }
    // 0058EE59 resize (flag 0x10).
    if cx.engine.tile(tile).flags_2c & 0x10 != 0 && node.is_some() && !culled {
        resize_geometry(cx, tile, is_image, &mut changed);
    }
    // 0058F2A9 3D (flag 0x40).
    if cx.engine.tile(tile).flags_2c & 0x40 != 0 && node.is_some() && !culled {
        changed = true;
        cx.engine.tile_mut(tile).flags_2c &= !0x40;
    }
    // 0058F2EA position (flag 1).
    if cx.engine.tile(tile).flags_2c & 1 != 0
        && !culled
        && let Some(node) = node
    {
        update_position(cx, tile, node);
        changed = true;
        cx.engine.tile_mut(tile).flags_2c &= !1;
    }
    if node.is_some() {
        // 0058F5D3 clip window (flag 0x100): the original clears 0x100 only when flag 1 is set (0058F6A5).
        if cx.engine.tile(tile).flags_2c & 0x100 != 0 {
            let rect = clip_rect(cx, tile);
            set_descendant_scissors(cx, tile, rect);
            changed = true;
            if cx.engine.tile(tile).flags_2c & 1 != 0 {
                cx.engine.tile_mut(tile).flags_2c ^= 0x100;
            }
        }
        // 0058F6BA scissor rectangles (flag 0x200).
        if cx.engine.tile(tile).flags_2c & 0x200 != 0 {
            // The nearest ancestor whose clipwindow (4004) is 2, whose rectangle goes on this tile's scissor shapes.
            let mut parent = cx.engine.tile(tile).parent;
            while let Some(id) = parent {
                if find_value(cx, id, TRAIT_CLIP_WINDOW)
                    .is_some_and(|value| cx.engine.value(value).number_04 == 2.0)
                {
                    let rect = clip_rect(cx, id);
                    set_scissor(cx, tile, rect);
                    break;
                }
                parent = cx.engine.tile(id).parent;
            }
            changed = true;
            cx.engine.tile_mut(tile).flags_2c &= !0x200;
        }
    }
    // 0058F8A5 colour (flag 8).
    if cx.engine.tile(tile).flags_2c & 8 != 0 && !culled && node.is_some() {
        update_color(cx, tile, &mut changed);
    }
    Ok(changed)
}

/// IDA: 00591A80 TileImage_SetTexture: filename = the empty text, +0x44 = the texture, flags |= 0x30. Completion: 100%.
pub(in crate::tes) fn image_set_preview(
    cx: &mut InterfaceCx,
    tile: TileId,
    preview: Option<std::sync::Arc<SavePreview>>,
) -> Result<(), GameError> {
    tile_set_string_trait(cx, tile, TRAIT_FILENAME, b"")?;
    let tile = cx.engine.tile_mut(tile);
    tile.image_override_44 = preview;
    tile.flags_2c |= 0x30;
    Ok(())
}

#[cfg(test)]
mod tests {
    // 58B777: `__ftol2_sse`, then `push eax` into 4A17F0: truncated, not rounded, so the left edge of 405.9 is 405.
    #[test]
    fn a_scissor_is_whole_pixels() {
        assert_eq!(
            super::scissor_words([405.9, 247.5, 870.99, 820.25]),
            [405.0, 247.0, 870.0, 820.0]
        );
    }

    use super::*;
    use crate::gamebryo::scene_update::NoSceneControllers;
    use crate::tes::interface_manager::{InterfaceSettings, interface_settings_from};
    use crate::tes::settings::{SettingCollections, register_static_settings};
    use crate::tes::tile::{NullInterfaceHost, with_interface_cx};

    fn settings() -> InterfaceSettings {
        let mut collections = SettingCollections::default();
        register_static_settings(&mut collections);
        interface_settings_from(&collections.ini, 1920, 1080, 8, Default::default())
    }

    // 00591C20 looks on the node (00707530, +0x9C), and 00591DC0 put the material on the shape: a menu's material keeps
    // its alpha and its revision of 4, as a rect's does. The old code wrote alpha 0 and revision 5 into the shape.
    #[test]
    fn a_menu_builds_as_a_rect() {
        let mut world = World::new();
        world.insert_resource(InterfaceManager::default());
        let mut host = NullInterfaceHost::default();
        with_interface_cx(
            &mut world,
            &settings(),
            &mut host,
            &mut NoSceneControllers,
            |cx| {
                let tile = tile_initialize(cx, TileKind::Menu, None, None, None).unwrap();
                tile_set_numeric_trait(cx, tile, TRAIT_ALPHA, 128.0).unwrap();
                build_node(cx, tile).unwrap();
                let geometry = &cx.engine.tile(tile).scene_24.as_ref().unwrap().geometry[0];
                assert_eq!(geometry.material_revision_54, 4);
                assert_eq!(geometry.alpha, 1.0);
            },
        );
    }

    // 0058E959 tests flag 0x80 and a +0x28 list. Bit 31 is not that flag, and without a list nothing warns.
    #[test]
    fn bit_31_is_not_the_transition_flag() {
        let mut world = World::new();
        world.insert_resource(InterfaceManager::default());
        let mut host = NullInterfaceHost::default();
        with_interface_cx(
            &mut world,
            &settings(),
            &mut host,
            &mut NoSceneControllers,
            |cx| {
                let tile = tile_initialize(cx, TileKind::Rect, None, None, None).unwrap();
                cx.engine.tile_mut(tile).flags_2c = 0x8000_0000;
                interface_update_tile_scene(cx, tile).unwrap();
            },
        );
        assert!(host.warnings.is_empty());
    }
}
