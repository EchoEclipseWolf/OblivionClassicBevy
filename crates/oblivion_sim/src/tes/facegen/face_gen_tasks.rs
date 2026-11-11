//! QueuedHead, an actor task's FaceGen child: `0043BA30` QueuedHead_Create, its walk (`00435300` QueuedHead_Run, `00435260`
//! the Finalize that notifies the parent) and the head build under it (`00523220`, `0052DED0`, `00555A80`). The map is in
//! `docs/target-a-plan-2026-11-07.md` ("FaceGen (background agent) working notes").
//!
//! Host deferral D12 (narrowed, 2026-11-08): the walk builds each part's morphed positions, exact against run-af's
//! `after_move`; the node tree they go into (the two BSFaceGenNiNodes, the clones with their textures and properties, the
//! animation data), the hair, the eyes and the face texture aren't built yet. QueuedHelmet is `helmet_tasks`.

use std::collections::HashMap;

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::face_gen_head::{FaceDescription, PART_NAMES_B10CA8, fill_face_description};
use crate::bs::face_gen_model::{
    BsFaceGenModelMap, FaceGenFileServices, FaceGenModel, FaceGenModelFiles,
    create_morphed_positions, load_face_gen_model,
};
use crate::bs::model_loader::{IoTaskId, QueuedModel, queue_io_task};
use crate::bs::reference_tasks::ReferenceTaskId;
use crate::bs::task_manager::TaskGlobals;
use crate::tes::character_3d::FaceGenSettings;

/// IDA: QueuedHead's vtable (0x30 task, 0043BA30): vt+4 00435300 Run, vt+8 00435260 (vt+0x28), vt+0x18 004365F0
/// BSTask_SetUnpackedKey, vt+0x20 004346D0 QueuedFile_QueueToIOManager, vt+0x28 00436F30 QueuedFile_NotifyParent.
pub const QUEUED_HEAD_VTABLE: u32 = 0x00A3_6CE4;

/// IDA: one head part `00555A80` built: its index and name (00B10CA8), the model path, which node it goes under (+0xB8
/// a skin instance: the skinned node), and the clone's morphed positions.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceGenPart {
    pub index: usize,
    pub name: &'static str,
    pub model_path: Vec<u8>,
    pub skinned: bool,
    pub positions: Vec<[f32; 3]>,
}

/// IDA: a QueuedHead's +0x20 the NPC base, +0x24 the biped node and +0x28 the skinned node; here the parts that go under
/// them (D12: the nodes aren't built), and what the build left to later work.
#[derive(Clone, Debug, PartialEq)]
pub struct QueuedHead {
    pub base_20: Entity,
    /// +0x24 the BSFaceGenNiNodeBiped and +0x28 the BSFaceGenNiNodeSkinned (`0052DED0`).
    pub biped_24: Option<Entity>,
    pub skinned_28: Option<Entity>,
    pub parts: Vec<FaceGenPart>,
    /// IDA: N/A the steps of `0052DED0` the port records instead of running (D12), by their site.
    pub deferred: Vec<&'static str>,
}

/// IDA: N/A the QueuedHead tasks' own fields, by task (the walk's store holds their BSTask part), and each actor task's
/// +0x38 (the head child `0043DC00` stores). Looked up only, never iterated for order.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct FaceGenTasks {
    pub heads: HashMap<IoTaskId, QueuedHead>,
    pub head_of_actor_38: HashMap<ReferenceTaskId, IoTaskId>,
}

