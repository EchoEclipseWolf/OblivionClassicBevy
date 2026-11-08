//! Phase 09 (D2): the Bethesda-side Havok setup TES_Constructor 00441970 runs: 005350F0 bhk_InitializeSystem (0088B070 once: the error callbacks,
//! the collision layer tables 008A83C0, bhkCollisionFilter 0088A570 and the 43 bhk factory registrations; HavokError 00534430 with its disabled
//! message IDs and hkVisualDebugger.cpp lines; HavokStreambufFactory; the "TES4" directory trim; six movement floats; 008896F0 iSimType), 00537760
//! and the 004D5600 world call under GridCellArray vt+4.
//!
//! Host substitution (D2): the Havok SDK runtime is not ported. bhkMemory (0x2C, 008A7060) and hkBaseSystem init 008BBA80 (which sets 00BA8040), the
//! 16-aligned 393,216-byte block 00B47900 handed to the TLS memory router, the 240 class/type statics 008BB420 reads, and hkWorld creation 004D5000
//! (bhkWorldM with TESWindListener/TESTrapListener/TESArrowTargetListener and the 00B3659C singleton) have no host counterpart until a physics
//! phase. The host keeps the Bethesda-visible globals they write.
//!
//! x87: the main thread is 24-bit here, so the float stores are plain `f32` in the original grouping.
//! Evidence: evidence/phase09-window-2026-10-06 (disasm-09c.txt, disasm-09i.txt, havok_layer_tables.py), P09-RUN-001.

use bevy_ecs::prelude::*;

use super::havok_layers::apply_layer_stores;
use crate::gamebryo::registration::NiFactoryRegistry;
use crate::tes::game_settings::GameSettingCollection;

/// IDA: vtables stored by the Havok setup.
pub const HAVOK_ERROR_VTABLE: u32 = 0x00A5_6214;
pub const HAVOK_STREAMBUF_FACTORY_VTABLE: u32 = 0x00A5_61E0;
pub const BHK_COLLISION_FILTER_VTABLE: u32 = 0x00A9_5CE4;
pub const BHK_COLLISION_FILTER_INTERFACE_VTABLES: [u32; 4] =
    [0x00A9_5CDC, 0x00A9_5CD0, 0x00A9_5CC4, 0x00A9_5CBC];

// IDA: 008B0E80 slot hash: ((key >> 4) * 0x9E3779B1) & mask. Completion: 100%.
const fn havok_pointer_hash(key: u32, mask: u32) -> u32 {
    (key >> 4).wrapping_mul(0x9E37_79B1) & mask
}

/// IDA: 0088B17A..0088B40E the 43 0088B070 registrations: class name and the original constructor each NiFactoryRegistry_InsertOrAssign call stores
/// (P09-RUN-001 records 12..54; bhkMeshShape and bhkNiTriStripsShape share 008C62B0). Completion: 100% of the order.
pub const BHK_FACTORIES: [(&str, u32); 43] = [
    ("bhkBoxShape", 0x008B_7E10),
    ("bhkConvexSweepShape", 0x008C_9C80),
    ("bhkConvexTransformShape", 0x008C_93F0),
    ("bhkConvexVerticesShape", 0x008C_8900),
    ("bhkCylinderShape", 0x008C_8100),
    ("bhkMultiSphereShape", 0x008B_7470),
    ("bhkMeshShape", 0x008C_62B0),
    ("bhkNiTriStripsShape", 0x008C_62B0),
    ("bhkPackedNiTriStripsShape", 0x008C_5160),
    ("hkPackedNiTriStripsData", 0x008C_4840),
    ("bhkPlaneShape", 0x008C_4210),
    ("bhkSphereShape", 0x008A_F380),
    ("bhkTriangleShape", 0x008C_3D20),
    ("bhkMoppBvTreeShape", 0x008C_33C0),
    ("bhkTransformShape", 0x008A_1B80),
    ("bhkCapsuleShape", 0x008B_6780),
    ("bhkListShape", 0x008A_0EF0),
    ("bhkBallAndSocketConstraint", 0x008C_2FB0),
    ("bhkHingeConstraint", 0x008C_2690),
    ("bhkFixedConstraint", 0x008C_2020),
    ("bhkLimitedHingeConstraint", 0x008B_2BE0),
    ("bhkPrismaticConstraint", 0x008C_17B0),
    ("bhkRagdollConstraint", 0x008C_09E0),
    ("bhkStiffSpringConstraint", 0x008C_05C0),
    ("bhkWheelConstraint", 0x008B_FE90),
    ("bhkBreakableConstraint", 0x008B_F6C0),
    ("bhkMalleableConstraint", 0x008B_EFC0),
    ("bhkMouseSpringAction", 0x0089_E430),
    ("bhkMotorAction", 0x008B_E8E0),
    ("bhkDashpotAction", 0x008B_E240),
    ("bhkAngularDashpotAction", 0x008B_DCE0),
    ("bhkSpringAction", 0x008B_D780),
    ("bhkAabbPhantom", 0x008B_A750),
    ("bhkCollisionObject", 0x0089_E960),
    ("bhkPCollisionObject", 0x0089_EFC0),
    ("bhkSPCollisionObject", 0x008B_71B0),
    ("bhkBlendCollisionObject", 0x0088_ED20),
    ("bhkBlendController", 0x008A_A920),
    ("bhkRigidBody", 0x008A_41F0),
    ("bhkRigidBodyT", 0x008B_97A0),
    ("bhkSimpleShapePhantom", 0x008A_F070),
    ("bhkCachingShapePhantom", 0x008B_D350),
    ("bhkExtraData", 0x008B_CF80),
];

