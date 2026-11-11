//! M6: 0057F170's drawing as a list of quads, the D3 substitution's other half. The original culls the scene graph under `MenuRoot`, its NiAlphaAccumulator sorts the shapes back to front, and the renderer draws them. Here the tile records are walked with the nodes' transforms and the same camera, and `oblivion_host` draws the list. The list is plain data, so a test reads it without a window. Main thread after CreateDevice: 24-bit x87.

use bevy_ecs::prelude::*;

use super::interface_frame::{tile_hierarchy_visible, top_stacked_menu_mode};
use super::interface_manager::{InterfaceManager, ui_virtual_height, ui_virtual_width};
use super::interface_scene::{SavePreview, TileSceneGeometry};
use super::tile::{InterfaceCx, TileEngine, TileId};
use super::tile_values::{TRAIT_ID, TRAIT_TARGET_ID, find_value, tile_numeric_trait};
use crate::gamebryo::scene_node::SceneNodePayload;

/// IDA: N/A a texture as the host fetches it: a path through the archives, a font's pixel data, or a save's screenshot.
#[derive(Clone, Debug, PartialEq)]
pub enum QuadTexture {
    /// A rect, or an image with no texture: NiTexturingProperty with no base map draws the material alone.
    None,
    File(Vec<u8>),
    Font(Entity),
    Preview(std::sync::Arc<SavePreview>),
}

/// IDA: one shape's quad, projected. `positions` are renderer pixels, top left, bottom left, top right, bottom right. `color` is the material's emissive and alpha, or the vertex colour (00574200). `clamp` is NiTexturingProperty's mode 0 (clamp) or 3 (wrap). `scissor` is 4A17F0's rectangle when it isn't empty. Completion: 100% of the values the D3 record holds.
#[derive(Clone, Debug, PartialEq)]
pub struct InterfaceQuad {
    pub tile: TileId,
    pub texture: QuadTexture,
    pub positions: [[f32; 2]; 4],
    pub uvs: [[f32; 2]; 4],
    pub color: [f32; 4],
    pub clamp: u32,
    pub scissor: Option<[f32; 4]>,
    /// The camera's y of the shape: what the accumulator sorts.
    pub depth: f32,
}

/// IDA: N/A what a draw needs from the World and the settings.
#[derive(Clone, Copy, Debug)]
pub struct InterfaceView {
    pub renderer_width: f32,
    pub renderer_height: f32,
    pub virtual_width: f32,
    pub virtual_height: f32,
    /// +0x74.
    pub camera_distance: f32,
}

#[derive(Clone, Copy)]
struct Placement {
    translation: [f32; 3],
    scale: f32,
}

// IDA: 00707370's world transform for an unrotated node: parent translation + parent scale * local, scale * local scale. Completion: 100% of what a tile node holds: nothing rotates one.
fn compose(parent: Placement, local: [f32; 3], scale: f32) -> Placement {
    Placement {
        translation: [
            parent.translation[0] + parent.scale * local[0],
            parent.translation[1] + parent.scale * local[1],
            parent.translation[2] + parent.scale * local[2],
        ],
        scale: parent.scale * scale,
    }
}

// IDA: the camera the Phase 05 graph sets (0057ED86 and SceneGraph_SetCameraFOV): the view looks along +y from the origin, so a point at y scales by +0x74 / y, which is 1 on the main root's plane; 0057F490's own mapping gives the pixels. Completion: 100% of the arithmetic; the camera's matrix itself is a host reading of P05-RUN-001's FOV, a no-data item.
fn project(view: &InterfaceView, point: [f32; 3]) -> [f32; 2] {
    let perspective = view.camera_distance / point[1];
    [
        view.renderer_width * 0.5
            + view.renderer_width / view.virtual_width * (point[0] * perspective),
        view.renderer_height * 0.5
            - view.renderer_height / view.virtual_height * (point[2] * perspective),
    ]
}

// IDA: N/A the node's own values, or those of a tile with none.
fn node_state(world: &World, node: Option<Entity>) -> ([f32; 3], f32, bool) {
    node.and_then(|node| world.get::<SceneNodePayload>(node))
        .map_or(([0.0; 3], 1.0, false), |payload| {
            (
                payload.local_transform.translation,
                payload.local_transform.scale,
                payload.effective_av_flags & 1 != 0,
            )
        })
}