/// IDA: 0043BA30 QueuedHead_Create(out, base, priority, parent): FormHeap(0x30), IOTask_Constructor(priority) (00B33A20 up,
/// the priority in the key's byte 2), +0x18 = +0x1C = 0, the vtable, +0x20 = base, +0x24 = +0x28 = 0; out takes a reference;
/// `QueuedFile_SetParent`(parent) and vt+0x20, QueuedFile_QueueToIOManager: IOManager vt+0x3C with vt+0x18's unpacked key.
/// The caller (0043DCE6) stores out at the actor task's +0x38. Completion: 100%.
/// # Errors
/// No ModelLoader or IOManager, or a released parent.
pub fn create_queued_head(
    world: &mut World,
    base: Entity,
    parent: ReferenceTaskId,
) -> Result<IoTaskId, GameError> {
    let id = {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let TaskGlobals {
            io_manager_b33a10,
            model_loader_b33a1c,
            counters,
            ..
        } = &mut *tasks;
        let loader = model_loader_b33a1c.as_mut().ok_or(GameError::unclassified(
            0x0043BA30,
            "0043BA30: a null g_modelLoader",
        ))?;
        let priority = loader
            .references
            .get(parent)
            .ok_or(GameError::unclassified(
                0x0043AC40,
                "0043AC40: a released parent task",
            ))?
            .priority();
        let io = io_manager_b33a10.as_mut().ok_or(GameError::unclassified(
            0x004346D0,
            "004346D0: a null g_IOManager",
        ))?;
        counters.live_tasks_b33a20 = counters.live_tasks_b33a20.wrapping_add(1);
        let id = IoTaskId(io.tasks.store.len());
        io.tasks.store.push(Some(QueuedModel {
            original_vtable: QUEUED_HEAD_VTABLE,
            references_08: 1,
            state_0c: 0,
            key_10: u64::from(priority) << 16,
            parent_18: None,
            children_1c: 0,
            path_20: Vec::new(),
            entry_24: None,
            model_28: None,
            field_2c: 0,
            lod_30: 0,
            flags_34: 0,
            host_gate: None,
        }));
        crate::bs::model_loader::set_parent(io, loader, id, parent)?;
        queue_io_task(io, counters, id)?;
        id
    };
    let mut tasks = world.get_resource_or_init::<FaceGenTasks>();
    tasks.heads.insert(
        id,
        QueuedHead {
            base_20: base,
            biped_24: None,
            skinned_28: None,
            parts: Vec::new(),
            deferred: Vec::new(),
        },
    );
    tasks.head_of_actor_38.insert(parent, id);
    Ok(id)
}

/// IDA: 00435300 QueuedHead_Run: under 00B39C80 ("QueuedHead::Run()"), `00523220` TESNPC_BuildFaceGenHead(base, &biped,
/// &skinned), the two refcounted into +0x24/+0x28. 00523220: the NPC's +0x1D4/+0x1D8 released (none yet in the port); with a
/// race, `0052DED0`(race, ..., npc, 1, 0); then NPC +0x1E0 = race +0x2FC (u16). Completion: as `build_face_gen_head`.
/// # Errors
/// A stop in the build.
pub fn run_queued_head(
    world: &mut World,
    id: IoTaskId,
    services: &mut FaceGenFileServices<'_>,
    prepare: &mut crate::bs::sky_shader::PrepareGeometryContext<'_>,
) -> Result<(), GameError> {
    let base = world
        .get_resource::<FaceGenTasks>()
        .and_then(|tasks| tasks.heads.get(&id))
        .ok_or(GameError::unclassified(
            0x00435300,
            "00435300: a QueuedHead with no fields",
        ))?
        .base_20;
    let Some(race) = crate::tes::forms_actors::base_race(world, base)? else {
        return Ok(());
    };
    let BuiltHead {
        parts,
        deferred,
        biped,
        skinned,
    } = build_face_gen_head(world, base, true, services, prepare)?;
    let fnam = world
        .get::<crate::tes::forms_actors::TesRace>(race)
        .ok_or(GameError::unclassified(
            0x005232B3,
            "005232B3: a race that isn't a TESRace",
        ))?
        .snam_2fc;
    world
        .get_mut::<crate::tes::forms_actors::TesNpc>(base)
        .ok_or(GameError::unclassified(
            0x005232C0,
            "005232C0: a base that isn't an NPC",
        ))?
        .fnam_1e0 = fnam;
    let mut tasks = world.resource_mut::<FaceGenTasks>();
    let head = tasks.heads.get_mut(&id).expect("tested above");
    head.parts = parts;
    head.deferred = deferred;
    head.biped_24 = Some(biped);
    head.skinned_28 = Some(skinned);
    Ok(())
}