// IDA: 00534430 the 24 HavokError disabled message IDs in insert order (0xAD67FA3A twice). Completion: 100%.
const HAVOK_DISABLED_IDS: [u32; 24] = [
    0x6E8D_163B,
    0x6CEE_9071,
    0xF0DE_4355,
    0x2FF8_C16F,
    0xF0FE_4356,
    0x275E_C1FD,
    0x1ADA_AD0E,
    0x70DC_41CB,
    0x1FF8_8F0E,
    0x475D_86B1,
    0x6FE8_4A9B,
    0x21C8_AB2A,
    0x3093_14D9,
    0xAD67_FA3A,
    0xAD67_FA3A,
    0xF032_DE34,
    0x34DF_5494,
    0xF02E_32DF,
    0xF043_D534,
    0xF02D_E43E,
    0xF021_32DF,
    0xF021_32FF,
    0x2A1D_B936,
    0x68C4_E1DC,
];

// IDA: 005345DD..00534677 the hkVisualDebugger.cpp lines. Completion: 100%.
const HAVOK_VISUAL_DEBUGGER_LINES: [i32; 11] = [38, 86, 93, 106, 110, 115, 126, 133, 191, 246, 277];

/// IDA: 008B0E10 hkPointerMap<uint, uint> (0x0C bytes): +0x00 storage (mask + 1 keys, then mask + 1 values; key 0 = empty), +0x04 count, +0x08 mask.
/// The constructor allocates 0x80 bytes (16 slots) and zeroes all of them. Completion: 100%.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HavokPointerMap {
    keys: Vec<u32>,
    values: Vec<u32>,
    count: u32,
    mask: u32,
}

impl Default for HavokPointerMap {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            keys: vec![0; 16],
            values: vec![0; 16],
            count: 0,
            mask: 0x0F,
        }
    }
}

impl HavokPointerMap {
    // IDA: 008B1060 Resize(newCapacity): new keys cleared, mask = capacity - 1, count = 0, the old occupied slots reinserted in slot order. The values
    // of a resize stay uninitialized Havok-allocator bytes in the original (it clears only the keys); the host zeroes them, so compare occupied slots
    // only. Completion: 100%.
    fn resize(&mut self, capacity: u32) {
        let old_keys = std::mem::replace(&mut self.keys, vec![0; capacity as usize]);
        let old_values = std::mem::replace(&mut self.values, vec![0; capacity as usize]);
        self.mask = capacity - 1;
        self.count = 0;
        for (slot, key) in old_keys.iter().enumerate() {
            if *key != 0 {
                self.insert(*key, old_values[slot]);
            }
        }
    }

    // IDA: 008B0E80 Insert(key, value): resize to 2 * mask + 2 (008B1060) when 2 * count > mask; slot = ((key >> 4) * 0x9E3779B1) & mask, then linear
    // probing to the key or an empty slot; count += (slot != key); the key and value are stored. Completion: 100%.
    pub fn insert(&mut self, key: u32, value: u32) {
        if 2 * self.count > self.mask {
            self.resize(2 * self.mask + 2);
        }
        let mut slot = havok_pointer_hash(key, self.mask) as usize;
        while self.keys[slot] != 0 && self.keys[slot] != key {
            slot = (slot + 1) & self.mask as usize;
        }
        self.count += u32::from(self.keys[slot] != key);
        self.keys[slot] = key;
        self.values[slot] = value;
    }

