//! Phase 09 D7: 0055F750 BSTreeManager_ResetSingleton(recreate), called at 00406654 with 0 after "Initializing TreeManager..." (the IDB's UIRenderProperties_* names are wrong; the
//! strings and the SpeedTree calls identify the class): an existing 00B39E04 is kept unless `recreate` asks for 0055F550 + FormHeap_Free; a null one gets FormHeap(0x28) 0055E850: the
//! z-buffer, material, vertex-colour and alpha properties the tree renderer shares, a BSXFlags(3) "BSX", the SpeedTree wind matrices (CSpeedTreeRT::SetNumWindMatrices(4) on 00B42A10),
//! the 00B4297C/00B4297D bytes, 00B44F04 = iCanopyShadowScale and the LockFreeMap<TESObjectREFR*, BSTreeNode*> (0055E720(2, 37, 12)). State only (no tree is loaded before Phase 15+).
//! Evidence: evidence/phase09-window-2026-10-06/disasm-09f.txt (0055F750, 0055E850, 0055F550, 006FA890, 0078BD80, 00787830, 0055E720, 0055E390, 0055F3C0), the 007A66B0/007A6670 identity
//! stores; P09-RUN-001 records 651..659 (00406659).

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::nif_objects::BsxFlagsExtraData;
use super::task_manager::LockFreeMap;
use crate::gamebryo::object_net::{AlphaProperty, NiExtraDataName, NiObjectNet};
use crate::gamebryo::properties::{
    MaterialProperty, VertexColorProperty, ZBufferProperty, new_material_property,
};
use crate::gamebryo::stream_objects::NiClass;
use crate::tes::settings::SettingCollection;

/// IDA: LockFreeMap<TESObjectREFR*, BSTreeNode*> vtable (0055E757).
pub const TREE_NODE_MAP_VTABLE: u32 = 0x00A6_51C4;
/// IDA: 006FA890's BSXFlags vtable.
pub const BSX_FLAGS_VTABLE: u32 = 0x00A7_D14C;

// IDA: the SpeedTree INI statics 0055E850 reads.
const FORCE_FULL_LOD: u32 = 0x00B1_25F0;
const CANOPY_SHADOW_SCALE: u32 = 0x00B1_2630;

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// IDA: 00B42A10 the CSpeedTreeRT wind matrix static (SpeedTree SDK): +0x00 u16 count, +0x04 FormHeap array of count 4x4 float matrices, each built by 007A66B0 and reset by 007A6670 (both
/// store the identity).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpeedTreeWindMatrices {
    pub count_00: u16,
    pub matrices_04: Vec<[f32; 16]>,
    pub allocated_04: bool,
}

/// IDA: 0055E850 BSTreeManager_Constructor (0x28; three-part BSTreeManager in the IDB): +0x00 the cached-property map (null; 0055E390 purges it), +0x04 NiPointer (null), +0x08
/// NiZBufferProperty, +0x0C NiMaterialProperty, +0x10 NiVertexColorProperty, +0x14 NiAlphaProperty, +0x18 BSXFlags, +0x1C 0.0, bytes +0x20 = +0x21 = 1, +0x22 = 0, +0x23 =
/// bForceFullLOD:SpeedTree, +0x24 the tree-node LockFreeMap. Completion: 100% of the constructor stores.
/// TODO: [ ] Name +0x00/+0x04/+0x1C..+0x22 with the tree loader (Phase 15+, M5).
#[derive(Clone, Debug, PartialEq)]
pub struct BsTreeManager {
    pub cached_properties_00: u32,
    pub pointer_04: Option<Entity>,
    pub zbuffer_08: Option<Entity>,
    pub material_0c: Option<Entity>,
    pub vertex_color_10: Option<Entity>,
    pub alpha_14: Option<Entity>,
    pub bsx_flags_18: Option<Entity>,
    pub float_1c: f32,
    pub byte_20: u8,
    pub byte_21: u8,
    pub byte_22: u8,
    pub force_full_lod_23: u8,
    pub tree_nodes_24: Option<LockFreeMap>,
}

impl Default for BsTreeManager {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            cached_properties_00: 0,
            pointer_04: None,
            zbuffer_08: None,
            material_0c: None,
            vertex_color_10: None,
            alpha_14: None,
            bsx_flags_18: None,
            float_1c: 0.0,
            byte_20: 1,
            byte_21: 1,
            byte_22: 0,
            force_full_lod_23: 0,
            tree_nodes_24: None,
        }
    }
}