/// IDA: N/A what `build_face_gen_head` made: the parts, the steps left (D12), and `0052DED0`'s two node out-pointers.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltHead {
    pub parts: Vec<FaceGenPart>,
    pub deferred: Vec<&'static str>,
    pub biped: Entity,
    pub skinned: Entity,
}

/// IDA: 0052DED0 TESRace_BuildFaceGenHead(race, biped, skinned, npc, face_texture, 0) and its `00555A80` part loop, as the
/// port runs them (QueuedHead passes 1, `00525A70` TESNPC_CreateHead 0, which skips `005551C0` on both nodes). The face description (`0052CD50`, bFaceGenTexturing 00B10D3C and 00B120B0's skip of the `_50.NIF` head); per part
/// i (the ears by sex: 1 skipped for a female, 2 for a male; no model or texture path skipped): "Meshes\<model>", its .egt,
/// .tri and .egm siblings; parts 0..2 through `00553620` FaceGen_GetModel(egm, nif, tri, egt, 1, 0), 3..6 the same inline
/// with no EGT; 7 and 8 (the eyes) are `005547F0`'s. A model goes through `00559870` for the morphed clone, which goes under
/// the skinned node with a skin instance, else the biped node. Completion: the morphed positions of parts 0..6; the rest is
/// recorded in `QueuedHead::deferred` (D12).
/// # Errors
/// A stop in the description or a file.
pub fn build_face_gen_head(
    world: &mut World,
    npc: Entity,
    face_texture: bool,
    services: &mut FaceGenFileServices<'_>,
    prepare: &mut crate::bs::sky_shader::PrepareGeometryContext<'_>,
) -> Result<BuiltHead, GameError> {
    use super::face_gen_nodes::{spawn_face_gen_animation_data, spawn_face_gen_ni_node};
    // 0052DEF3..0052DFB0: the animation data, then the biped node (vt+0xB0(1), vt+0xB8(1)) and the skinned node (0, 0).
    let animation = spawn_face_gen_animation_data(world);
    let biped = spawn_face_gen_ni_node(world, b"BSFaceGenNiNodeBiped", animation, true);
    let skinned = spawn_face_gen_ni_node(world, b"BSFaceGenNiNodeSkinned", animation, false);
    let settings = world
        .get_resource::<FaceGenHeadSettings>()
        .copied()
        .unwrap_or_default();
    let use_heads = world
        .get_resource_or_init::<FaceGenSettings>()
        .use_face_gen_heads_b120b4;
    let description = fill_face_description(
        world,
        npc,
        settings.face_texturing_b10d3c,
        settings.skip_lod_head_b120b0,
    )?;
    let mut deferred = Vec::new();
    if description.face_texturing_b4 {
        deferred.push("0052CF86: 00524100, the NPC's own face textures");
    }
    let mut parts = Vec::new();
    for index in 0..9 {
        let skip_ear = match index {
            2 => description.sex_70 == 0,
            1 => description.sex_70 == 1,
            _ => false,
        };
        if skip_ear
            || description.model_paths_74[index].is_empty()
            || description.texture_paths_84[index].is_empty()
            || index > 6
        {
            continue;
        }
        if index == 0 && description.skip_lod_head_c0 == 0 {
            return Err(GameError::unclassified(
                0x00555C42,
                "00555C42: 00551B40, the _50.NIF head (00B120B0 is 0)",
            ));
        }
        if let Some((part, geometry, morph)) =
            build_part(world, &description, index, use_heads, services)?
        {
            let texture = description.texture_paths_84[index].clone();
            let clone = super::face_gen_nodes::add_head_part(
                world,
                biped,
                skinned,
                index,
                part.name,
                geometry,
                &part.positions,
                &texture,
                prepare,
            )?;
            world.entity_mut(clone).insert(morph);
            parts.push(part);
        }
    }
    deferred.push("00555FA3: the parts' NiTexturingProperty through TES_CreateTextureImage (D13; the path is kept)");
    if world
        .get_resource_or_init::<FaceGenHeadSettings>()
        .fix_face_normals_b120bc
    {
        deferred.push("005561CF: 00481920, bFixFaceNormals");
    }
    // 005561FC 00554260 FaceGen_BuildHair.
    build_hair(
        world,
        &description,
        biped,
        skinned,
        use_heads,
        services,
        prepare,
    )?;
    // 0055622D 005547F0 FaceGen_BuildEyes, with both eye models (+0xB8, +0xBC).
    build_eyes(
        world,
        &description,
        biped,
        skinned,
        use_heads,
        services,
        prepare,
    )?;
    // 0052E069 005551C0 FaceGen_ApplyFaceTexture on both nodes with a5: the hair's tint (`apply_hair_tint`) and the face and
    // ears arm (`apply_face_texture`) are ported; the eyes' arms are D13's.
    if face_texture {
        for node in [biped, skinned] {
            super::face_gen_nodes::apply_hair_tint(world, node, description.hair_color_64);
            super::face_gen_nodes::apply_face_texture(world, node, &description, services)?;
        }
        deferred.push("0052E069: 005551C0's eye arms (D13)");
    }
    // 0052E074: the player's base (form 7) sets +0x111 on both nodes.
    if world
        .get::<crate::tes::form::FormIdentity>(npc)
        .is_some_and(|identity| identity.form_id_0c == 7)
    {
        for node in [biped, skinned] {
            world
                .get_mut::<super::face_gen_nodes::BsFaceGenNiNode>(node)
                .expect("just made")
                .bytes_110[1] = 1;
        }
    }
    Ok(BuiltHead {
        parts,
        deferred,
        biped,
        skinned,
    })
}