    /// IDA: +0x04.
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }
    /// IDA: +0x08.
    #[must_use]
    pub const fn mask(&self) -> u32 {
        self.mask
    }
    /// IDA: +0x00 the key half of the storage.
    #[must_use]
    pub fn keys(&self) -> &[u32] {
        &self.keys
    }
    /// IDA: the value half of the storage.
    #[must_use]
    pub fn values(&self) -> &[u32] {
        &self.values
    }
}

/// IDA: 005340F0's {file copy, line} entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HavokErrorReportSource {
    pub file: Vec<u8>,
    pub line: i32,
}

/// IDA: 00534430 HavokError_Constructor on the 0x124-byte block 005350F0 takes from the Havok allocator (00BA7D98 vt+0x10, size class 0x15): +0x00
/// vtable A56214, +0x04 u16 memory size 0x124 (stored by 00535142 before the constructor), +0x06 u16 reference count 1, +0x08 hkArray<report source>
/// (+0x08 data, +0x0C size, +0x10 capacity and flags, initially 0x80000000), +0x14 hkPointerMap of disabled message IDs (24 inserts, one duplicate),
/// +0x20 char[260] the path prefix before "TES4" (byte 0 = 0). Completion: 100%.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HavokError {
    pub original_vtable: u32,
    pub memory_size_04: u16,
    pub reference_count_06: u16,
    pub reports_08: Vec<HavokErrorReportSource>,
    /// IDA: 008A6EE0 growth: capacity 2 * size (1 when empty); the flags word keeps only bit 30 of the old value.
    pub report_capacity_10: u32,
    pub disabled_ids_14: HavokPointerMap,
    pub directory_20: Vec<u8>,
}

/// IDA: 005351AA HavokStreambufFactory (8 bytes from the Havok allocator): +0x00 vtable A561E0, +0x04 u16 memory size 8, +0x06 u16 reference count 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HavokStreambufFactory {
    pub original_vtable: u32,
    pub memory_size_04: u16,
    pub reference_count_06: u16,
}

/// IDA: 0088A570 bhkCollisionFilter_Constructor on the 0x18-byte Havok block 0088B16B allocates: +0x00 vtable A95CE4, +0x04 u16 memory size 0x18
/// (0088B16F), +0x06 u16 reference count 1, +0x08..+0x14 the four filter interface vtables. Completion: 100%.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BhkCollisionFilter {
    pub original_vtable: u32,
    pub memory_size_04: u16,
    pub reference_count_06: u16,
    pub interface_vtables_08: [u32; 4],
}

/// IDA: 004D5000 bhkWorld creation (host substitution): a non-null owner for 00B35C24 so readers see the world as present; the hkWorld, its listeners
/// and the 00B3659C singleton are not created.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BhkWorldSubstitute {
    /// IDA: 004D52C6 world +0x19 = (00B33A34 == 0): kept as the computed value.
    pub byte_19: u8,
}

/// IDA: N/A the settings 005350F0 / 004D5000 / TES_Constructor read: GMSTs through the GameSettingCollection, INI values through the static collection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HavokSettingInputs {
    /// 00B376F8 fMaximumWind.
    pub maximum_wind: f32,
    /// 00B374B0 fJumpMoveBase.
    pub jump_move_base: f32,
    /// 00B374B8 fJumpMoveMult.
    pub jump_move_mult: f32,
    /// 00B374A8 fJumpDoubleMult.
    pub jump_double_mult: f32,
    /// 00B37470 fJumpFallVelocityMin.
    pub jump_fall_velocity_min: f32,
    /// 0056A360(3): 00B3A508 iSkillExpertMin.
    pub skill_expert_min: i32,
    /// 00B11638 iSimType:HAVOK.
    pub sim_type: i32,
    /// 00B097D8 iNumHavokThreads:HAVOK.
    pub num_havok_threads: i32,
    /// 00B097C0 fMaxTime:HAVOK.
    pub max_time: f32,
    /// 00B33A34 (read by 004D52BF).
    pub b33a34: u8,
    /// IDA: 005351F6 GetCurrentDirectoryA(0x104): the host passes its own (host substitution).
    pub current_directory: Vec<u8>,
}

