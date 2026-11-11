//! Phase 08 D4: BSShaderManager_PrepareGeometry 007B7FC0 as the sky prelude reaches it. 007B8940 refreshes the property state, classifies the
//! root (00B42EB4 = 004E0CF0) and assigns the SkyShader slot (index 10) to every geometry of the tree. SkyShader's vtable +0x18 (007BDAC0)
//! attaches a new SkyShaderProperty, or re-runs its +0x58 on an existing one. State only: nothing is drawn (the sky renders once a world cell
//! is up, Phase 15+).
//!
//! WaterShader's 007DD370 (Phase 09) is in `shader_slot_18` and `shader_slot_1c`. 007D1920 (the original index 1, Phase 13e) is `classify_geometry_shader`. The other shader classes' +0x18, such as the ShadowLight family's 0080B190, go to a counted boundary (`unported_shader_setup`).
//!
//! Evidence: evidence/phase08-window-2026-10-06/decompile-b (007B7FC0), decompile-c (007BDAC0, 007C53B0, 007C5190), P08-RUN-001 (the
//! SkyShaderProperty bytes of every sky and weather geometry).

use std::sync::Arc;

use bevy_ecs::prelude::*;
use oblivion_core::crt::{stricmp, strnicmp};
use oblivion_core::error::GameError;

use crate::bs::scene_graphs::REFERENCE_FORM_TYPE_CLASSIFIER_ADDRESS;
use crate::bs::shader::{BsShaderClass, BsShaderSlot};
#[cfg(test)]
use crate::gamebryo::av_object::NiAvProperties;
use crate::gamebryo::av_object::{detach_property_of_type, find_property_of_type, remove_property};
use crate::gamebryo::mesh_geometry::{
    MeshTriShapeData, NI_COLOR_A_BLACK, NiGeometry, construct_tri_shape, construct_tri_shape_data,
};
use crate::gamebryo::object_net::{AlphaProperty, NiObjectNet};
use crate::gamebryo::particle_system::ParticleSystem;
use crate::gamebryo::properties::{
    LightingProperty, MaterialProperty, SkyShaderProperty, WaterShaderProperty,
};
use crate::gamebryo::scene_node::NiNodeChildren;
use crate::gamebryo::scene_state::{
    ScenePropertyDefaults, attach_object_property, refresh_property_state,
};
use crate::gamebryo::stream_objects::NiClass;
use crate::gamebryo::texture::{SourceTexture, TexturingProperty};

/// IDA: SkyShader's vtable (007BA440).
pub const SKY_SHADER_VTABLE: u32 = 0x00A8_F654;
/// IDA: WaterShader's vtable (007DC9A0).
pub const WATER_SHADER_VTABLE: u32 = 0x00A9_170C;
/// IDA: 007B4290 slot index of WaterShader (0043F6C0 passes 17).
pub const WATER_SHADER_SLOT_INDEX: u32 = 17;
/// IDA: 007B4290 slot index of SkyShader (007BD920 SkyShader_CreateSlot).
pub const SKY_SHADER_SLOT_INDEX: u32 = 10;

/// IDA: NiGeometry +0xBC NiPointer<NiD3DShader> (004EC910). Shaders are not stream objects: their slots hold them as `Arc`s, so the C++
/// `shared_ptr` is the same `Arc` here, on any geometry class. Compare with `Arc::ptr_eq`. Never printed.
#[derive(Component, Clone)]
pub struct NiGeometryShader(pub Arc<dyn BsShaderClass>);

/// IDA: N/A 007B8200(path, 1, 1) as a service: whether a texture comes back.
pub type LoadNormalMap<'a> = &'a mut (dyn FnMut(&mut World, &[u8]) -> bool + 'a);

/// IDA: N/A the globals and services the Phase 08 PrepareGeometry calls read.
pub struct PrepareGeometryContext<'a> {
    /// 00B42F48 g_renderPath.
    pub render_path: i32,
    /// 00B43070 g_rendererHDR (007C53B7).
    pub hdr: bool,
    /// 00B42EB4 (00406B66 stores 004E0CF0).
    pub reference_classifier: u32,
    /// 007319E0 for 00707610.
    pub property_defaults: ScenePropertyDefaults,
    /// 007B4290 BSShaderManager_GetShader (the Phase 05 slot dispatcher).
    pub get_shader: &'a mut (dyn FnMut(u32) -> Option<Arc<BsShaderSlot>> + 'a),
    /// 00B42E8C g_shaderWarningCallback(text, null), in call order.
    pub warnings: Vec<&'static str>,
    /// IDA: N/A host trace of the geometries 007BDAC0 ran on, in order.
    pub sky_shader_trace: Vec<Entity>,
    /// IDA: N/A host trace of the geometries 007DD370 ran on, in order.
    pub water_shader_trace: Vec<Entity>,
    /// IDA: 00B477F0+0x54 (00B47844): 0085BBF8 clears it and 007DD427's "lava" material sets it.
    pub lava_b47844: u32,
    /// IDA: 00B42EA5 g_rendererShaders30, which 007D1920 reads.
    pub shaders_30_b42ea5: bool,
    /// IDA: 007B8200(path, 1, 1) as 007D1A1E calls it: whether a texture comes back. `None` stops at the arm that needs it.
    pub load_normal_map: Option<LoadNormalMap<'a>>,
    /// IDA: N/A the counted boundary (13e): vtable +0x18 of a shader class with no port, with the shader's vtable and the geometry. It answers the +0x18 result. `None` stops.
    pub unported_shader_setup: Option<&'a mut (dyn FnMut(u32, Entity) -> bool + 'a)>,
    /// IDA: N/A host trace of 007D1920's results, in call order.
    pub classify_trace: Vec<(Entity, u32)>,
}

/// IDA: N/A host seam for 00B42EB4's target: 00406B66 stores 004E0CF0 SceneNode_ClassifyReferenceFormType there, and
/// 007B8940 calls through it. 004E0CF0 reads TES references (004DC270's TESObjectExtraData, the base's type byte), above
/// `bs`, so the game layer installs its port (`tes::reference_3d::classify_reference`) as this resource where it makes the
/// 00406B66 store. A world with no game layer (a `bs`-only test) has none, and the classifier answers 0 there.
#[derive(Resource, Clone, Copy)]
pub struct ReferenceClassifier(pub fn(&World, Entity) -> Result<i32, GameError>);

// IDA: vtable +0x54 of a type-4 property: BSShaderProperty 00741360 returns 11, WaterShaderProperty 0073D5D0 12, NiShadeProperty 006F7030 -1 (A81584 +0x54: `or eax,-1`; the C++ stops here).
// Only those classes are bound; another is a host stop.
pub(crate) fn shader_property_id(world: &World, property: Entity) -> Result<i32, GameError> {
    match world.get::<NiClass>(property) {
        Some(NiClass::PpLightingProperty) => return Ok(5),
        Some(NiClass::Lighting30Property) => return Ok(10),
        _ => {}
    }
    if world.get::<SkyShaderProperty>(property).is_some() {
        return Ok(SkyShaderProperty::SHADER_PROPERTY_ID);
    }
    if world.get::<WaterShaderProperty>(property).is_some() {
        return Ok(WaterShaderProperty::SHADER_PROPERTY_ID);
    }
    if world
        .get::<crate::gamebryo::registration::NiShadeProperty>(property)
        .is_some()
    {
        return Ok(-1);
    }
    Err(GameError::unclassified(
        0,
        "type-4 property class without a bound +0x54",
    ))
}