// IDA: 00553620's and 00555A80's first step: a null g_faceGen +0xDAC takes FormHeap(0x20) 00551E80, +0x18 =
// uiFaceGenMaxEGMDataSize (00B120EC) and +0x1C = uiFaceGenMaxEGTDataSize (00B120F4), each followed by 005506B0(0) (a trim
// that finds nothing). A null g_faceGen would run 00553550, which isn't run from here (a stop).
pub(crate) fn ensure_model_map(world: &mut World) -> Result<(), GameError> {
    if world.get_resource::<BsFaceGenModelMap>().is_some() {
        return Ok(());
    }
    let settings = world
        .get_resource::<FaceGenHeadSettings>()
        .copied()
        .unwrap_or_default();
    world
        .get_resource_mut::<crate::tes::facegen::FaceGenSingleton>()
        .and_then(|singleton| singleton.into_inner().0.as_deref_mut())
        .ok_or(GameError::unclassified(
            0x00553680,
            "00553680: g_faceGen is null, and 00553550 isn't run from here",
        ))?
        .model_map_dac = 1;
    world.insert_resource(BsFaceGenModelMap {
        max_egm_size_18: settings.max_egm_size_b120ec,
        max_egt_size_1c: settings.max_egt_size_b120f4,
        ..BsFaceGenModelMap::default()
    });
    Ok(())
}

// IDA: N/A the sibling path of a model path with another extension (005500C0 .egt, 00550010 .tri, 0054FEB0 .egm): the text
// up to the last '.', sprintf "%s.<ext>"; no '.' gives null.
fn sibling(path: &[u8], extension: &[u8]) -> Option<Vec<u8>> {
    let dot = path.iter().rposition(|&byte| byte == b'.')?;
    let mut out = path[..dot].to_vec();
    out.push(b'.');
    out.extend_from_slice(extension);
    Some(out)
}

// IDA: one part of 00555A80 (see `build_face_gen_head`).
fn build_part(
    world: &mut World,
    description: &FaceDescription,
    index: usize,
    use_heads: bool,
    services: &mut FaceGenFileServices<'_>,
) -> Result<Option<(FaceGenPart, Entity, super::face_gen_nodes::FaceGenPartMorph)>, GameError> {
    let mut nif = b"Meshes\\".to_vec();
    nif.extend_from_slice(&description.model_paths_74[index]);
    let Some(model) = morphed_model(
        world,
        &nif,
        index <= 2,
        false,
        &description.coefficients_00,
        use_heads,
        services,
    )?
    else {
        return Ok(None);
    };
    Ok(Some((
        FaceGenPart {
            index,
            name: PART_NAMES_B10CA8[index],
            model_path: nif,
            skinned: model.skinned,
            positions: model.positions,
        },
        model.geometry,
        model.part,
    )))
}