/// IDA: 00B35C14, the static Setting of fSeenDataUpdateRadius (9E2FCF).
pub const SEEN_DATA_UPDATE_RADIUS_B35C14: u32 = 0x00B3_5C14;

/// IDA: the Havok globals Phase 09 writes. Defaults are the image values the host observes before the window (P09-RUN-001 records 4/5).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct HavokGlobals {
    /// IDA: 00BA8040 hkBaseSystem initialized flag (set inside 008BBA80).
    pub base_system_initialized: bool,
    /// IDA: 0088B0A6/0088B0B0 psub_889770 (00B2EB04) and psub_6078C0 (00B2EB08).
    pub error_report_callback_b2eb04: u32,
    pub memory_callback_b2eb08: u32,
    /// IDA: 00BA7DB0..00BA7F2F the 008A83C0 tables (0x180 bytes as dwords).
    pub layer_tables: [u32; 0x60],
    /// 00BA7904.
    pub collision_filter: Option<BhkCollisionFilter>,
    /// 00BA7FB0.
    pub error: Option<HavokError>,
    /// 00BA7FB4.
    pub streambuf: Option<HavokStreambufFactory>,
    pub maximum_wind_b2e8a8: f32,
    pub jump_move_base_ba7a60: f32,
    pub jump_move_mult_b2e76c: f32,
    pub jump_skill_factor_b2e774: f32,
    pub jump_double_mult_b2e770: f32,
    pub jump_fall_velocity_b2e778: f32,
    /// 00B2E2DC (008896F0).
    pub sim_type_rate_b2e2dc: u32,
    /// IDA: 00537760 psub_5376E0 (00BA7A50) = 005376E0.
    pub step_callback_ba7a50: u32,
    /// IDA: 004D5600 / 004D5000 (substitution): world owner and its globals.
    pub world_b35c24: Option<BhkWorldSubstitute>,
    pub b35c00: u32,
    pub num_havok_threads_b2e2d4: u32,
    pub max_time_b2e2e8: f32,
    /// IDA: N/A host record of the factory names 0088B070 registered, in order.
    pub registered_bhk_classes: Vec<&'static str>,
}

impl Default for HavokGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            base_system_initialized: false,
            error_report_callback_b2eb04: 0x008A_70A0,
            memory_callback_b2eb08: 0x008A_70B0,
            layer_tables: [0; 0x60],
            collision_filter: None,
            error: None,
            streambuf: None,
            maximum_wind_b2e8a8: 250.0,
            jump_move_base_ba7a60: 0.0,
            jump_move_mult_b2e76c: 0.3,
            jump_skill_factor_b2e774: 0.75,
            jump_double_mult_b2e770: 0.5,
            jump_fall_velocity_b2e778: 700.0,
            sim_type_rate_b2e2dc: 9,
            step_callback_ba7a50: 0,
            world_b35c24: None,
            b35c00: 0,
            num_havok_threads_b2e2d4: 1,
            max_time_b2e2e8: 1.0 / 60.0,
            registered_bhk_classes: Vec::new(),
        }
    }
}

// IDA: 005340F0 HavokError_AddDisabledReport(file, line): _splitpath keeps the file name and extension, strcat them, hkString::strDup (008B18F0) and
// append {copy, line} through 008A6EE0 growth. Completion: 100%.
fn add_disabled_report(error: &mut HavokError, path: &[u8], line: i32) {
    let file = path
        .rsplit(|&byte| matches!(byte, b'\\' | b'/' | b':'))
        .next()
        .unwrap_or(path);
    let size = error.reports_08.len() as u32;
    if size == error.report_capacity_10 & 0x3FFF_FFFF {
        // IDA: 008A6EE0 capacity = 2 * size (1 when empty); flags keep bit 30.
        let capacity = if size != 0 { 2 * size } else { 1 };
        error.report_capacity_10 = (error.report_capacity_10 & 0x4000_0000) | capacity;
    }
    error.reports_08.push(HavokErrorReportSource {
        file: file.to_vec(),
        line,
    });
}