// IDA: 007E2430 BSShaderProperty_SetAlpha. Return +1C bit 6 for the caller's
// NiAlphaProperty update. Completion: threshold crossings and stores for bound shaders.
pub(crate) fn set_shader_alpha(
    world: &mut World,
    property: Entity,
    alpha: f32,
) -> Result<bool, GameError> {
    let assign = |old: &mut f32, pass: &mut u32| {
        if (*old >= 1.0 && alpha < 1.0) || (*old < 1.0 && alpha >= 1.0) {
            *pass = 0;
        }
        *old = alpha;
    };
    if let Some(p) = world.get_mut::<LightingProperty>(property) {
        let p = p.into_inner();
        assign(&mut p.alpha_20, &mut p.last_pass_state_24);
        return Ok(p.alpha_blend_pass_1c);
    }
    if let Some(p) = world.get_mut::<SkyShaderProperty>(property) {
        let p = p.into_inner();
        assign(&mut p.alpha_20, &mut p.last_pass_state_24);
        return Ok(p.pass_info_1c & 0x40 != 0);
    }
    if let Some(p) = world.get_mut::<WaterShaderProperty>(property) {
        let p = p.into_inner();
        assign(&mut p.alpha_20, &mut p.last_pass_state_24);
        return Ok(p.pass_info_1c & 0x40 != 0);
    }
    Err(GameError::unclassified(
        0x007E2430,
        "unbound BSShaderProperty alpha",
    ))
}

// IDA: 007B804E / 007B80DE the BSShaderProperty +0x68 classifier store. Completion: 100% for the bound classes.
pub(crate) fn set_shader_property_classifier(
    world: &mut World,
    property: Option<Entity>,
    value: i32,
) -> Result<(), GameError> {
    let Some(property) = property else {
        return Ok(());
    };
    if let Some(mut lighting) = world.get_mut::<LightingProperty>(property) {
        lighting.classifier_68 = value;
        return Ok(());
    }
    if let Some(mut sky) = world.get_mut::<SkyShaderProperty>(property) {
        sky.classifier_68 = value;
        return Ok(());
    }
    if let Some(mut water) = world.get_mut::<WaterShaderProperty>(property) {
        water.classifier_68 = value;
        return Ok(());
    }
    Err(GameError::unclassified(
        0x007B80DE,
        "007B80DE: +0x68 of an unbound type-4 property",
    ))
}

// IDA: vtable +0x0C (007616D0 returns this) answers for NiTriShape, NiTriStrips, NiParticleSystem and NiScreenElements (A7E29C +0x0C; the C++ omits this one).
pub(crate) fn is_geometry(world: &World, object: Entity) -> bool {
    matches!(
        world.get::<NiClass>(object),
        Some(
            NiClass::TriShape
                | NiClass::TriStrips
                | NiClass::ParticleSystem
                | NiClass::ScreenElements
        )
    )
}

// IDA: N/A the +0xAC NiPropertyState of a geometry class.
fn property_state_of(world: &World, geometry: Entity) -> Option<Entity> {
    world
        .get::<NiGeometry>(geometry)
        .map(|links| links.property_state_ac)
        .or_else(|| {
            world
                .get::<ParticleSystem>(geometry)
                .map(|system| system.property_state_ac)
        })
        .flatten()
}

// IDA: shader vtable +0x1C (SkyShader 007BA960 returns 22, WaterShader 007DB4A0 returns 0x18); the other classes' slots are not ported.
fn shader_slot_1c(shader: &dyn BsShaderClass) -> Result<i32, GameError> {
    match shader.base().original_vtable {
        0x00A90634 => return Ok(1),  // ShadowLightShader
        0x00A941AC => return Ok(4),  // SkinShader
        0x00A930C4 => return Ok(27), // Lighting30Shader
        _ => {}
    }
    if shader.base().original_vtable == SKY_SHADER_VTABLE {
        return Ok(22);
    }
    if shader.base().original_vtable == WATER_SHADER_VTABLE {
        return Ok(0x18);
    }
    Err(GameError::unclassified(
        0,
        "shader vtable +0x1C of a class other than Sky/WaterShader",
    ))
}

// IDA: shader vtable +0x18 (SkyShader 007BDAC0, WaterShader 007DD370). Any other class goes to the counted boundary when the caller has one, else stops: the ShadowLight family's 0080B190 builds a BSShaderPPLightingProperty (0xF0, 007D94B0), whose +0x58 isn't ported.
pub(crate) fn shader_slot_18(
    world: &mut World,
    shader: &dyn BsShaderClass,
    geometry: Entity,
    context: &mut PrepareGeometryContext<'_>,
) -> Result<bool, GameError> {
    if shader.base().original_vtable == SKY_SHADER_VTABLE {
        return sky_shader_prepare(world, geometry, context);
    }
    if shader.base().original_vtable == WATER_SHADER_VTABLE {
        return water_shader_prepare(world, geometry, context);
    }
    let vtable = shader.base().original_vtable;
    let class = match vtable {
        0x00A90634 | 0x00A941AC => Some(NiClass::PpLightingProperty),
        0x00A930C4 => Some(NiClass::Lighting30Property),
        _ => None,
    };
    if let Some(class) = class {
        return setup_lighting_property(world, geometry, class, vtable, context);
    }
    if let Some(boundary) = context.unported_shader_setup.as_mut() {
        return Ok(boundary(vtable, geometry));
    }
    Err(GameError::unclassified(
        0,
        "shader vtable +0x18 of a class other than Sky/WaterShader",
    ))
}