/// IDA: N/A a FaceGen model found or loaded in the model map and morphed: its geometry, the morphed positions, whether
/// it has a skin instance, and its morph data (the hair's).
pub(crate) struct MorphedModel {
    pub(crate) geometry: Entity,
    pub(crate) positions: Vec<[f32; 3]>,
    skinned: bool,
    pub(crate) morph_data: Option<crate::bs::face_gen_model::FaceGenMorphData>,
    /// The model's index in the map and the clone's base morph (its BSFaceGenModelExtraData and
    /// BSFaceGenBaseMorphExtraData).
    pub(crate) part: super::face_gen_nodes::FaceGenPartMorph,
}

// IDA: the model-map step 00553620 (GetModel) and 00555A80's inline copy for parts 3..6 and 00554260's for the hair share:
// the .egm, .tri and (with `with_egt`) .egt siblings; BSFaceGenModelMap_Find, a found model not yet loaded loaded
// (BSFaceGenModel_Load with `hair`), a new one loaded and added only when the load succeeds; then 00559870's morph.
#[allow(clippy::too_many_arguments, reason = "the model-map step's inputs")]
pub(crate) fn morphed_model(
    world: &mut World,
    nif: &[u8],
    with_egt: bool,
    hair: bool,
    coefficients: &[crate::tes::facegen::FaceGenMatrix],
    use_heads: bool,
    services: &mut FaceGenFileServices<'_>,
) -> Result<Option<MorphedModel>, GameError> {
    let tri = sibling(nif, b"tri");
    let Some(egm) = sibling(nif, b"egm") else {
        return Ok(None);
    };
    let egt = if with_egt { sibling(nif, b"egt") } else { None };
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    let nif_text = text(nif);
    let tri_text = tri.as_deref().map(text);
    ensure_model_map(world)?;
    let found = world.resource::<BsFaceGenModelMap>().find(&egm);
    let model = match found {
        Some(model) => {
            let mut held = world.resource::<BsFaceGenModelMap>().models[model].clone();
            if !held.loaded {
                load_face_gen_model(
                    world,
                    &mut held,
                    FaceGenModelFiles {
                        egm: &egm,
                        nif: Some(&nif_text),
                        tri: tri_text.as_deref(),
                        egt: egt.as_deref(),
                    },
                    hair,
                    use_heads,
                    services,
                )?;
            }
            world.resource_mut::<BsFaceGenModelMap>().models[model] = held;
            Some(model)
        }
        None => {
            let mut held = FaceGenModel::default();
            if load_face_gen_model(
                world,
                &mut held,
                FaceGenModelFiles {
                    egm: &egm,
                    nif: Some(&nif_text),
                    tri: tri_text.as_deref(),
                    egt: egt.as_deref(),
                },
                hair,
                use_heads,
                services,
            )? {
                let mut map = world.resource_mut::<BsFaceGenModelMap>();
                map.models.push(held);
                let model = map.models.len() - 1;
                map.add(&egm, model);
                Some(model)
            } else {
                None
            }
        }
    };
    let Some(model) = model else {
        return Ok(None);
    };
    let mut held = world.resource::<BsFaceGenModelMap>().models[model].clone();
    let positions = create_morphed_positions(world, &mut held, coefficients, services)?;
    let skinned = held.geometry_10.is_some_and(|geometry| {
        world
            .get::<crate::gamebryo::mesh_geometry::NiGeometry>(geometry)
            .is_some_and(|links| links.skin_b8.is_some())
    });
    let geometry = held.geometry_10;
    let morph_data = held.morph_data_1c.clone();
    world.resource_mut::<BsFaceGenModelMap>().models[model] = held;
    Ok(positions
        .zip(geometry)
        .map(|((positions, base), geometry)| MorphedModel {
            geometry,
            positions,
            skinned,
            morph_data,
            part: super::face_gen_nodes::FaceGenPartMorph { model, base },
        }))
}