// IDA: 00534430 HavokError_Constructor(this). Completion: 100% of the store order.
fn construct_havok_error() -> HavokError {
    let mut error = HavokError {
        original_vtable: HAVOK_ERROR_VTABLE,
        memory_size_04: 0x124,
        reference_count_06: 1,
        reports_08: Vec::new(),
        report_capacity_10: 0x8000_0000,
        // 00534483 008B0E10 hkPointerMap at +0x14.
        disabled_ids_14: HavokPointerMap::default(),
        directory_20: Vec::new(),
    };
    // 00534496.. 24 inserts of 1.
    for id in HAVOK_DISABLED_IDS {
        error.disabled_ids_14.insert(id, 1);
    }
    // 005345DD.. 11 report sources.
    for line in HAVOK_VISUAL_DEBUGGER_LINES {
        add_disabled_report(&mut error, b"hkVisualDebugger.cpp", line);
    }
    // 00534683 +0x20 = 0.
    error.directory_20.clear();
    error
}

// IDA: 008A83C0 bhkCollisionLayers_Initialize: three 0x80-byte memsets and 190 ordered dword stores (bs/havok_layers.rs). Completion: 100%.
pub fn initialize_collision_layer_tables(tables: &mut [u32; 0x60]) {
    apply_layer_stores(tables);
}

// IDA: 008896F0 bhk_SetSimTypeRate(iSimType): 00B2E2DC = 1 / 2 / 5 / 9 for 0 / 2 / 3 / 4, otherwise 4 (case 1 included). Completion: 100%.
#[must_use]
pub const fn havok_sim_type_rate(sim_type: i32) -> u32 {
    match sim_type {
        0 => 1,
        2 => 2,
        3 => 5,
        4 => 9,
        _ => 4,
    }
}

// IDA: 0056A360 SkillLevel_GetMinimum(level): 0 for 0, iSkillApprenticeMin / JourneymanMin / ExpertMin / MasterMin (00B3A4F8..00B3A510) for 1..4,
// otherwise 100. Completion: 100%. A missing setting reads 0, as in the C++ port.
#[must_use]
pub fn skill_level_minimum(settings: &GameSettingCollection, level: i32) -> i32 {
    let value = |key: &str| settings.find(key).map_or(0, |setting| setting.raw as i32);
    match level {
        0 => 0,
        1 => value("iSkillApprenticeMin"),
        2 => value("iSkillJourneymanMin"),
        3 => value("iSkillExpertMin"),
        4 => value("iSkillMasterMin"),
        _ => 100,
    }
}

// IDA: 0088B070 bhk_InitializeOnce: with 00BA8040 clear, the two callbacks, bhkMemory + 008BBA80 (substitution; sets 00BA8040), the 00B47900 block
// (substitution), 008A83C0, bhkCollisionFilter -> 00BA7904 and the 43 NiFactoryRegistry_InsertOrAssign calls in order. The registered constructors
// have no `spawn_cold` arm, so `NiFactoryRegistry::create` answers Unsupported until a later phase binds them. Completion: 100% of the order of the
// Bethesda-side effects.
// TODO: [ ] The Havok runtime and the bhk constructors (a physics phase, after M5).
pub fn havok_initialize_once(globals: &mut HavokGlobals, registry: &mut NiFactoryRegistry) {
    // 0088B092 cmp 00BA8040, 0: once.
    if globals.base_system_initialized {
        return;
    }
    // 0088B0A6/0088B0B0 the two callbacks.
    globals.error_report_callback_b2eb04 = 0x0088_9770;
    globals.memory_callback_b2eb08 = 0x0060_78C0;
    // 0088B0BA..0088B10E bhkMemory (008A7060) and 008BBA80 hkBaseSystem init: host substitution; 008BBA80 ends with 00BA8040 = 1.
    globals.base_system_initialized = true;
    // 0088B110..0088B155 the 00B47900 block to the TLS memory router: host substitution (no Havok allocator).
    // 0088B157 008A83C0.
    initialize_collision_layer_tables(&mut globals.layer_tables);
    // 0088B15C..0088B184 bhkCollisionFilter -> 00BA7904.
    globals.collision_filter = Some(BhkCollisionFilter {
        original_vtable: BHK_COLLISION_FILTER_VTABLE,
        memory_size_04: 0x18,
        reference_count_06: 1,
        interface_vtables_08: BHK_COLLISION_FILTER_INTERFACE_VTABLES,
    });
    // 0088B17A..0088B40E the 43 registrations.
    for (name, constructor) in BHK_FACTORIES {
        registry.insert_or_assign(name, constructor);
        globals.registered_bhk_classes.push(name);
    }
}