// IDA: 0080B190 ShadowLightShader_SetupGeometry / 007FF350 Lighting30Shader_AttachShaderProperty.
// Completion: material selection, property creation, the 007B7710 tangent-space failure arm and 007DA220's texturing removal.
// TODO: [ ] 007DA220's texture copies, pass flags, dynalpha's alpha clone and the hair pass, and 00864400, stay behind the
// counted renderer boundary.
fn setup_lighting_property(
    world: &mut World,
    geometry: Entity,
    class: NiClass,
    shader_vtable: u32,
    context: &mut PrepareGeometryContext<'_>,
) -> Result<bool, GameError> {
    let boundary = context
        .unported_shader_setup
        .as_mut()
        .ok_or(GameError::unclassified(
            0x007DA220,
            "lighting shader texture setup has no renderer boundary",
        ))?;
    let renderer_result = boundary(shader_vtable, geometry);
    if let Some(old) = find_property_of_type(world, geometry, 4)? {
        let id = shader_property_id(world, old)?;
        if id == 10 || (class == NiClass::PpLightingProperty && (5..=10).contains(&id)) {
            return Ok(true);
        }
        detach_property_of_type(world, geometry, 4)?;
    }
    let alpha_blend_pass_1c = find_property_of_type(world, geometry, 0)?
        .and_then(|p| world.get::<AlphaProperty>(p))
        .is_some_and(|p| p.flags_18 & 1 != 0);
    let property = world
        .spawn((
            class,
            NiObjectNet::default(),
            LightingProperty {
                alpha_20: 1.0,
                alpha_blend_pass_1c,
                ..Default::default()
            },
        ))
        .id();
    attach_object_property(world, geometry, Some(property))?;
    let tangent_extra = crate::gamebryo::object_net::find_extra_data(
        world,
        geometry,
        b"Tangent space (binormal & tangent vectors)",
    )?
    .is_some();
    let billboard = world
        .get::<NiObjectNet>(geometry)
        .and_then(|net| net.name.as_deref())
        .is_some_and(|name| strnicmp(name, b"STBB", 4) == 0);
    let has_normals = world
        .get::<NiGeometry>(geometry)
        .and_then(|g| g.data_b4)
        .and_then(|d| crate::gamebryo::mesh_geometry::geometry_data_base(world, d))
        .is_some_and(|data| !data.normals_20.is_empty());
    if !tangent_extra && !billboard && !has_normals {
        detach_property_of_type(world, geometry, 6)?;
        remove_property(world, geometry, property);
        world.despawn(property);
        if let Some(mut geom) = world.get_mut::<NiGeometry>(geometry) {
            geom.property_state_ac = None;
        }
        return Ok(false);
    }
    // 007DA4F5..007DA506, the success tail of 007DA220 BSShaderPPLightingProperty_SetupGeometry (the property's vt+0x58):
    // the geometry's type-6 NiTexturingProperty leaves it once the textures are the property's.
    if renderer_result && let Some(texturing) = find_property_of_type(world, geometry, 6)? {
        // 007DA4B0..007DA4D6 (LABEL_69): the base map's texture (maps +0x00, its +0x08) into +0xBC[0], counted up.
        let base = world
            .get::<crate::gamebryo::texture::TexturingProperty>(texturing)
            .and_then(|texturing| texturing.maps_1c.first().copied().flatten())
            .and_then(|map| map.texture_08);
        if let Some(mut lighting) = world.get_mut::<LightingProperty>(property) {
            lighting.diffuse_map_bc = base;
        }
        remove_property(world, geometry, texturing);
    }
    Ok(renderer_result)
}

/// IDA: 007B4160(out, path, suffix): *out = 0; strcpy into 0x104 stack bytes; the last '.', '\\' and '_' (00983FD0), with a '_' before the last '\\' ignored; no '.' leaves out empty. The extension goes to 12 stack bytes; the '_' store, then the '.' store, cut the name. A name whose bytes 0 or 1 are '\\' or ':', or that `_strnicmp`s "data" for 4, takes "%s%s%s" (A2F88C), others "Data\\%s%s%s" (A8D32C). Bytes 0 and 1 are read in the buffer as left, so byte 1 can be past the cut. Completion: 100%.
/// # Errors
/// A path or an extension past its stack buffer.
pub fn shader_texture_variant_path(path: &[u8], suffix: &[u8]) -> Result<Vec<u8>, GameError> {
    if path.len() >= 0x104 {
        return Err(GameError::unclassified(
            0x007B4193,
            "007B4193: a texture path past the 0x104 stack bytes",
        ));
    }
    let last = |byte: u8| path.iter().rposition(|&other| other == byte);
    let (dot, slash) = (last(b'.'), last(b'\\'));
    let underscore = last(b'_').filter(|&at| slash.is_none_or(|slash| at >= slash));
    let Some(dot) = dot else {
        return Ok(Vec::new());
    };
    let extension = &path[dot..];
    if extension.len() >= 12 {
        return Err(GameError::unclassified(
            0x007B41E6,
            "007B41E6: a texture extension past the 12 stack bytes",
        ));
    }
    let name = &path[..underscore.map_or(dot, |at| at.min(dot))];
    // 007B41FB, 007B4202 the buffer's bytes after the two NUL stores; a NUL store at byte 0 or 1 reads as 0.
    let byte = |index: usize| {
        if index == dot || underscore == Some(index) {
            0
        } else {
            path.get(index).copied().unwrap_or(0)
        }
    };
    let rooted = byte(0) == b'\\' || byte(1) == b':' || strnicmp(name, b"data", 4) == 0;
    let mut out = if rooted {
        Vec::new()
    } else {
        b"Data\\".to_vec()
    };
    out.extend_from_slice(name);
    out.extend_from_slice(suffix);
    out.extend_from_slice(extension);
    Ok(out)
}

// IDA: N/A a property's +0x08 name, as `_stricmp` would read it.
fn property_name(world: &World, property: Entity) -> Option<Vec<u8>> {
    world
        .get::<NiObjectNet>(property)
        .and_then(|net| net.name.clone())
}

/// IDA: 007D1920 BSShaderManager_ClassifyGeometryShader(geometry, a2): the shader index for 007B7FC0's index 1. 00405760 +0xAC, with 00707610 and a second read on a null. A null geometry and vt+0x14 (only NiParticleSystem among the bound classes: A85B0C +0x14 is 007616D0, and A7E29C and the shape vtables' are 006F7070) return 0; so does state +0x20 null. A type-2 material named "lava" returns 0x11. The texturing property's map 0 and its +0x08 texture: null returns 0, no NiSourceTexture (560920 B3F95C) or no +0x38 name returns 1. 007B4160 "_n" and 007B8200(1, 1): no texture and a2 = 0 returns 0. A material with a name that `_strnicmp`s "hair" for 4: 0x1A with g_rendererShaders30 or g_renderPath >= 5, else 1. At render path >= 2, a material named "skin" (`_stricmp`, a null name faults): `fld1; fcomp +0x4C; test ah,0x41` bumps +0x54 and stores 4.0 at +0x4C when 1.0 > shine (not on NaN), and returns 0xE. Texturing +0x18 byte & 0xE == 8 returns 0xF. The alpha property and the material's `_strnicmp` "dynalpha" for 7 (-1 with no material; a null name faults). An alpha with bit 0, or "dynalpha": 0x1A with shaders 3.0, else 1. Otherwise with shaders 3.0, a NiGeomMorpherController (B3CE30) on the +0x0C chain gives 0x1A; else 1. Completion: 100% of the arms; "Block", "STBB" at 007D1C0C, 007D1C20 are `strncmp`s whose results aren't read, and 007D1B7C's alpha with no owner is a leak with no effect here, so neither is kept. No bound host class is NiGeomMorpherController, and the stream refuses one, so the walk can't match.
/// # Errors
/// A stop for a fault in the original, or no `load_normal_map`.
pub fn classify_geometry_shader(
    world: &mut World,
    geometry: Entity,
    allow_missing_normal: bool,
    context: &mut PrepareGeometryContext<'_>,
) -> Result<u32, GameError> {
    // 007D1971 00405760; 007D19A6 00707610 on a null.
    let mut state = property_state_of(world, geometry);
    if state.is_none() {
        refresh_property_state(world, geometry, &context.property_defaults)?;
        state = property_state_of(world, geometry);
    }
    let index =
        classify_geometry_shader_body(world, geometry, state, allow_missing_normal, context)?;
    context.classify_trace.push((geometry, index));
    Ok(index)
}