// IDA: N/A a helper, with no address of its own.
fn emit_geometry(
    view: &InterfaceView,
    tile: TileId,
    placement: Placement,
    geometry: &TileSceneGeometry,
    out: &mut Vec<InterfaceQuad>,
) {
    let shape = compose(placement, geometry.translate_54, 1.0);
    let scissor = (geometry.scissor_c0 != [0.0; 4]).then_some(geometry.scissor_c0);
    if !geometry.glyphs.is_empty() {
        let texture = geometry
            .font_texture
            .map_or(QuadTexture::None, QuadTexture::Font);
        for glyph in &geometry.glyphs {
            let mut positions = [[0.0; 2]; 4];
            for (position, vertex) in positions.iter_mut().zip(glyph.positions) {
                *position = project(
                    view,
                    [
                        shape.translation[0] + shape.scale * vertex[0],
                        shape.translation[1],
                        shape.translation[2] + shape.scale * vertex[1],
                    ],
                );
            }
            out.push(InterfaceQuad {
                tile,
                texture: texture.clone(),
                positions,
                uvs: glyph.uvs,
                color: geometry.vertex_color,
                clamp: 0,
                scissor,
                depth: shape.translation[1],
            });
        }
        return;
    }
    let vertex = |index: usize| {
        project(
            view,
            [
                shape.translation[0] + shape.scale * geometry.vertices[index * 3],
                shape.translation[1] + shape.scale * geometry.vertices[index * 3 + 1],
                shape.translation[2] + shape.scale * geometry.vertices[index * 3 + 2],
            ],
        )
    };
    let texture = if let Some(preview) = &geometry.preview {
        QuadTexture::Preview(preview.clone())
    } else if geometry.texture.is_empty() {
        QuadTexture::None
    } else {
        QuadTexture::File(geometry.texture.clone())
    };
    let uvs = if geometry.has_uvs {
        [
            [geometry.uvs[0], geometry.uvs[1]],
            [geometry.uvs[2], geometry.uvs[3]],
            [geometry.uvs[4], geometry.uvs[5]],
            [geometry.uvs[6], geometry.uvs[7]],
        ]
    } else {
        [[0.0; 2]; 4]
    };
    let color = if geometry.has_vertex_colors {
        geometry.vertex_color
    } else {
        [
            geometry.emissive[0],
            geometry.emissive[1],
            geometry.emissive[2],
            geometry.alpha,
        ]
    };
    out.push(InterfaceQuad {
        tile,
        texture,
        positions: [vertex(0), vertex(1), vertex(2), vertex(3)],
        uvs,
        color,
        clamp: geometry.clamp,
        scissor,
        depth: shape.translation[1],
    });
}

// IDA: N/A a helper, with no address of its own.
fn walk(
    world: &World,
    engine: &TileEngine,
    view: &InterfaceView,
    tile: TileId,
    parent: Placement,
    out: &mut Vec<InterfaceQuad>,
) {
    let node = engine
        .tile(tile)
        .scene_24
        .as_ref()
        .and_then(|record| record.node);
    let (local, scale, culled) = node_state(world, node);
    if culled {
        return;
    }
    let placement = if node.is_some() {
        compose(parent, local, scale)
    } else {
        parent
    };
    if let Some(record) = engine.tile(tile).scene_24.as_ref() {
        for geometry in &record.geometry {
            emit_geometry(view, tile, placement, geometry, out);
        }
    }
    for &child in &engine.tile(tile).children_30 {
        walk(world, engine, view, child, placement, out);
    }
}