// IDA: N/A `strstr`.
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// IDA: 005350F0 bhk_InitializeSystem: 0088B070, the HavokError (replacing a previous 00BA7FB0 through its reference count), the HavokStreambufFactory
// (likewise 00BA7FB4), the "TES4"/"tes4" directory prefix copy into HavokError +0x20, the six floats and 008896F0(iSimType). Completion: 100% of the order.
pub fn initialize_havok_system(
    globals: &mut HavokGlobals,
    registry: &mut NiFactoryRegistry,
    inputs: &HavokSettingInputs,
) {
    // 00535129 0088B070.
    havok_initialize_once(globals, registry);
    // 0053512E..00535159 the 0x124 block and 00534430.
    let mut error = construct_havok_error();
    // 00535160..00535199 release the previous 00BA7FB0 (memory size nonzero, reference count to 0 -> vtable +0 delete), then store the new one. The
    // new owner replaces the slot whatever the count, so the release has no host effect.
    // 00535193..005351F0 the HavokStreambufFactory, likewise.
    globals.streambuf = Some(HavokStreambufFactory {
        original_vtable: HAVOK_STREAMBUF_FACTORY_VTABLE,
        memory_size_04: 8,
        reference_count_06: 1,
    });
    // 005351F6..00535240 the "TES4" (then "tes4") prefix into +0x20.
    let directory = &inputs.current_directory;
    let found = find_bytes(directory, b"TES4").or_else(|| find_bytes(directory, b"tes4"));
    if let Some(found) = found {
        error.directory_20 = directory[..found].to_vec();
    }
    globals.error = Some(error);
    // 00535245..005352A3 the six floats (24-bit x87: `f32`).
    globals.maximum_wind_b2e8a8 = inputs.maximum_wind;
    globals.jump_move_base_ba7a60 = inputs.jump_move_base;
    globals.jump_move_mult_b2e76c = inputs.jump_move_mult;
    // 0053526B fild 0056A360(3), fmul dbl_A3B150 (the float 0.01).
    globals.jump_skill_factor_b2e774 = inputs.skill_expert_min as f32 * 0.01;
    globals.jump_double_mult_b2e770 = inputs.jump_double_mult;
    // 0053529D fmul dbl_A39088 (the float 0.14287673).
    globals.jump_fall_velocity_b2e778 = inputs.jump_fall_velocity_min * 0.142_876_73;
    // 005352A9 008896F0(iSimType).
    globals.sim_type_rate_b2e2dc = havok_sim_type_rate(inputs.sim_type);
}

// IDA: 004D5600 bhkWorld_CreateDefault: 00B35C00 = 0, 004D5000 on the zero vector (substitution: 00B2E2D4 = iNumHavokThreads, world +0x19, then
// 00B2E2E8 = min(fMaxTime, 1.0)), the NiPointer store into 00B35C24, then 00B35C14 = min(00B35C14, 2048.0). Completion: 100% of the Bethesda-side stores.
pub fn create_default_havok_world(
    globals: &mut HavokGlobals,
    settings: &mut GameSettingCollection,
    inputs: &HavokSettingInputs,
) {
    // 004D5624 00B35C00 = 0.
    globals.b35c00 = 0;
    // 004D562E 004D5000 (substitution): 004D50BE 00B2E2D4 = iNumHavokThreads.
    globals.num_havok_threads_b2e2d4 = inputs.num_havok_threads as u32;
    // 00889B20 (under 004D5000) the world's ahkWorld takes a reference on the 00BA7904 collision filter (P09-RUN-001: reference count 2).
    if let Some(filter) = &mut globals.collision_filter {
        filter.reference_count_06 += 1;
    }
    // 004D52CB world +0x19, 004D52EE 00B2E2E8 = min(fMaxTime, 1.0).
    let world = BhkWorldSubstitute {
        byte_19: u8::from(inputs.b33a34 == 0),
    };
    globals.max_time_b2e2e8 = if inputs.max_time < 1.0 {
        inputs.max_time
    } else {
        1.0
    };
    // 004D5635..004D5673 the NiPointer store into 00B35C24.
    globals.world_b35c24 = Some(world);
    // 004D5679..004D568D fSeenDataUpdateRadius = 2048.0 when 2048.0 < it. 00B35C14 is the GMST's static Setting, so this reads and writes the live value (the C++ keeps a detached copy).
    if let Some(setting) = settings.setting_at_mut(SEEN_DATA_UPDATE_RADIUS_B35C14)
        && 2048.0 < f32::from_bits(setting.raw)
    {
        setting.raw = 2048.0f32.to_bits();
    }
}