// IDA: N/A no address of its own; its comments cite 007D19E9.
fn classify_geometry_shader_body(
    world: &mut World,
    geometry: Entity,
    state: Option<Entity>,
    allow_missing_normal: bool,
    context: &mut PrepareGeometryContext<'_>,
) -> Result<u32, GameError> {
    // 007D19E9 vt+0x14.
    if world.get::<NiClass>(geometry) == Some(&NiClass::ParticleSystem) {
        return Ok(0);
    }
    let state = state.ok_or(GameError::unclassified(
        0x007D19F4,
        "007D19F4: a null +0xAC after 00707610",
    ))?;
    // 007D19F4 state +0x20, the texturing slot.
    let Some(texturing) = world
        .get::<crate::gamebryo::scene_state::ScenePropertyState>(state)
        .ok_or(GameError::unclassified(
            0x007D19F4,
            "007D19F4: +0xAC isn't a NiPropertyState",
        ))?
        .slots[6]
    else {
        return Ok(0);
    };
    // 007D19FF 00707530(geometry, 2); 007D1A17 `_stricmp`, skipped for a null name.
    let material = find_property_of_type(world, geometry, 2)?;
    let material_name = material.and_then(|material| property_name(world, material));
    if material_name
        .as_deref()
        .is_some_and(|name| stricmp(name, b"lava") == 0)
    {
        return Ok(0x11);
    }
    // 007D1A2D map 0, 007D1A36 its +0x08.
    let (map_texture, apply_mode) = {
        let property = world
            .get::<TexturingProperty>(texturing)
            .ok_or(GameError::unclassified(
                0x007D1A2D,
                "007D1A2D: state +0x20 isn't a NiTexturingProperty",
            ))?;
        let texture = property
            .maps_1c
            .first()
            .and_then(Option::as_ref)
            .and_then(|map| map.texture_08);
        (texture, property.flags_18 as u8)
    };
    let Some(texture) = map_texture else {
        return Ok(0);
    };
    // 007D1A43 560920(B3F95C); 007D1A53 +0x38.
    let Some(file_name) = world
        .get::<SourceTexture>(texture)
        .and_then(|source| source.platform_filename_38.clone())
    else {
        return Ok(1);
    };
    // 007D1A69 007B4160, 007D1A7C 007B8200(buffer, 1, 1), 007D1A84 the texture, 007D1A89 a2.
    let normal = shader_texture_variant_path(&file_name, b"_n")?;
    let load = context
        .load_normal_map
        .as_mut()
        .ok_or(GameError::unclassified(
            0x007D1A7C,
            "007D1A7C: 007B8200 without texture services",
        ))?;
    if !load(world, &normal) && !allow_missing_normal {
        return Ok(0);
    }
    // 007D1AB5 "hair".
    if material_name
        .as_deref()
        .is_some_and(|name| strnicmp(name, b"hair", 4) == 0)
    {
        return Ok(if context.shaders_30_b42ea5 || context.render_path >= 5 {
            0x1A
        } else {
            1
        });
    }
    // 007D1AE8, 007D1AF1: the `_stricmp` reads the name with no null test.
    if context.render_path >= 2
        && let Some(material) = material
    {
        let name = material_name.as_deref().ok_or(GameError::unclassified(
            0x007D1AFE,
            "007D1AFE: `_stricmp` of a null material name",
        ))?;
        if stricmp(name, b"skin") == 0 {
            let mut values =
                world
                    .get_mut::<MaterialProperty>(material)
                    .ok_or(GameError::unclassified(
                        0x007D1B0C,
                        "007D1B0C: the type-2 property's values",
                    ))?;
            // 007D1B0C `fcomp`, 007D1B11 `test ah,0x41`: on `1.0 > shine` only; NaN sets C0 and takes the skip.
            if 1.0_f32 > values.values.shine {
                values.raw_state_54 = values.raw_state_54.wrapping_add(1);
                values.values.shine = 4.0;
            }
            return Ok(0xE);
        }
    }
    // 007D1B30.
    if apply_mode & 0xE == 8 {
        return Ok(0xF);
    }
    // 007D1B46 alpha, 007D1B51 material again.
    let alpha = find_property_of_type(world, geometry, 0)?;
    let dynalpha = match material {
        Some(_) => strnicmp(
            material_name.as_deref().ok_or(GameError::unclassified(
                0x007D1B65,
                "007D1B65: `_strnicmp` of a null material name",
            ))?,
            b"dynalpha",
            7,
        ),
        None => -1,
    };
    // 007D1B74..007D1BBB. 007D1B97's new property has flags 0xEC, with 007D1BA0 clearing bit 0, so "dynalpha" decides it.
    let alpha_bit = alpha.is_some_and(|alpha| {
        world
            .get::<AlphaProperty>(alpha)
            .is_some_and(|alpha| alpha.flags_18 & 1 != 0)
    });
    if (alpha_bit || dynalpha == 0) && context.shaders_30_b42ea5 {
        return Ok(0x1A);
    }
    // 007D1BD6..007D1C25: nothing here can change the answer.
    Ok(1)
}

// IDA: 007C53B0 SkyShaderProperty vtable +0x58(geometry): without g_rendererHDR returns 1 (007C53B7); otherwise 00405760 reads +0xAC and a null
// state gets 00707610 (007C53FD; the second 00405760 read is released at once). Returns 1. Completion: 100%.
pub fn sky_shader_property_setup(
    world: &mut World,
    geometry: Entity,
    context: &mut PrepareGeometryContext<'_>,
) -> Result<bool, GameError> {
    if !context.hdr {
        return Ok(true);
    }
    if is_geometry(world, geometry) && property_state_of(world, geometry).is_none() {
        refresh_property_state(world, geometry, &context.property_defaults)?;
    }
    Ok(true)
}

// IDA: 007BDAC0 SkyShader vtable +0x18(geometry): an existing type-4 property with shader property id 11 runs its +0x58 and returns 1; any other
// type-4 property is detached (00708560); then a new 0x8C SkyShaderProperty (007C5190) is attached (00405680) and its +0x58 run; a false result
// removes it again (004A1220) and returns 0. Completion: 100% of the order; another class with id 11 stops, as in the C++.
pub fn sky_shader_prepare(
    world: &mut World,
    geometry: Entity,
    context: &mut PrepareGeometryContext<'_>,
) -> Result<bool, GameError> {
    context.sky_shader_trace.push(geometry);
    // 007BDAE3 00707530(geometry, 4).
    if let Some(existing) = find_property_of_type(world, geometry, 4)? {
        // 007BDB0B vtable +0x54 == 11 -> +0x58(geometry); return 1.
        if shader_property_id(world, existing)? == 11 {
            if world.get::<SkyShaderProperty>(existing).is_none() {
                return Err(GameError::unclassified(
                    0x007BDBB1,
                    "007BDBB1: +0x58 of an unbound BSShaderProperty",
                ));
            }
            sky_shader_property_setup(world, geometry, context)?;
            return Ok(true);
        }
        // 007BDB1A 00708560(geometry, out, 4); the returned owner is released.
        detach_property_of_type(world, geometry, 4)?;
    }
    // 007BDB48 FormHeap(0x8C) 007C5190; 007BDB78 00405680.
    let property = world
        .spawn((NiObjectNet::default(), SkyShaderProperty::default()))
        .id();
    attach_object_property(world, geometry, Some(property))?;
    // 007BDB85 +0x58(geometry); false -> 004A1220 removes it and returns 0.
    if !sky_shader_property_setup(world, geometry, context)? {
        remove_property(world, geometry, property);
        world.despawn(property);
        return Ok(false);
    }
    Ok(true)
}