/// IDA: 00554260 FaceGen_BuildHair(biped, skinned, description, 0): the description's hair (+0x60) model as
/// "Meshes\%s", its .tri and .egm (no EGT), the model map's step with `hair` 1, the morph (00559870); named "FaceGenHair",
/// 00550980's length morph (+0x68), the hair's texture ("Textures\%s", clamp 3, filter 2), g_faceGen's alpha property when
/// it has none, and the placement under a node (`face_gen_nodes::add_hair`). Completion: as `add_hair`.
/// # Errors
/// A stop in the model or the clone.
fn build_hair(
    world: &mut World,
    description: &FaceDescription,
    biped: Entity,
    skinned: Entity,
    use_heads: bool,
    services: &mut FaceGenFileServices<'_>,
    prepare: &mut crate::bs::sky_shader::PrepareGeometryContext<'_>,
) -> Result<(), GameError> {
    let Some(hair) = description.hair_60 else {
        return Ok(());
    };
    let model_path = world
        .get::<crate::tes::form_components::TesModel>(hair)
        .map(|model| super::face_gen_head::path_text(&model.path_04))
        .unwrap_or_default();
    let texture = world
        .get::<crate::tes::data_handler::TesTexture>(hair)
        .map(|texture| super::face_gen_head::path_text(&texture.path_04))
        .unwrap_or_default();
    let mut nif = b"Meshes\\".to_vec();
    nif.extend_from_slice(&model_path);
    let Some(model) = morphed_model(
        world,
        &nif,
        false,
        true,
        &description.coefficients_00,
        use_heads,
        services,
    )?
    else {
        return Ok(());
    };
    let clone = super::face_gen_nodes::add_hair(
        world,
        biped,
        skinned,
        model.geometry,
        &model.positions,
        model.morph_data.as_ref(),
        description.hair_length_68,
        &texture,
        prepare,
    )?;
    world.entity_mut(clone).insert(model.part);
    Ok(())
}

/// IDA: 005547F0 FaceGen_BuildEyes(biped, skinned, description, 0): for the left (+0xB8, the race's part 7) then the right
/// (+0xBC, part 8) eye model: "Meshes\%s", its .tri and .egm, the model map's step (no EGT, not hair), the morph; the
/// clone named "FaceGenEyeLeft" / "FaceGenEyeRight", its texture the description's eyes (+0x6C, its TESTexture path at
/// +0x28) or "Textures\Characters\Eyes\EyeDefault.dds" (the property's clamp 3 and filter 2 are D13's), and the placement.
/// Completion: as `face_gen_nodes::add_eye`.
/// # Errors
/// A stop in a model or a clone.
fn build_eyes(
    world: &mut World,
    description: &FaceDescription,
    biped: Entity,
    skinned: Entity,
    use_heads: bool,
    services: &mut FaceGenFileServices<'_>,
    prepare: &mut crate::bs::sky_shader::PrepareGeometryContext<'_>,
) -> Result<(), GameError> {
    let texture = description
        .eyes_6c
        .and_then(|eyes| world.get::<crate::tes::data_handler::TesTexture>(eyes))
        .map(|texture| {
            let mut path = b"Textures\\".to_vec();
            path.extend_from_slice(&super::face_gen_head::path_text(&texture.path_04));
            path
        })
        .unwrap_or_else(|| b"Textures\\Characters\\Eyes\\EyeDefault.dds".to_vec());
    for (index, name) in [(7, "FaceGenEyeLeft"), (8, "FaceGenEyeRight")] {
        let mut nif = b"Meshes\\".to_vec();
        nif.extend_from_slice(&description.model_paths_74[index]);
        if let Some(model) = morphed_model(
            world,
            &nif,
            false,
            false,
            &description.coefficients_00,
            use_heads,
            services,
        )? {
            let clone = super::face_gen_nodes::add_eye(
                world,
                biped,
                skinned,
                name,
                model.geometry,
                &model.positions,
                &texture,
                prepare,
            )?;
            world.entity_mut(clone).insert(model.part);
        }
    }
    Ok(())
}