/// IDA: the statics 0055F750 and its callees write. 0055F550 releases the four NiPointers (null at startup) and stores 1 into 00B06A28 (`TesGlobals::grid_changed_b06a28`, passed by
/// reference).
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct TreeManagerGlobals {
    pub tree_manager_b39e04: Option<BsTreeManager>,
    pub wind_matrices_b42a10: SpeedTreeWindMatrices,
    /// 007871C0.
    pub byte_b4297c: u8,
    /// 00787690.
    pub byte_b4297d: u8,
    pub canopy_shadow_scale_b44f04: f32,
    pub pointer_b43108: Option<Entity>,
    pub pointer_b4310c: Option<Entity>,
    pub pointer_b35414: Option<Entity>,
    pub pointer_b35418: Option<Entity>,
}

// IDA: N/A a registered SpeedTree INI static.
fn ini_raw(ini: &SettingCollection, address: u32) -> Result<u32, StreamError> {
    ini.find_address(address)
        .map(|setting| setting.raw)
        .ok_or(StreamError("0055E850: missing SpeedTree INI static"))
}

// IDA: 0078BD80 CSpeedTreeRT::SetNumWindMatrices(count) -> 00787830 on 00B42A10: FormHeap_Free(+0x04), +0x00 = (u16)count, FormHeap(0x40 * count) with 007A66B0 per matrix (identity), then
// 007A6670 on each (identity again). The SDK's "%s - failed [%s]" catch arms are not reached. Completion: 100% of the stores.
fn set_wind_matrix_count(wind: &mut SpeedTreeWindMatrices, count: u16) {
    wind.count_00 = count;
    wind.matrices_04 = vec![IDENTITY; usize::from(count)];
    wind.allocated_04 = true;
    wind.matrices_04.fill(IDENTITY);
}

// IDA: 0055E850 BSTreeManager_Constructor. Completion: 100% of the store order.
pub fn construct_tree_manager(
    world: &mut World,
    globals: &mut TreeManagerGlobals,
    ini: &SettingCollection,
) -> Result<BsTreeManager, StreamError> {
    let mut manager = BsTreeManager {
        // 0055E87F..0055E8B3 +0x04..+0x18 = 0, +0x1C = 0.0, +0x00 = 0, +0x21 = +0x20 = 1, +0x22 = 0, +0x23 = bForceFullLOD (byte).
        force_full_lod_23: ini_raw(ini, FORCE_FULL_LOD)? as u8,
        ..BsTreeManager::default()
    };
    // 0055E8AE..0055E943 NiZBufferProperty (+0x18 = 0x0F) -> +0x08 (NiPointer assign); +0x18 |= 1, |= 2, then (& 0xFFCF) | 0x0C.
    let mut z = ZBufferProperty::default();
    z.flags_18 |= 1;
    z.flags_18 |= 2;
    z.flags_18 = (z.flags_18 & 0xFFCF) | 0x0C;
    manager.zbuffer_08 = Some(
        world
            .spawn((NiClass::ZBufferProperty, NiObjectNet::default(), z))
            .id(),
    );
    // 0055E945..0055EA23 NiMaterialProperty (007095A0) -> +0x0C; diffuse (+0x28..+0x30) = 1.0, +0x54 += 1 twice, ambient (+0x1C..+0x24) = 1.0, +0x54 += 1, emissive (+0x40..+0x48) = 1.0.
    let mut material: MaterialProperty = new_material_property(world);
    material.values.diffuse = [1.0; 3];
    material.raw_state_54 += 1;
    material.raw_state_54 += 1;
    material.values.ambient = [1.0; 3];
    material.raw_state_54 += 1;
    material.values.emissive = [1.0; 3];
    manager.material_0c = Some(
        world
            .spawn((NiClass::MaterialProperty, NiObjectNet::default(), material))
            .id(),
    );
    // 0055EA26..0055EAA0 NiVertexColorProperty (+0x18 = 8) -> +0x10; +0x18 &= 0xFFCF, then |= 8.
    let mut vertex_color = VertexColorProperty::default();
    vertex_color.flags_18 &= 0xFFCF;
    vertex_color.flags_18 |= 8;
    manager.vertex_color_10 = Some(
        world
            .spawn((
                NiClass::VertexColorProperty,
                NiObjectNet::default(),
                vertex_color,
            ))
            .id(),
    );
    // 0055EAA4..0055EB3E NiAlphaProperty (+0x18 = 0xEC, +0x1A = 0) -> +0x14; +0x18 |= 0x200, (& 0xF3FF) | 0x1000, +0x1A = 0x54, +0x18 &= 0xFFFE.
    let mut alpha = AlphaProperty::default();
    alpha.flags_18 |= 0x0200;
    alpha.flags_18 = (alpha.flags_18 & 0xF3FF) | 0x1000;
    alpha.alpha_test_ref_1a = 0x54;
    alpha.flags_18 &= 0xFFFE;
    manager.alpha_14 = Some(
        world
            .spawn((NiClass::AlphaProperty, NiObjectNet::default(), alpha))
            .id(),
    );
    // 0055EB44..0055EBA1 FormHeap(0x10) 006FA890(3): NiExtraData, +0x0C = 3, vtable A7D14C, SetName("BSX") -> +0x18.
    manager.bsx_flags_18 = Some(
        world
            .spawn((
                NiClass::BsxFlagsExtraData,
                NiExtraDataName(Some(b"BSX".to_vec())),
                BsxFlagsExtraData { flags_0c: 3 },
            ))
            .id(),
    );
    // 0055EBA9 0078BD80(4); 0055EBB0 007871C0(1); 0055EBB7 00787690(1).
    set_wind_matrix_count(&mut globals.wind_matrices_b42a10, 4);
    globals.byte_b4297c = 1;
    globals.byte_b4297d = 1;
    // 0055EBBC..0055EBD3 00B44F04 = float(iCanopyShadowScale) (unsigned; the main thread's 24-bit fild).
    globals.canopy_shadow_scale_b44f04 =
        recomp_core::x87::u32_to_x87_float(ini_raw(ini, CANOPY_SHADOW_SCALE)?);
    // 0055EBD9..0055EBFF FormHeap(0x1C) 0055E720(2, 37, 12) -> +0x24.
    manager.tree_nodes_24 = Some(LockFreeMap::new(TREE_NODE_MAP_VTABLE, 2, 37, 12));
    Ok(manager)
}