// IDA: 007DD370 WaterShader vtable +0x18(geometry): an existing type-4 property with shader property id 0xC returns 1; any other type-4 property is detached (00708560); then a new 0x88
// WaterShaderProperty (0085BBE0, 007E27F0 base, whose 0085BBF8 store clears 00B477F0+0x54); a type-2 material whose +0x08 name is "lava" (_stricmp) sets 00B477F0+0x54; 00405680 attaches the
// property and 006FDFF0's +0x58 returns 1. Completion: 100% of the order.
pub fn water_shader_prepare(
    world: &mut World,
    geometry: Entity,
    context: &mut PrepareGeometryContext<'_>,
) -> Result<bool, GameError> {
    context.water_shader_trace.push(geometry);
    // 007DD39B 00707530(geometry, 4).
    if let Some(existing) = find_property_of_type(world, geometry, 4)? {
        // 007DD3AB vtable +0x54 == 0xC -> return 1.
        if shader_property_id(world, existing)? == 0xC {
            return Ok(true);
        }
        // 007DD3C8 00708560(geometry, out, 4); the owner is released.
        detach_property_of_type(world, geometry, 4)?;
    }
    // 007DD3F6 FormHeap(0x88) 0085BBE0.
    let property = world
        .spawn((NiObjectNet::default(), WaterShaderProperty::default()))
        .id();
    context.lava_b47844 = 0;
    // 007DD427 00707530(geometry, 2): a material whose +0x08 name is "lava" sets 00B477F0+0x54.
    if let Some(material) = find_property_of_type(world, geometry, 2)?
        && world
            .get::<NiObjectNet>(material)
            .and_then(|net| net.name.as_deref())
            .is_some_and(|name| stricmp(name, b"lava") == 0)
    {
        context.lava_b47844 = 1;
    }
    // 007DD453 00405680; 007DD460 +0x58 (006FDFF0 returns 1).
    attach_object_property(world, geometry, Some(property))?;
    Ok(true)
}

// IDA: 007B7FC0 BSShaderManager_PrepareGeometry(object, index, a3, a4, classifierValue): 0 when g_renderPath is 0 or the object null; a geometry
// (vtable +0x0C) with a type-4 property (+0x54 != -1) and a +0xBC shader re-runs that shader's +0x18 and stores +0x68; otherwise index > 0x1C warns
// ("Object prepared with invalid shader index") and uses 1, an original index 1 resolves through 007D1920, a zero index returns 0, 007B4290 gives
// the slot (null: 0), 004EC910 stores the slot shader at +0xBC, its +0x18 runs and the type-4 property +0x68 = classifierValue. A NiNode ORs the
// results of its nonnull children. a3 goes to 007D1920 and a4 is never read (007D1920 takes it and never reads it). Completion: 100% for SkyShader and WaterShader; another class's +0x18 is a counted boundary.
pub fn prepare_geometry(
    world: &mut World,
    object: Option<Entity>,
    shader_index: u32,
    arg3: i32,
    arg4: i32,
    classifier_value: i32,
    context: &mut PrepareGeometryContext<'_>,
) -> Result<bool, GameError> {
    // 007B7FC1..007B7FCF: g_renderPath 0 returns 0; so does a null object.
    let Some(object) = object else {
        return Ok(false);
    };
    if context.render_path == 0 {
        return Ok(false);
    }
    let mut result = false;
    if is_geometry(world, object) {
        // 007B7FF4 00707530(object, 4); 007B7FF9 +0xBC shader.
        let property = find_property_of_type(world, object, 4)?;
        let shader = world
            .get::<NiGeometryShader>(object)
            .map(|shader| Arc::clone(&shader.0));
        let property_usable = match property {
            Some(property) => shader_property_id(world, property)? != -1,
            None => false,
        };
        let shader_usable = match &shader {
            Some(shader) => shader_slot_1c(shader.as_ref())? != -1,
            None => false,
        };
        // 007B803E: both present -> the shader's +0x18 again, +0x68 = value.
        if property_usable && let Some(shader) = &shader {
            if !shader_usable {
                return Err(GameError::unclassified(
                    0x007B8048,
                    "007B8048: +0x18 through a null shader view",
                ));
            }
            let again = shader_slot_18(world, shader.as_ref(), object, context)?;
            set_shader_property_classifier(world, property, classifier_value)?;
            return Ok(again);
        }
        // 007B805E: index > 0x1C warns and uses 1.
        let mut index = shader_index;
        if shader_index > 0x1C {
            context
                .warnings
                .push("Object prepared with invalid shader index");
            index = 1;
        }
        // 007B8084: the original index 1 resolves through 007D1920(object, a3, a4).
        if shader_index == 1 {
            index = classify_geometry_shader(world, object, arg3 != 0, context)?;
        }
        let _ = arg4;
        if index == 0 {
            return Ok(false);
        }
        // 007B809E 007B4290(index); a null slot returns 0.
        let Some(slot) = (context.get_shader)(index) else {
            return Ok(false);
        };
        let Some(slot_shader) = slot.shader.as_ref().map(Arc::clone) else {
            return Err(GameError::unclassified(
                0x007B80B9,
                "007B80B9: null shader in the slot",
            ));
        };
        // 007B80B9 004EC910: +0xBC NiPointer assign (equal pointers skip).
        let same = world
            .get::<NiGeometryShader>(object)
            .is_some_and(|current| Arc::ptr_eq(&current.0, &slot_shader));
        if !same {
            world
                .entity_mut(object)
                .insert(NiGeometryShader(Arc::clone(&slot_shader)));
        }
        // 007B80CD the slot shader's +0x18(object).
        result = shader_slot_18(world, slot_shader.as_ref(), object, context)?;
        // 007B80CF..007B80DE: the type-4 property +0x68 = value.
        let property = find_property_of_type(world, object, 4)?;
        set_shader_property_classifier(world, property, classifier_value)?;
        return Ok(result);
    }
    // 007B80EB vtable +0x08 (NiNode): OR over each nonnull child slot.
    if matches!(
        world.get::<NiClass>(object),
        Some(NiClass::Node | NiClass::BillboardNode)
    ) {
        let children = world
            .get::<NiNodeChildren>(object)
            .expect("a NiNode has child slots")
            .slots_ac
            .clone();
        for child in children.into_iter().flatten() {
            result = prepare_geometry(
                world,
                Some(child),
                shader_index,
                arg3,
                arg4,
                classifier_value,
                context,
            )? || result;
        }
    }
    Ok(result)
}