/// IDA: the INI values the head build reads live: 00B10D3C bFaceGenTexturing:General, 00B120BC bFixFaceNormals:General,
/// 00B120EC and 00B120F4 uiFaceGenMaxEGMDataSize and uiFaceGenMaxEGTDataSize:General, and
/// 00B120B0 (no INI setting and no writer: its image value 0xFFFFFFFF skips 00555A80's `_50.NIF` head). The defaults are the
/// image values; `begin_load_selected_files` would publish the live ones (the main session's hook).
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceGenHeadSettings {
    pub face_texturing_b10d3c: bool,
    pub fix_face_normals_b120bc: bool,
    pub skip_lod_head_b120b0: u32,
    pub max_egm_size_b120ec: u32,
    pub max_egt_size_b120f4: u32,
}

impl Default for FaceGenHeadSettings {
    /// IDA: N/A the image values.
    fn default() -> Self {
        Self {
            face_texturing_b10d3c: true,
            fix_face_normals_b120bc: false,
            skip_lod_head_b120b0: 0xFFFF_FFFF,
            max_egm_size_b120ec: 0x0400_0000,
            max_egt_size_b120f4: 0x0400_0000,
        }
    }
}

/// IDA: N/A `run_queued_head` with the walk's services: the stream services for BSStream_LoadFromPath, the archives for
/// BSFile_OpenCore(path, 0, 0x2800, 1) (no loose meshes: the archives' type 1), FaceGen's last error (00B3F068), and its
/// warnings through DebugMessage_Warning (`services.warning`). 00B39C80's lock has no host form (D9's one thread).
/// # Errors
/// As `run_queued_head`.
pub fn run_queued_head_in_walk(
    world: &mut World,
    id: IoTaskId,
    precision24: bool,
    services: &mut crate::bs::model_loader::ModelBuildServices<'_>,
) -> Result<(), GameError> {
    with_face_gen_files(
        world,
        precision24,
        services,
        &mut |world, files, prepare| run_queued_head(world, id, files, prepare),
    )
}

/// IDA: N/A the walk's services as FaceGen's file services (`run_queued_head_in_walk`'s), lent to `body`, with the calling
/// thread's x87 precision (`precision24` on the main thread); the warnings go out through DebugMessage_Warning afterwards.
/// # Errors
/// As `body`.
pub fn with_face_gen_files(
    world: &mut World,
    precision24: bool,
    services: &mut crate::bs::model_loader::ModelBuildServices<'_>,
    body: &mut dyn FnMut(
        &mut World,
        &mut FaceGenFileServices<'_>,
        &mut crate::bs::sky_shader::PrepareGeometryContext<'_>,
    ) -> Result<(), GameError>,
) -> Result<(), GameError> {
    let mut last_error = world
        .remove_resource::<crate::tes::facegen::FaceGenLastError>()
        .unwrap_or_default();
    let mut warnings = Vec::new();
    let result = {
        let stream = &mut services.stream;
        let archives = &mut services.archives;
        let mut load_nif = |world: &mut World, path: &str| {
            crate::bs::stream::load_bs_stream_from_path(world, path, stream)
        };
        let shared = std::cell::RefCell::new(archives);
        let mut open = |path: &str| {
            let mut opened = None;
            if let Some(archives) = shared.borrow_mut().as_deref_mut() {
                archives.with_archives(&mut |manager| {
                    opened = manager.and_then(|manager| manager.open_packed(path, 0x2800, 1));
                });
            }
            opened
        };
        let mut exists = |path: &str| {
            let mut found = false;
            if let Some(archives) = shared.borrow_mut().as_deref_mut() {
                archives.with_archives(&mut |manager| {
                    found = manager
                        .is_some_and(|manager| manager.open_packed(path, 0, 0xFFFF).is_some());
                });
            }
            found
        };
        let mut files = FaceGenFileServices {
            load_nif: &mut load_nif,
            open: &mut open,
            exists: &mut exists,
            warnings: &mut warnings,
            last_error: &mut last_error,
            precision24,
        };
        body(world, &mut files, &mut services.prepare)
    };
    world.insert_resource(last_error);
    for text in warnings {
        let disabled = *services.disable_warning_messages_b06b18 != 0;
        (services.warning)(text.into_bytes(), disabled);
    }
    result
}