// IDA: 0055F550 BSTreeManager_Destructor: 0055E390(0) (a nonnull +0x00 map is a host stop), 00B43108/00B4310C released, 00B06A28 = 1, 004A3C60 (00B35414/00B35418 released), the +0x24 map
// freed (0055F3C0(1) and its arrays), then +0x18..+0x04 released. Completion: 100% of the order of the reached effects. Properties have no count here, so a release clears the field and leaves
// the entity, as `NiProcessDefaults` does.
pub fn destroy_tree_manager(
    manager: &mut BsTreeManager,
    globals: &mut TreeManagerGlobals,
    grid_changed_b06a28: &mut u8,
) -> Result<(), StreamError> {
    // 0055F586 0055E390(0): a null +0x00 map returns at once.
    if manager.cached_properties_00 != 0 {
        return Err(StreamError(
            "0055E390: purging a cached-property map is not ported",
        ));
    }
    // 0055F58B..0055F5E3 00B43108 and 00B4310C released and cleared.
    globals.pointer_b43108 = None;
    globals.pointer_b4310c = None;
    // 0055F5ED 00B06A28 = 1; 0055F5F4 004A3C60.
    *grid_changed_b06a28 = 1;
    globals.pointer_b35414 = None;
    globals.pointer_b35418 = None;
    // 0055F5F9..0055F62F the +0x24 map: 0055F3C0(1) (TLS lock freed, the buckets cleared), +0x0C and +0x04 freed, the map freed.
    manager.tree_nodes_24 = None;
    // 0055F632..0055F70D +0x18, +0x14, +0x10, +0x0C, +0x08, +0x04 released.
    manager.bsx_flags_18 = None;
    manager.alpha_14 = None;
    manager.vertex_color_10 = None;
    manager.material_0c = None;
    manager.zbuffer_08 = None;
    manager.pointer_04 = None;
    Ok(())
}

// IDA: 0055F750 BSTreeManager_ResetSingleton(recreate). Completion: 100% of both arms of the 00B39E04 test.
pub fn reset_tree_manager(
    world: &mut World,
    recreate: bool,
    ini: &SettingCollection,
    grid_changed_b06a28: &mut u8,
) -> Result<(), StreamError> {
    world.init_resource::<TreeManagerGlobals>();
    world.resource_scope(|world, mut globals: Mut<TreeManagerGlobals>| {
        // 0055F772..0055F79D an existing singleton is kept unless `recreate`; then 0055F550, FormHeap_Free and 00B39E04 = 0.
        if let Some(mut manager) = globals.tree_manager_b39e04.take() {
            if !recreate {
                globals.tree_manager_b39e04 = Some(manager);
                return Ok(());
            }
            destroy_tree_manager(&mut manager, &mut globals, grid_changed_b06a28)?;
        }
        // 0055F79D..0055F7C2 FormHeap(0x28) 0055E850 -> 00B39E04.
        let manager = construct_tree_manager(world, &mut globals, ini)?;
        globals.tree_manager_b39e04 = Some(manager);
        Ok(())
    })
}