// IDA: 007B8940(object, index, a3, a4): 0 for null; 00707610, the 00B42EB4 classifier when set (else 0), 007B7FC0 with its value, 00707610 again.
// Completion: 100% of the order.
pub fn prepare_object_geometry(
    world: &mut World,
    object: Option<Entity>,
    shader_index: u32,
    arg3: i32,
    arg4: i32,
    context: &mut PrepareGeometryContext<'_>,
) -> Result<bool, GameError> {
    // 007B8944: a null object returns 0.
    let Some(entity) = object else {
        return Ok(false);
    };
    // 007B8955 00707610.
    refresh_property_state(world, entity, &context.property_defaults)?;
    // 007B895A..007B8968: the 00B42EB4 classifier when set, else 0.
    let classifier_value = match context.reference_classifier {
        0 => 0,
        // The installed 004E0CF0 (`ReferenceClassifier`); 0 only with no game layer present (host seam).
        REFERENCE_FORM_TYPE_CLASSIFIER_ADDRESS => world
            .get_resource::<ReferenceClassifier>()
            .map_or(Ok(0), |classifier| (classifier.0)(world, entity))?,
        _ => {
            return Err(GameError::unclassified(
                0x007B8962,
                "007B8962: unknown 00B42EB4 classifier",
            ));
        }
    };
    // 007B8974 007B7FC0, then 007B897C 00707610 again.
    let result = prepare_geometry(
        world,
        object,
        shader_index,
        arg3,
        arg4,
        classifier_value,
        context,
    )?;
    refresh_property_state(world, entity, &context.property_defaults)?;
    Ok(result)
}

// IDA: 007BD0D0 the atmosphere quad (without g_rendererHDR): positions (-1, -1, 0.999999), (1, -1, z), (1, 1, z), (-1, 1, z) with z = A8F8E4 (0x3F7FFFEF),
// triangles (0,1,2) (3,0,2), four NiColorA 00B25AD0 (0, 0, 0, 1), 007174B0(4, positions, null, colors, null, 0, 0, 2, triangles), then the data bound
// (+0x0C..+0x18) = (0, 0, 0, 1000.0). Completion: 100% of the values.
pub(crate) fn build_atmosphere_quad(world: &mut World) -> Entity {
    let z = f32::from_bits(0x3F7F_FFEF);
    let data = construct_tri_shape_data(
        world,
        4,
        vec![
            [-1.0, -1.0, z],
            [1.0, -1.0, z],
            [1.0, 1.0, z],
            [-1.0, 1.0, z],
        ],
        Vec::new(),
        vec![NI_COLOR_A_BLACK; 4],
        Vec::new(),
        0,
        0,
        2,
        vec![0, 1, 2, 3, 0, 2],
    );
    let quad = construct_tri_shape(world, Some(data));
    // 007BD2D4..007BD30D: data +0x0C / +0x10 / +0x14 = 0, +0x18 = 1000.0.
    world
        .get_mut::<MeshTriShapeData>(data)
        .expect("just constructed")
        .base
        .bounds_0c = [0.0, 0.0, 0.0, 1000.0];
    quad
}

#[cfg(test)]
mod tests {
    // 00A81584 +0x54 is 006F7030: `or eax,-1`, so 007BDAC0 detaches a NiShadeProperty rather than stopping.
    #[test]
    fn a_shade_property_answers_minus_one() {
        let mut world = World::new();
        let shade = world
            .spawn(crate::gamebryo::registration::NiShadeProperty { flags_18: 1 })
            .id();
        assert_eq!(shader_property_id(&world, shade).unwrap(), -1);
        let screen = world.spawn(NiClass::ScreenElements).id();
        assert!(is_geometry(&world, screen));
    }

    use super::*;
    use crate::bs::shader_classes_geometry::SkyShader;
    use crate::gamebryo::mesh_geometry::{MeshTriShapePayload, spawn_mesh_geometry};
    use crate::gamebryo::scene_node::{SceneNodePayload, spawn_scene_node};

    fn sky_slot() -> Arc<BsShaderSlot> {
        let mut shader = SkyShader::default();
        shader.base.original_vtable = SKY_SHADER_VTABLE;
        Arc::new(BsShaderSlot {
            declaration: None,
            shader: Some(Arc::new(shader)),
        })
    }