/// IDA: 0057F170's main pass (0070C0B0, 00701970, 0070E0A0) as data: under the main root, then the cursor root, whose shapes come last in the graph, and the accumulator's back-to-front order by the camera's y, later shapes first on a tie. A stable sort stands in for the accumulator, a recorded substitution; the tie's direction is the C++ capture's, a no-data item. The cursor tile's node hangs from the cursor root, not from a tile parent. Completion: 100% of the walk over tile shapes.
#[must_use]
pub fn interface_draw_list(
    world: &World,
    engine: &TileEngine,
    view: &InterfaceView,
) -> Vec<InterfaceQuad> {
    let mut out = Vec::new();
    let Some(manager) = world.get_resource::<InterfaceManager>() else {
        return out;
    };
    let root = Placement {
        translation: [0.0, manager.camera_distance_074, 0.0],
        scale: 1.0,
    };
    if let Some(menu_root) = manager.menu_root_068 {
        // The root's own node is culled or absent only in a test: it stops the whole tree, as a culled node does.
        for &child in &engine.tile(menu_root).children_30 {
            walk(world, engine, view, child, root, &mut out);
        }
    }
    out.sort_by(|left, right| right.depth.total_cmp(&left.depth));
    let mut cursor = Vec::new();
    if let Some(tile) = manager.cursor_01c {
        walk(world, engine, view, tile, root, &mut cursor);
    }
    out.extend(cursor);
    out
}

/// IDA: 00581390's walk of the NiPick results (the cursor's node culled first, as the pick skips it): nearest first, the shape's tile; 0057D240 keeps the top stacked menu's; a clipped tile needs the point inside its scissor; the first with target 2 wins, and without the `id` trait its nearest ancestor with one. Host: the draw list's quads for the NiPick, a recorded substitution. Completion: 100% of the tests on a hit.
#[must_use]
pub fn interface_pick(cx: &InterfaceCx, pixel: [f32; 2]) -> Option<TileId> {
    let view = interface_view(cx);
    let quads = interface_draw_list(cx.world, cx.engine, &view);
    let cursor = cx.world.resource::<InterfaceManager>().cursor_01c;
    let stack = cx.world.resource::<InterfaceManager>().menu_mode_stack_0e0;
    let top_tile = cx.engine.tile_menu_by_id(top_stacked_menu_mode(&stack));
    let top_menu = top_tile.and_then(|tile| cx.engine.containing_menu(tile));
    let stacks = top_tile.is_some_and(|tile| {
        let stacking = tile_numeric_trait(cx, tile, super::tile_values::TRAIT_STACKING);
        stacking == 102.0 || stacking == 6006.0
    });
    for quad in quads.iter().rev() {
        if Some(quad.tile) == cursor || !quad_contains(quad, pixel) {
            continue;
        }
        if stacks && cx.engine.containing_menu(quad.tile) != top_menu {
            continue;
        }
        if let Some(scissor) = quad.scissor
            && (scissor[0] > pixel[0]
                || scissor[2] <= pixel[0]
                || scissor[1] > pixel[1]
                || scissor[3] <= pixel[1])
        {
            continue;
        }
        if tile_numeric_trait(cx, quad.tile, TRAIT_TARGET_ID) != 2.0 {
            continue;
        }
        let mut tile = quad.tile;
        if find_value(cx, tile, TRAIT_ID).is_none() {
            let mut parent = cx.engine.tile(tile).parent;
            while let Some(id) = parent {
                if find_value(cx, id, TRAIT_ID).is_some() {
                    tile = id;
                    break;
                }
                parent = cx.engine.tile(id).parent;
            }
        }
        if tile_hierarchy_visible(cx, tile) {
            return Some(tile);
        }
    }
    None
}

// IDA: N/A the screen rectangle of an axis-aligned quad.
fn quad_contains(quad: &InterfaceQuad, pixel: [f32; 2]) -> bool {
    let left = quad.positions[0][0].min(quad.positions[3][0]);
    let right = quad.positions[0][0].max(quad.positions[3][0]);
    let top = quad.positions[0][1].min(quad.positions[3][1]);
    let bottom = quad.positions[0][1].max(quad.positions[3][1]);
    pixel[0] >= left && pixel[0] < right && pixel[1] >= top && pixel[1] < bottom
}

/// IDA: N/A the settings' values for a draw.
#[must_use]
pub fn interface_view(cx: &InterfaceCx) -> InterfaceView {
    InterfaceView {
        renderer_width: cx.settings.renderer_width as f32,
        renderer_height: cx.settings.renderer_height as f32,
        virtual_width: ui_virtual_width(cx.settings),
        virtual_height: ui_virtual_height(cx.settings),
        camera_distance: cx.world.resource::<InterfaceManager>().camera_distance_074,
    }
}