    fn context<'a>(
        get_shader: &'a mut (dyn FnMut(u32) -> Option<Arc<BsShaderSlot>> + 'a),
    ) -> PrepareGeometryContext<'a> {
        PrepareGeometryContext {
            render_path: 5,
            hdr: true,
            reference_classifier: REFERENCE_FORM_TYPE_CLASSIFIER_ADDRESS,
            property_defaults: ScenePropertyDefaults::default(),
            get_shader,
            warnings: Vec::new(),
            sky_shader_trace: Vec::new(),
            water_shader_trace: Vec::new(),
            lava_b47844: 0,
            shaders_30_b42ea5: false,
            load_normal_map: None,
            unported_shader_setup: None,
            classify_trace: Vec::new(),
        }
    }

    fn shape(world: &mut World) -> Entity {
        spawn_mesh_geometry(world, NiClass::TriShape, MeshTriShapePayload::default())
    }

    // 007B4160's arms. Bytes 0 and 1 read the buffer after the cut, so ":" at byte 1 survives a '_' at byte 0.
    #[test]
    fn the_normal_map_path_keeps_its_arms() {
        let path = |text: &[u8]| shader_texture_variant_path(text, b"_n").unwrap();
        assert_eq!(
            path(b"textures\\wood\\oak_d.dds"),
            b"Data\\textures\\wood\\oak_n.dds"
        );
        assert_eq!(
            path(b"textures\\a_b\\oak.dds"),
            b"Data\\textures\\a_b\\oak_n.dds"
        );
        assert_eq!(path(b"Data\\x.dds"), b"Data\\x_n.dds");
        assert_eq!(path(b"\\x.dds"), b"\\x_n.dds");
        assert_eq!(path(b"c:\\x.dds"), b"c:\\x_n.dds");
        assert_eq!(path(b"_:x.dds"), b"_n.dds");
        assert_eq!(path(b"no_extension"), b"");
        assert_eq!(path(b"a.b_c"), b"Data\\a_n.b_c");
        assert!(shader_texture_variant_path(&[b'x'; 0x104], b"_n").is_err());
        assert!(shader_texture_variant_path(b"x.0123456789a", b"_n").is_err());
    }

    fn textured_shape(
        world: &mut World,
        material_name: Option<&[u8]>,
        map_name: Option<&[u8]>,
    ) -> Entity {
        use crate::gamebryo::texture::{TexturingMap, TexturingMapKind};
        let object = shape(world);
        let texture = world
            .spawn(SourceTexture {
                platform_filename_38: map_name.map(<[u8]>::to_vec),
                ..SourceTexture::default()
            })
            .id();
        let mut texturing = TexturingProperty::default();
        texturing.maps_1c[0] = Some(TexturingMap {
            kind: TexturingMapKind::Map,
            flags_04: 0,
            texture_08: Some(texture),
            transform_0c: None,
            luma_scale_10: 0.0,
            luma_offset_14: 0.0,
            bump_matrix_18: [0.0; 4],
            shader_id_10: 0,
        });
        let texturing = world.spawn((NiClass::TexturingProperty, texturing)).id();
        let mut list = vec![Some(texturing)];
        if let Some(name) = material_name {
            let material = world
                .spawn((
                    NiClass::MaterialProperty,
                    MaterialProperty {
                        material_id_18: 0,
                        values: Default::default(),
                        raw_state_54: 1,
                        raw_state_58: 0,
                    },
                    NiObjectNet {
                        name: Some(name.to_vec()),
                        ..NiObjectNet::default()
                    },
                ))
                .id();
            list.push(Some(material));
        }
        world.entity_mut(object).insert(NiAvProperties(list));
        object
    }

    // 007D1920's arms, in the exe's order.
    #[test]
    fn the_shader_classification_takes_each_arm() {
        let mut world = World::new();
        let mut get = |_: u32| None;
        let mut found = |_: &mut World, _: &[u8]| true;
        let mut context = context(&mut get);
        context.load_normal_map = Some(&mut found);
        let name = Some(&b"t_d.dds"[..]);
        let classify = |world: &mut World, object: Entity, context: &mut _| {
            classify_geometry_shader(world, object, false, context).unwrap()
        };
        // A name of "LAVA" comes before the map; a map with no file name is 1.
        let object = textured_shape(&mut world, Some(b"LAVA"), None);
        assert_eq!(classify(&mut world, object, &mut context), 0x11);
        let object = textured_shape(&mut world, None, None);
        assert_eq!(classify(&mut world, object, &mut context), 1);
        // "hair": render path 5, then shaders 3.0, and otherwise 1.
        let object = textured_shape(&mut world, Some(b"HAIRy"), name);
        assert_eq!(classify(&mut world, object, &mut context), 0x1A);
        context.render_path = 4;
        assert_eq!(classify(&mut world, object, &mut context), 1);
        context.shaders_30_b42ea5 = true;
        assert_eq!(classify(&mut world, object, &mut context), 0x1A);
        context.shaders_30_b42ea5 = false;
        // "skin": shine below 1.0 bumps +0x54 and takes 4.0; NaN doesn't.
        let object = textured_shape(&mut world, Some(b"Skin"), name);
        let material = find_property_of_type(&world, object, 2).unwrap().unwrap();
        world
            .get_mut::<MaterialProperty>(material)
            .unwrap()
            .values
            .shine = 0.5;
        assert_eq!(classify(&mut world, object, &mut context), 0xE);
        let held = world.get::<MaterialProperty>(material).unwrap();
        assert_eq!((held.raw_state_54, held.values.shine), (2, 4.0));
        world
            .get_mut::<MaterialProperty>(material)
            .unwrap()
            .values
            .shine = f32::NAN;
        assert_eq!(classify(&mut world, object, &mut context), 0xE);
        assert_eq!(
            world
                .get::<MaterialProperty>(material)
                .unwrap()
                .raw_state_54,
            2
        );
        // Below render path 2 "skin" isn't tested.
        context.render_path = 1;
        assert_eq!(classify(&mut world, object, &mut context), 1);
        context.render_path = 5;
        // The apply mode.
        let object = textured_shape(&mut world, None, name);
        let texturing = world.get::<NiAvProperties>(object).unwrap().0[0].unwrap();
        world
            .get_mut::<TexturingProperty>(texturing)
            .unwrap()
            .flags_18 = 0xFF08;
        assert_eq!(classify(&mut world, object, &mut context), 0xF);
        world
            .get_mut::<TexturingProperty>(texturing)
            .unwrap()
            .flags_18 = 4;
        assert_eq!(classify(&mut world, object, &mut context), 1);
        // "dynalpha" with no alpha takes 007D1B7C's new property; only shaders 3.0 changes the answer.
        let object = textured_shape(&mut world, Some(b"dynalphA9"), name);
        assert_eq!(classify(&mut world, object, &mut context), 1);
        context.shaders_30_b42ea5 = true;
        assert_eq!(classify(&mut world, object, &mut context), 0x1A);
        // An alpha with bit 0.
        let object = textured_shape(&mut world, None, name);
        let alpha = world
            .spawn((
                NiClass::AlphaProperty,
                AlphaProperty {
                    flags_18: 1,
                    ..AlphaProperty::default()
                },
            ))
            .id();
        world
            .get_mut::<NiAvProperties>(object)
            .unwrap()
            .0
            .push(Some(alpha));
        assert_eq!(classify(&mut world, object, &mut context), 0x1A);
        context.shaders_30_b42ea5 = false;
        // No normal map and a2 = 0 is 0; a2 = 1 goes on.
        let mut missing = |_: &mut World, _: &[u8]| false;
        context.load_normal_map = Some(&mut missing);
        let object = textured_shape(&mut world, None, name);
        assert_eq!(classify(&mut world, object, &mut context), 0);
        assert_eq!(
            classify_geometry_shader(&mut world, object, true, &mut context).unwrap(),
            1
        );
        // A particle system answers 0 before anything else (vt+0x14).
        let system = world
            .spawn((NiClass::ParticleSystem, ParticleSystem::default()))
            .id();
        assert_eq!(classify(&mut world, system, &mut context), 0);
    }

    #[test]
    fn find_property_takes_the_head_first_match_below_ten() {
        let mut world = World::new();
        let object = shape(&mut world);
        let first = world
            .spawn((NiClass::AlphaProperty, AlphaProperty::default()))
            .id();
        let second = world
            .spawn((NiClass::AlphaProperty, AlphaProperty::default()))
            .id();
        world
            .entity_mut(object)
            .insert(NiAvProperties(vec![None, Some(first), Some(second)]));
        assert_eq!(
            find_property_of_type(&world, object, 0).unwrap(),
            Some(first)
        );
        assert_eq!(find_property_of_type(&world, object, 1).unwrap(), None);
        assert_eq!(find_property_of_type(&world, object, 10).unwrap(), None);
        assert_eq!(
            detach_property_of_type(&mut world, object, 0).unwrap(),
            Some(first)
        );
        assert_eq!(
            world.get::<NiAvProperties>(object).unwrap().0,
            [None, Some(second)]
        );
    }

    #[test]
    fn prepare_sets_the_shader_property_and_classifier_once() {
        let mut world = World::new();
        let slot = sky_slot();
        let mut requested = Vec::new();
        let mut get = |index: u32| {
            requested.push(index);
            Some(Arc::clone(&slot))
        };
        let mut context = context(&mut get);
        let object = shape(&mut world);
        assert!(
            prepare_object_geometry(
                &mut world,
                Some(object),
                SKY_SHADER_SLOT_INDEX,
                0,
                1,
                &mut context
            )
            .unwrap()
        );
        let list = world.get::<NiAvProperties>(object).unwrap().0.clone();
        assert_eq!(list.len(), 1);
        let property = list[0].unwrap();
        assert_eq!(
            *world.get::<SkyShaderProperty>(property).unwrap(),
            SkyShaderProperty::default()
        );
        // A second prepare: the geometry has both, so it re-runs +0x18 without a slot request, and keeps its one property.
        assert!(
            prepare_object_geometry(
                &mut world,
                Some(object),
                SKY_SHADER_SLOT_INDEX,
                0,
                1,
                &mut context
            )
            .unwrap()
        );
        assert_eq!(world.get::<NiAvProperties>(object).unwrap().0, list);
        assert_eq!(context.sky_shader_trace, [object, object]);
        drop(context);
        assert_eq!(requested, [10]);
    }

    #[test]
    fn prepare_arms_for_a_null_object_render_path_zero_and_bad_indexes() {
        let mut world = World::new();
        let mut get = |_: u32| None;
        let mut context = context(&mut get);
        assert!(!prepare_object_geometry(&mut world, None, 10, 0, 1, &mut context).unwrap());
        let object = shape(&mut world);
        // A null slot returns 0.
        assert!(!prepare_geometry(&mut world, Some(object), 10, 0, 1, 0, &mut context).unwrap());
        // A zero index returns 0.
        assert!(!prepare_geometry(&mut world, Some(object), 0, 0, 1, 0, &mut context).unwrap());
        // Index 0x1D warns and falls to index 1 without 007D1920, since the original index was not 1.
        assert!(!prepare_geometry(&mut world, Some(object), 0x1D, 0, 1, 0, &mut context).unwrap());
        assert_eq!(
            context.warnings,
            ["Object prepared with invalid shader index"]
        );
        // The original index 1 goes to 007D1920: a shape with no texturing property answers 0, with the 00707610 refresh before it.
        assert!(!prepare_geometry(&mut world, Some(object), 1, 0, 1, 0, &mut context).unwrap());
        assert_eq!(context.classify_trace, [(object, 0)]);
        context.render_path = 0;
        assert!(!prepare_geometry(&mut world, Some(object), 1, 0, 1, 0, &mut context).unwrap());
        assert!(context.sky_shader_trace.is_empty());
    }

    #[test]
    fn a_node_ors_its_nonnull_children_and_other_classes_do_nothing() {
        let mut world = World::new();
        let slot = sky_slot();
        let mut get = |_: u32| Some(Arc::clone(&slot));
        let mut context = context(&mut get);
        let node = spawn_scene_node(&mut world, SceneNodePayload::default());
        let first = shape(&mut world);
        world
            .get_mut::<NiNodeChildren>(node)
            .unwrap()
            .slots_ac
            .extend([None, Some(first)]);
        assert!(prepare_geometry(&mut world, Some(node), 10, 0, 1, 7, &mut context).unwrap());
        let property = find_property_of_type(&world, first, 4).unwrap().unwrap();
        assert_eq!(
            world
                .get::<SkyShaderProperty>(property)
                .unwrap()
                .classifier_68,
            7
        );
        let light = crate::gamebryo::scene_derived::spawn_directional_light(&mut world);
        assert!(!prepare_geometry(&mut world, Some(light), 10, 0, 1, 0, &mut context).unwrap());
    }

    #[test]
    fn prepare_replaces_a_foreign_type_four_property_and_stops_on_an_unbound_shader() {
        let mut world = World::new();
        let slot = sky_slot();
        let mut get = |_: u32| Some(Arc::clone(&slot));
        let mut context = context(&mut get);
        let object = shape(&mut world);
        let water = world.spawn(WaterShaderProperty::default()).id();
        world
            .entity_mut(object)
            .insert(NiAvProperties(vec![Some(water)]));
        // 007BDB1A detaches id 12; the new SkyShaderProperty takes its place.
        assert!(prepare_geometry(&mut world, Some(object), 10, 0, 1, 0, &mut context).unwrap());
        let list = world.get::<NiAvProperties>(object).unwrap().0.clone();
        assert_eq!(list.len(), 1);
        assert!(world.get::<SkyShaderProperty>(list[0].unwrap()).is_some());

        // An unported class stops where Phase 13e plugs in (WaterShaderHeightMap's vtable; this slot was WaterShader's until Phase 09 bound it).
        let mut other = SkyShader::default();
        other.base.original_vtable = 0x00A9_1B74;
        let other = Arc::new(BsShaderSlot {
            declaration: None,
            shader: Some(Arc::new(other)),
        });
        let mut get_other = |_: u32| Some(Arc::clone(&other));
        let mut water_context = super::tests::context(&mut get_other);
        let second = shape(&mut world);
        assert!(
            prepare_geometry(&mut world, Some(second), 17, 0, 1, 0, &mut water_context).is_err()
        );
    }

    #[test]
    fn water_prepare_arms() {
        let mut world = World::new();
        let mut shader = SkyShader::default();
        shader.base.original_vtable = WATER_SHADER_VTABLE;
        let slot = Arc::new(BsShaderSlot {
            declaration: None,
            shader: Some(Arc::new(shader)),
        });
        let mut get = |_: u32| Some(Arc::clone(&slot));
        let mut context = context(&mut get);
        // 007DD3F6 a new property; 0085BBF8 clears the lava byte, as no material is named "lava".
        context.lava_b47844 = 1;
        let plain = shape(&mut world);
        assert!(prepare_geometry(&mut world, Some(plain), 17, 0, 1, 7, &mut context).unwrap());
        let list = world.get::<NiAvProperties>(plain).unwrap().0.clone();
        assert_eq!(list.len(), 1);
        let water = world.get::<WaterShaderProperty>(list[0].unwrap()).unwrap();
        assert_eq!(water.classifier_68, 7, "007B80DE");
        assert_eq!(context.lava_b47844, 0);
        assert_eq!(context.water_shader_trace, [plain]);
        // 007DD3AB an existing id 12 returns 1 and adds nothing; the second pass goes through 007B803E, which only re-stores +0x68.
        assert!(prepare_geometry(&mut world, Some(plain), 17, 0, 1, 9, &mut context).unwrap());
        assert_eq!(world.get::<NiAvProperties>(plain).unwrap().0, list);
        assert_eq!(
            world
                .get::<WaterShaderProperty>(list[0].unwrap())
                .unwrap()
                .classifier_68,
            9
        );
        // 007DD3C8 a foreign type-4 property is detached; 007DD427 a "lava" material sets the byte, whatever the case.
        let lava = shape(&mut world);
        let foreign = world.spawn(SkyShaderProperty::default()).id();
        let material = world
            .spawn((
                NiClass::MaterialProperty,
                NiObjectNet {
                    name: Some(b"LAVA".to_vec()),
                    ..NiObjectNet::default()
                },
            ))
            .id();
        world
            .entity_mut(lava)
            .insert(NiAvProperties(vec![Some(foreign), Some(material)]));
        assert!(water_shader_prepare(&mut world, lava, &mut context).unwrap());
        assert_eq!(context.lava_b47844, 1, "007DD44B");
        let list = world.get::<NiAvProperties>(lava).unwrap().0.clone();
        assert_eq!(list.len(), 2);
        assert!(world.get::<WaterShaderProperty>(list[0].unwrap()).is_some());
        assert_eq!(list[1], Some(material));
    }
}
