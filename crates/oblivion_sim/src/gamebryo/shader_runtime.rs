//! Phase 05 row 05-D-4a (shared core, part 1): the Gamebryo D3D shader runtime records the 27 BSShader slots
//! build. The pools keep the original's record identity and free-list order, with typed indexes standing in for the
//! original's pointers (host substitution): a record is never freed, as a slab never is.

use std::sync::Arc;

use bevy_ecs::prelude::Entity;
use oblivion_core::crt::{atol, stricmp};

use crate::bs::shader_programs::ShaderProgramObject;
use crate::bs::shader_tables::PREDEFINED_CONSTANT_NAMES;
use crate::gamebryo::renderer::RendererState;

// IDA: the slot tables 007802D0 (NiD3DRenderState_Constructor) leaves at
// 00B427B0..00B42833 (66 words): 00B427B0[state] maps a D3DSAMPLERSTATETYPE to
// one of five sampler slots (ADDRESSU 0, ADDRESSV 1, MAGFILTER 2, MINFILTER 3,
// MIPFILTER 4, else 0xFFFF); 00B427CC holds the five slot states as dwords;
// 00B427E0[state] maps a D3DTEXTURESTAGESTATETYPE to one of eight stage slots
// (COLOROP..ALPHAARG2 0..5, BUMPENVMAT00 6, TEXTURETRANSFORMFLAGS 7; the 0x42-
// byte memset leaves every other state 0xFFFF, TEXCOORDINDEX included);
// 00B42824 holds the eight slot states as words. Completion: 100% of the final image.
pub const SHADER_STATE_SLOT_WORDS: [u16; 66] = [
    0xFFFF, 0x0000, 0x0001, 0xFFFF, 0xFFFF, 0x0002, 0x0003, 0x0004, // 00B427B0
    0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0x0001, 0x0000, // 00B427C0
    0x0002, 0x0000, 0x0005, 0x0000, 0x0006, 0x0000, 0x0007, 0x0000, // 00B427D0
    0xFFFF, 0x0000, 0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, // 00B427E0
    0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, // 00B427F0
    0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, // 00B42800
    0x0007, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, // 00B42810
    0xFFFF, 0x0000, 0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, // 00B42820
    0x0007, 0x0018, // 00B42830
];

// IDA: N/A 00B427B0[state].
#[must_use]
pub fn sampler_state_slot(state: u32) -> u16 {
    SHADER_STATE_SLOT_WORDS
        .get(state as usize)
        .copied()
        .unwrap_or(0xFFFF)
}

// IDA: N/A 00B427E0[state].
#[must_use]
pub fn texture_stage_state_slot(state: u32) -> u16 {
    SHADER_STATE_SLOT_WORDS
        .get(state as usize + 24)
        .copied()
        .unwrap_or(0xFFFF)
}

/// IDA: N/A a record in a pool, by its place in slab order: a host substitution for the original's pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PassId(pub u32);
/// IDA: N/A see `PassId`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StageId(pub u32);
/// IDA: N/A see `PassId`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RenderGroupId(pub u32);
/// IDA: N/A see `PassId`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RenderEntryId(pub u32);
/// IDA: N/A see `PassId`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StageGroupId(pub u32);

// IDA: the 24-byte pool header (0075FFD0/007723C0/00772970/007734A0):
// +0 free items, +4 capacity, +8 free count, +0xC next batch (doubles after
// each growth), +0x10 initial batch, +0x14 slab list. Host mirror of the
// counters; capacity follows NiTArray_SetAllocatedSize doubling (0 -> 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderPoolCounters {
    pub capacity: u32,
    pub free_count: u32,
    pub next_batch: u32,
    pub initial_batch: u32,
    pub slabs: u32,
    pub records: u32,
}

impl ShaderPoolCounters {
    // IDA: N/A a helper, with no address of its own.
    const fn new(batch: u32) -> Self {
        Self {
            capacity: 0,
            free_count: 0,
            next_batch: batch,
            initial_batch: batch,
            slabs: 0,
            records: 0,
        }
    }

    // IDA: NiTArray_SetAllocatedSize doubling used by the pool growth and
    // NiTPrimitiveArray_AddUnique (0 -> 1, else x2) when count == capacity.
    fn reserve_slot(&mut self) {
        if self.free_count == self.capacity {
            self.capacity = if self.capacity != 0 {
                self.capacity * 2
            } else {
                1
            };
        }
    }
}

// IDA: 00772690 the 16-byte render-state entry {state, value, next, prev}.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderRenderStateEntry {
    pub state: u32,
    pub value: u32,
    pub next: Option<RenderEntryId>,
    pub previous: Option<RenderEntryId>,
}

// IDA: 007726A0 the 20-byte NiD3DRenderStateGroup: +0 byte (set to 1 by the
// first 00772DF0 acquire), +4/+8 list 0 (count, head: the "not saved" list,
// selector 0), +0xC/+0x10 list 1 (selector 1). Both lists insert at the head.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderRenderStateGroup {
    pub acquired: u8,
    pub unsaved_count: u32,
    pub unsaved_head: Option<RenderEntryId>,
    pub saved_count: u32,
    pub saved_head: Option<RenderEntryId>,
}

// IDA: 00772F30 the 184-byte NiD3DTextureStageGroup (all zero): +0 byte
// (acquired), +4 saved stage count, +0xC saved stage values[8], +0x2C saved
// flags[8], +0x34 unsaved stage count, +0x3C unsaved values[8], +0x5C unsaved
// flags[8], +0x64 saved sampler count, +0x6C saved sampler values[5], +0x80
// flags[5], +0x88 unsaved sampler count, +0x90 values[5], +0xA4 flags[5],
// +0xAC use-map count, +0xB0 use-map flags[5]; +8/+0x38/+0x68/+0x8C and the
// pad bytes are cleared too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureStageStateGroup {
    pub acquired: u8,
    pub saved_stage_count: u32,
    pub field_08: u32,
    pub saved_stage_values: [u32; 8],
    pub saved_stage_flags: [u8; 8],
    pub unsaved_stage_count: u32,
    pub field_38: u32,
    pub unsaved_stage_values: [u32; 8],
    pub unsaved_stage_flags: [u8; 8],
    pub saved_sampler_count: u32,
    pub field_68: u32,
    pub saved_sampler_values: [u32; 5],
    pub saved_sampler_flags: [u8; 5],
    pub unsaved_sampler_count: u32,
    pub field_8c: u32,
    pub unsaved_sampler_values: [u32; 5],
    pub unsaved_sampler_flags: [u8; 5],
    pub mapped_sampler_count: u32,
    pub mapped_sampler_flags: [u8; 5],
}

impl TextureStageStateGroup {
    // IDA: 00772FF0 SetStageState(state, value, save): slot = 00B427E0[state];
    // slots >= 8 are ignored. Moving an entry from the other list counts the
    // target list twice (the original adds 1 in the move and again after it).
    // Completion: 100%.
    pub fn set_stage_state(&mut self, state: u32, value: u32, save: bool) {
        let slot = texture_stage_state_slot(state) as usize;
        if slot >= 8 {
            return; // 00773000
        }
        if save {
            if self.saved_stage_flags[slot] != 0 {
                self.saved_stage_values[slot] = value; // 0077301C
                return;
            }
            if self.unsaved_stage_flags[slot] != 0 {
                // 00773030..00773041 the move counts the saved list once here...
                self.unsaved_stage_flags[slot] = 0;
                self.saved_stage_flags[slot] = 1;
                self.saved_stage_values[slot] = value;
                self.unsaved_stage_count = self.unsaved_stage_count.wrapping_sub(1);
                self.saved_stage_count = self.saved_stage_count.wrapping_add(1);
            }
            // ...and again at 00773044..0077304C.
            self.saved_stage_flags[slot] = 1;
            self.saved_stage_values[slot] = value;
            self.saved_stage_count = self.saved_stage_count.wrapping_add(1);
            return;
        }
        if self.unsaved_stage_flags[slot] != 0 {
            self.unsaved_stage_values[slot] = value; // 0077305A
            return;
        }
        if self.saved_stage_flags[slot] != 0 {
            self.saved_stage_flags[slot] = 0; // 0077306E..0077307E
            self.unsaved_stage_flags[slot] = 1;
            self.unsaved_stage_values[slot] = value;
            self.unsaved_stage_count = self.unsaved_stage_count.wrapping_add(1);
            self.saved_stage_count = self.saved_stage_count.wrapping_sub(1);
        }
        self.unsaved_stage_flags[slot] = 1; // 00773082..0077308A
        self.unsaved_stage_values[slot] = value;
        self.unsaved_stage_count = self.unsaved_stage_count.wrapping_add(1);
    }

    // IDA: 00773100 SetSamplerState(state, value, save, useMap): slot =
    // 00B427B0[state]; slots >= 5 are ignored; the use-map flag and count
    // follow the entry. Completion: 100%.
    pub fn set_sampler_state(&mut self, state: u32, value: u32, save: bool, use_map: bool) {
        let slot = sampler_state_slot(state) as usize;
        if slot >= 5 {
            return; // 00773110
        }
        if save {
            if self.saved_sampler_flags[slot] != 0 {
                self.saved_sampler_values[slot] = value; // 0077312F
                self.update_existing_map(slot, use_map);
                return;
            }
            if self.unsaved_sampler_flags[slot] != 0 {
                self.unsaved_sampler_flags[slot] = 0; // 0077313F..00773160
                self.unsaved_sampler_count = self.unsaved_sampler_count.wrapping_sub(1);
                if self.mapped_sampler_flags[slot] != 0 {
                    self.mapped_sampler_count = self.mapped_sampler_count.wrapping_sub(1);
                }
                self.mapped_sampler_flags[slot] = 0;
            }
            self.saved_sampler_flags[slot] = 1; // 00773168..0077317C
            self.saved_sampler_values[slot] = value;
            self.saved_sampler_count = self.saved_sampler_count.wrapping_add(1);
            self.finish_insert_map(slot, use_map);
            return;
        }
        if self.unsaved_sampler_flags[slot] != 0 {
            self.unsaved_sampler_values[slot] = value; // 00773192
            self.update_existing_map(slot, use_map);
            return;
        }
        if self.saved_sampler_flags[slot] != 0 {
            self.saved_sampler_flags[slot] = 0; // 007731D6..007731F4
            self.saved_sampler_count = self.saved_sampler_count.wrapping_sub(1);
            if self.mapped_sampler_flags[slot] != 0 {
                self.mapped_sampler_count = self.mapped_sampler_count.wrapping_sub(1);
            }
            self.mapped_sampler_flags[slot] = 0;
        }
        self.unsaved_sampler_flags[slot] = 1; // 007731FC..00773213
        self.unsaved_sampler_values[slot] = value;
        self.unsaved_sampler_count = self.unsaved_sampler_count.wrapping_add(1);
        self.finish_insert_map(slot, use_map);
    }

    // IDA: 00773199..007731C6.
    fn update_existing_map(&mut self, slot: usize, use_map: bool) {
        if use_map {
            if self.mapped_sampler_flags[slot] == 0 {
                self.mapped_sampler_count = self.mapped_sampler_count.wrapping_add(1);
            }
        } else if self.mapped_sampler_flags[slot] != 0 {
            self.mapped_sampler_count = self.mapped_sampler_count.wrapping_sub(1);
        }
        self.mapped_sampler_flags[slot] = u8::from(use_map);
    }

    // IDA: 00773219..00773227.
    fn finish_insert_map(&mut self, slot: usize, use_map: bool) {
        if use_map {
            self.mapped_sampler_count = self.mapped_sampler_count.wrapping_add(1);
        }
        self.mapped_sampler_flags[slot] = u8::from(use_map);
    }

    // IDA: 00773240 / 00773270 clear the stage / sampler halves. Completion: 100%.
    pub fn clear_stage_states(&mut self) {
        // 00773240: both flag arrays, then +0x34 and +4.
        self.saved_stage_flags = [0; 8];
        self.unsaved_stage_flags = [0; 8];
        self.unsaved_stage_count = 0;
        self.saved_stage_count = 0;
    }

    // IDA: 00773270.
    pub fn clear_sampler_states(&mut self) {
        // 00773270: the three flag arrays, then +0x88, +0x64, +0xAC.
        self.saved_sampler_flags = [0; 5];
        self.unsaved_sampler_flags = [0; 5];
        self.mapped_sampler_flags = [0; 5];
        self.unsaved_sampler_count = 0;
        self.saved_sampler_count = 0;
        self.mapped_sampler_count = 0;
    }
}

// IDA: 007720D0 the 96-byte NiD3DTextureStage: +0 stage index, +4 texture
// (NiPointer, null here), +8, +0xC state group (acquired from 00B42838 when
// the record is constructed), +0x10, +0x14 global-constant entry, +0x18..+0x57
// the texture transform (D3DMATRIX, not initialized), +0x58 word, +0x5A byte,
// +0x5B byte (set by the pool acquire), +0x5C reference count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderTextureStage {
    pub stage: u32,
    /// +0x04. The NiSourceTexture entity from `BsShaderContext::effect_texture`.
    pub texture: Option<Entity>,
    /// Host record: the 0076C910 source global.
    pub texture_global: u32,
    pub field_08: u32,
    pub states: Option<StageGroupId>,
    pub field_10: u32,
    /// +0x14. Never set by a ported class (0077CB50 has no factory entry here).
    pub global_constant: Option<u32>,
    pub field_58: u16,
    pub field_5a: u8,
    pub acquired: u8,
    pub references: u32,
}

// IDA: 007606A0 ShaderRuntimePool100_AcquireInto: grows by next_batch
// (00760600: 007601E0 per record) when empty, then takes free[0], moves the
// last item into slot 0, +0x60 += 1 and +0x5D = 1. Completion: 100%.
#[derive(Clone, Debug, Default)]
pub struct ShaderPass {
    pub field_04: u8,
    pub field_10: u32,
    pub stage_end: u32,
    pub stage_count: u32,
    pub textured_stage_count: u32,
    pub stages: Vec<Option<StageId>>,
    pub stage_array_capacity: u16,
    pub stage_array_size: u16,
    pub stage_array_count: u16,
    pub stage_array_grow: u16,
    pub render_states: Option<RenderGroupId>,
    /// +0x34. No ported class sets it.
    pub vertex_constant_map: Option<u32>,
    pub pixel_shader: Option<Arc<ShaderProgramObject>>,
    /// +0x48. No ported class sets it.
    pub pixel_constant_map: Option<u32>,
    pub vertex_shader: Option<Arc<ShaderProgramObject>>,
    pub field_5c: u8,
    pub acquired: u8,
    pub references: u32,
}

// IDA: the five pools of 0075FFD0/007723C0/00772970/007734A0 (initial and
// growth batch 8, the 16-byte pool 16). Host-owned slabs; pointers stay valid
// until the pools are destroyed.
#[derive(Clone, Debug)]
pub struct ShaderRuntimePools {
    pub pass_counters: ShaderPoolCounters,         // 00B42044
    pub stage_counters: ShaderPoolCounters,        // 00B4275C
    pub render_group_counters: ShaderPoolCounters, // 00B427A8
    pub render_entry_counters: ShaderPoolCounters, // 00B427AC
    pub stage_group_counters: ShaderPoolCounters,  // 00B42838
    pub max_texture_blend_stages: u16,             // 00B28CB0
    pub max_sampler_stages: u32,                   // 00B28CB8

    /// Records in slab order. `*_slab_starts` holds each slab's first index; slabs are in growth order here,
    /// where the original links each new one at the head of +0x14.
    pub passes: Vec<ShaderPass>,
    pub pass_slab_starts: Vec<u32>,
    pub stages: Vec<ShaderTextureStage>,
    pub stage_slab_starts: Vec<u32>,
    pub render_groups: Vec<ShaderRenderStateGroup>,
    pub render_entries: Vec<ShaderRenderStateEntry>,
    pub stage_groups: Vec<TextureStageStateGroup>,
    pub free_passes: Vec<PassId>,
    pub free_stages: Vec<StageId>,
    pub free_render_groups: Vec<RenderGroupId>,
    pub free_render_entries: Vec<RenderEntryId>,
    pub free_stage_groups: Vec<StageGroupId>,
}

impl Default for ShaderRuntimePools {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            pass_counters: ShaderPoolCounters::new(8),
            stage_counters: ShaderPoolCounters::new(8),
            render_group_counters: ShaderPoolCounters::new(8),
            render_entry_counters: ShaderPoolCounters::new(16),
            stage_group_counters: ShaderPoolCounters::new(8),
            max_texture_blend_stages: 0,
            max_sampler_stages: 0,
            passes: Vec::new(),
            pass_slab_starts: Vec::new(),
            stages: Vec::new(),
            stage_slab_starts: Vec::new(),
            render_groups: Vec::new(),
            render_entries: Vec::new(),
            stage_groups: Vec::new(),
            free_passes: Vec::new(),
            free_stages: Vec::new(),
            free_render_groups: Vec::new(),
            free_render_entries: Vec::new(),
            free_stage_groups: Vec::new(),
        }
    }
}

// IDA: the acquire idiom (`if (!free) Grow(next); next *= 2; item = free[0]; free[0] = free[--free_count]`).
fn take_free<T: Copy>(counters: &mut ShaderPoolCounters, free: &mut Vec<T>) -> T {
    let item = free[0];
    free[0] = free[counters.free_count as usize - 1];
    free.pop();
    counters.free_count -= 1;
    item
}

// IDA: NiTPrimitiveArray_AddUnique(pool, &item) (0073A5E0).
fn return_free<T: Copy + PartialEq>(counters: &mut ShaderPoolCounters, free: &mut Vec<T>, item: T) {
    if free.contains(&item) {
        return;
    }
    counters.reserve_slot();
    free.push(item);
    counters.free_count += 1;
}

// IDA: one record of a Grow loop: its pointer appended to the free list.
fn grown_record<T>(counters: &mut ShaderPoolCounters, free: &mut Vec<T>, item: T) {
    counters.reserve_slot();
    free.push(item);
    counters.free_count += 1;
    counters.records += 1;
}

impl ShaderRuntimePools {
    /// IDA: N/A
    #[must_use]
    pub fn pass(&self, id: PassId) -> &ShaderPass {
        &self.passes[id.0 as usize]
    }
    /// IDA: N/A
    pub fn pass_mut(&mut self, id: PassId) -> &mut ShaderPass {
        &mut self.passes[id.0 as usize]
    }
    /// IDA: N/A
    #[must_use]
    pub fn stage(&self, id: StageId) -> &ShaderTextureStage {
        &self.stages[id.0 as usize]
    }
    /// IDA: N/A
    pub fn stage_mut(&mut self, id: StageId) -> &mut ShaderTextureStage {
        &mut self.stages[id.0 as usize]
    }
    /// IDA: N/A a stage's +0xC state group. Every constructed stage has one (007720D0).
    #[must_use]
    pub fn stage_states(&self, id: StageId) -> &TextureStageStateGroup {
        let group = self.stage(id).states.expect("007720D0 acquired a group");
        &self.stage_groups[group.0 as usize]
    }
    /// IDA: N/A see `stage_states`.
    pub fn stage_states_mut(&mut self, id: StageId) -> &mut TextureStageStateGroup {
        let group = self.stage(id).states.expect("007720D0 acquired a group");
        &mut self.stage_groups[group.0 as usize]
    }
    /// IDA: N/A a group's list from the head (the original lookup order).
    #[must_use]
    pub fn render_state_entries(
        &self,
        group: RenderGroupId,
        saved: bool,
    ) -> Vec<ShaderRenderStateEntry> {
        let group = self.render_groups[group.0 as usize];
        let mut result = Vec::new();
        let mut entry = if saved {
            group.saved_head
        } else {
            group.unsaved_head
        };
        while let Some(id) = entry {
            let record = self.render_entries[id.0 as usize];
            result.push(record);
            entry = record.next;
        }
        result
    }

    // IDA: 007606A0 ShaderRuntimePool100_AcquireInto: grows by next_batch
    // (00760600: 007601E0 per record) when empty, then takes free[0], moves the
    // last item into slot 0, +0x60 += 1 and +0x5D = 1. Completion: 100%.
    pub fn acquire_pass(&mut self) -> PassId {
        if self.pass_counters.free_count == 0 {
            let count = self.pass_counters.next_batch;
            self.pass_slab_starts.push(self.passes.len() as u32);
            for _ in 0..count {
                // 007601E0; 007601FA..00760200 0075FE30(g_niD3DMaxTextureBlendStages, 1).
                let max = self.max_texture_blend_stages;
                self.passes.push(ShaderPass {
                    stage_array_capacity: max,
                    stage_array_grow: 1,
                    stages: vec![None; max as usize],
                    ..ShaderPass::default()
                });
                let id = PassId(self.passes.len() as u32 - 1);
                grown_record(&mut self.pass_counters, &mut self.free_passes, id);
            }
            self.pass_counters.slabs += 1;
            self.pass_counters.next_batch *= 2;
        }
        let id = take_free(&mut self.pass_counters, &mut self.free_passes);
        let pass = self.pass_mut(id);
        pass.references += 1; // 007606E7
        if pass.acquired == 0 {
            pass.acquired = 1; // 007606F0
        }
        id
    }

    // IDA: 00772630 ShaderRuntimePool96_AcquireInto (00772430: 007720D0 per
    // record, each acquiring its 184-byte group): +0x5C += 1, +0x5B = 1. Completion: 100%.
    pub fn acquire_texture_stage(&mut self) -> StageId {
        if self.stage_counters.free_count == 0 {
            let count = self.stage_counters.next_batch;
            self.stage_slab_starts.push(self.stages.len() as u32);
            for _ in 0..count {
                // 007720D0: the fields, then the 184-byte group.
                let states = self.acquire_stage_state_group();
                self.stages.push(ShaderTextureStage {
                    states: Some(states),
                    ..ShaderTextureStage::default()
                });
                let id = StageId(self.stages.len() as u32 - 1);
                grown_record(&mut self.stage_counters, &mut self.free_stages, id);
            }
            self.stage_counters.slabs += 1;
            self.stage_counters.next_batch *= 2;
        }
        let id = take_free(&mut self.stage_counters, &mut self.free_stages);
        let stage = self.stage_mut(id);
        stage.references += 1; // 00772677
        stage.acquired = 1; // 0077267A
        id
    }

    // IDA: 00772DF0 ShaderRuntimePool20_AcquireRecord: byte +0 = 1 when 0. Completion: 100%.
    pub fn acquire_render_state_group(&mut self) -> RenderGroupId {
        if self.render_group_counters.free_count == 0 {
            for _ in 0..self.render_group_counters.next_batch {
                self.render_groups.push(ShaderRenderStateGroup::default());
                let id = RenderGroupId(self.render_groups.len() as u32 - 1);
                grown_record(
                    &mut self.render_group_counters,
                    &mut self.free_render_groups,
                    id,
                );
            }
            self.render_group_counters.slabs += 1;
            self.render_group_counters.next_batch *= 2;
        }
        let id = take_free(
            &mut self.render_group_counters,
            &mut self.free_render_groups,
        );
        let group = &mut self.render_groups[id.0 as usize];
        if group.acquired == 0 {
            group.acquired = 1;
        }
        id
    }

    // IDA: 007735E0 ShaderRuntimePool184_AcquireRecord. Completion: 100%.
    pub fn acquire_stage_state_group(&mut self) -> StageGroupId {
        if self.stage_group_counters.free_count == 0 {
            for _ in 0..self.stage_group_counters.next_batch {
                self.stage_groups.push(TextureStageStateGroup::default());
                let id = StageGroupId(self.stage_groups.len() as u32 - 1);
                grown_record(
                    &mut self.stage_group_counters,
                    &mut self.free_stage_groups,
                    id,
                );
            }
            self.stage_group_counters.slabs += 1;
            self.stage_group_counters.next_batch *= 2;
        }
        let id = take_free(&mut self.stage_group_counters, &mut self.free_stage_groups);
        let group = &mut self.stage_groups[id.0 as usize];
        if group.acquired == 0 {
            group.acquired = 1;
        }
        id
    }

    // IDA: the 00772CD0 pool path for a new entry (00772A80 growth). Completion: 100%.
    pub fn acquire_render_state_entry(&mut self) -> RenderEntryId {
        if self.render_entry_counters.free_count == 0 {
            for _ in 0..self.render_entry_counters.next_batch {
                self.render_entries.push(ShaderRenderStateEntry::default());
                let id = RenderEntryId(self.render_entries.len() as u32 - 1);
                grown_record(
                    &mut self.render_entry_counters,
                    &mut self.free_render_entries,
                    id,
                );
            }
            self.render_entry_counters.slabs += 1;
            self.render_entry_counters.next_batch *= 2;
        }
        take_free(
            &mut self.render_entry_counters,
            &mut self.free_render_entries,
        )
    }

    // IDA: 00772560 texture-stage release at count 0: an acquired record
    // drops its texture, clears both halves of its group (00773240/
    // 00773270), releases the global constant (0077CB50), zeroes +0, +8,
    // +0x10, +0x14, +0x5A and returns to the pool. Completion: 100% for acquired records.
    pub fn release_texture_stage(&mut self, id: StageId) {
        // 007725BD..00772623.
        let states = self.stage(id).states;
        let stage = self.stage_mut(id);
        stage.texture = None;
        if let Some(states) = states {
            let group = &mut self.stage_groups[states.0 as usize];
            group.clear_stage_states();
            group.clear_sampler_states();
        }
        let stage = self.stage_mut(id);
        stage.global_constant = None; // 0077CB50 (no factory entry here)
        stage.stage = 0;
        stage.field_08 = 0;
        stage.field_10 = 0;
        stage.field_5a = 0;
        return_free(&mut self.stage_counters, &mut self.free_stages, id);
    }

    // IDA: 00772E30 render-state group return (entries of both lists first).
    pub fn release_render_state_group(&mut self, id: RenderGroupId) {
        // 00772E40..00772EB2.
        let mut entry = self.render_groups[id.0 as usize].unsaved_head;
        while let Some(current) = entry {
            let next = self.render_entries[current.0 as usize].next;
            return_free(
                &mut self.render_entry_counters,
                &mut self.free_render_entries,
                current,
            );
            entry = next;
        }
        let group = &mut self.render_groups[id.0 as usize];
        group.unsaved_head = None;
        group.unsaved_count = 0;
        let mut entry = group.saved_head;
        while let Some(current) = entry {
            let next = self.render_entries[current.0 as usize].next;
            return_free(
                &mut self.render_entry_counters,
                &mut self.free_render_entries,
                current,
            );
            entry = next;
        }
        let group = &mut self.render_groups[id.0 as usize];
        group.saved_head = None;
        group.saved_count = 0;
        return_free(
            &mut self.render_group_counters,
            &mut self.free_render_groups,
            id,
        );
    }

    // IDA: 007604D0 pass release at count 0 (acquired records): releases
    // +0x34/+0x44/+0x48/+0x58, returns the render-state group (00772E30) and
    // its entries, frees the program strings, clears the stage array
    // (0075FF80), zeroes +0x14/+0x18/+0x1C and +0x5C and returns to the pool.
    // Completion: 100% for acquired records.
    pub fn release_pass(&mut self, id: PassId) {
        // 007604F0..007605EE.
        let pass = self.pass_mut(id);
        pass.vertex_constant_map = None;
        pass.pixel_shader = None;
        pass.pixel_constant_map = None;
        pass.vertex_shader = None;
        if let Some(group) = pass.render_states {
            self.release_render_state_group(group);
        }
        self.pass_mut(id).render_states = None;
        // 0075FF80: release stages [0, size), then size and count to zero.
        let size = self.pass(id).stage_array_size as usize;
        for index in 0..size.min(self.pass(id).stages.len()) {
            let stage = self.pass(id).stages[index];
            release_stage_reference(self, stage);
            self.pass_mut(id).stages[index] = None;
        }
        let pass = self.pass_mut(id);
        pass.stage_array_size = 0;
        pass.stage_array_count = 0;
        pass.stage_end = 0;
        pass.stage_count = 0;
        pass.textured_stage_count = 0;
        pass.field_5c = 0;
        return_free(&mut self.pass_counters, &mut self.free_passes, id);
    }
}

// IDA: NiPointer<NiD3DPass> count helper (+0x60): increment. Completion: 100%.
pub fn add_pass_reference(pools: &mut ShaderRuntimePools, pass: Option<PassId>) {
    if let Some(pass) = pass {
        pools.pass_mut(pass).references += 1;
    }
}

// IDA: NiPointer<NiD3DPass> count helper (+0x60): decrement with the release at zero. Completion: 100%.
pub fn release_pass_reference(pools: &mut ShaderRuntimePools, pass: Option<PassId>) {
    if let Some(pass) = pass {
        let record = pools.pass_mut(pass);
        record.references -= 1;
        if record.references == 0 {
            pools.release_pass(pass);
        }
    }
}

// IDA: NiPointer<NiD3DTextureStage> count helper (+0x5C): increment. Completion: 100%.
pub fn add_stage_reference(pools: &mut ShaderRuntimePools, stage: Option<StageId>) {
    if let Some(stage) = stage {
        pools.stage_mut(stage).references += 1;
    }
}

// IDA: NiPointer<NiD3DTextureStage> count helper (+0x5C): decrement with the release at zero. Completion: 100%.
pub fn release_stage_reference(pools: &mut ShaderRuntimePools, stage: Option<StageId>) {
    if let Some(stage) = stage {
        let record = pools.stage_mut(stage);
        record.references -= 1;
        if record.references == 0 {
            pools.release_texture_stage(stage);
        }
    }
}

// IDA: 0075FED0 NiTArray<NiPointer<NiD3DTextureStage>>::SetAt(index, stage):
// size +0x2A and element count +0xC; the host vector grows past the
// capacity where the original would write past its allocation.
fn set_stage_slot(pools: &mut ShaderRuntimePools, id: PassId, index: u32, stage: Option<StageId>) {
    let i = index as usize;
    let pass = pools.pass_mut(id);
    if pass.stages.len() <= i {
        pass.stages.resize(i + 1, None);
    }
    if index < u32::from(pass.stage_array_size) {
        if stage.is_none() {
            if pass.stages[i].is_some() {
                pass.stage_array_count = pass.stage_array_count.wrapping_sub(1);
            }
        } else if pass.stages[i].is_none() {
            pass.stage_array_count = pass.stage_array_count.wrapping_add(1);
        }
    } else {
        pass.stage_array_size = (index + 1) as u16;
        if stage.is_some() {
            pass.stage_array_count = pass.stage_array_count.wrapping_add(1);
        }
    }
    let slot = pass.stages[i];
    if slot != stage {
        release_stage_reference(pools, slot);
        pools.pass_mut(id).stages[i] = stage;
        add_stage_reference(pools, stage);
    }
}

// IDA: `entry` from a list: 00772750 / 00772790.
fn unlink(
    entries: &mut [ShaderRenderStateEntry],
    id: RenderEntryId,
    head: &mut Option<RenderEntryId>,
    count: &mut u32,
) {
    let entry = entries[id.0 as usize];
    if let Some(next) = entry.next {
        entries[next.0 as usize].previous = entry.previous;
    }
    if let Some(previous) = entry.previous {
        entries[previous.0 as usize].next = entry.next;
    }
    if *head == Some(id) {
        *head = entry.next;
    }
    *count = count.wrapping_sub(1);
    entries[id.0 as usize].next = None;
    entries[id.0 as usize].previous = None;
}

// IDA: the head insertion both lists use (00772D62..00772DE3).
fn push_head(
    entries: &mut [ShaderRenderStateEntry],
    id: RenderEntryId,
    head: &mut Option<RenderEntryId>,
    count: &mut u32,
) {
    if let Some(old) = *head {
        entries[old.0 as usize].previous = Some(id);
        entries[id.0 as usize].next = Some(old);
    }
    entries[id.0 as usize].previous = None;
    *count = count.wrapping_add(1);
    *head = Some(id);
}

// IDA: 00772CD0 ShaderRuntimePool16_InsertOrUpdateLinkedEntry(group, state,
// value, selector): an entry found in list 0 then list 1 takes the value and,
// when found in the other list, moves to the head of the requested one;
// otherwise a pool entry {state, value} is inserted at the head of list
// `selector`. Completion: 100%.
pub fn set_render_state(
    pools: &mut ShaderRuntimePools,
    group: RenderGroupId,
    state: u32,
    value: u32,
    selector: bool,
) {
    // 007727D0: list 0, then list 1.
    let mut found = None;
    let mut found_selector = false;
    let record = pools.render_groups[group.0 as usize];
    let mut entry = record.unsaved_head;
    while let Some(id) = entry {
        if pools.render_entries[id.0 as usize].state == state {
            found = Some(id);
            break;
        }
        entry = pools.render_entries[id.0 as usize].next;
    }
    if found.is_none() {
        let mut entry = record.saved_head;
        while let Some(id) = entry {
            if pools.render_entries[id.0 as usize].state == state {
                found = Some(id);
                found_selector = true;
                break;
            }
            entry = pools.render_entries[id.0 as usize].next;
        }
    }
    if let Some(id) = found {
        pools.render_entries[id.0 as usize].value = value; // 00772D00
        if found_selector == selector {
            return;
        }
        let group = &mut pools.render_groups[group.0 as usize];
        let entries = &mut pools.render_entries;
        if found_selector {
            unlink(entries, id, &mut group.saved_head, &mut group.saved_count);
            push_head(
                entries,
                id,
                &mut group.unsaved_head,
                &mut group.unsaved_count,
            );
        } else {
            unlink(
                entries,
                id,
                &mut group.unsaved_head,
                &mut group.unsaved_count,
            );
            push_head(entries, id, &mut group.saved_head, &mut group.saved_count);
        }
        return;
    }
    // 00772D62..00772DE3.
    let id = pools.acquire_render_state_entry();
    pools.render_entries[id.0 as usize] = ShaderRenderStateEntry {
        state,
        value,
        next: None,
        previous: None,
    };
    let group = &mut pools.render_groups[group.0 as usize];
    if selector {
        push_head(
            &mut pools.render_entries,
            id,
            &mut group.saved_head,
            &mut group.saved_count,
        );
    } else {
        push_head(
            &mut pools.render_entries,
            id,
            &mut group.unsaved_head,
            &mut group.unsaved_count,
        );
    }
}

// IDA: the callers' `if (!pass->+0x30) pass->+0x30 = 00772DF0()` idiom,
// then 00772CD0(group, state, value, 0). Completion: 100%.
pub fn set_pass_render_state(
    pools: &mut ShaderRuntimePools,
    pass: PassId,
    state: u32,
    value: u32,
    selector: bool,
) {
    if pools.pass(pass).render_states.is_none() {
        let group = pools.acquire_render_state_group();
        pools.pass_mut(pass).render_states = Some(group);
    }
    let group = pools.pass(pass).render_states.expect("set above");
    set_render_state(pools, group, state, value, selector);
}

// IDA: 00760010 NiD3DPass::SetStage(index, stage): an index at or past
// g_niD3DMaxTextureBlendStages but below g_niD3DMaxSamplerStages is stored
// through 0075FED0 first; at or past the sampler count returns 1; then the old
// stage's counts drop (+0x18, and +0x1C when it had a texture), the new stage
// is stored (0075FED0 NiTArray SetAt growing the size to index + 1), takes
// index (+0), extends +0x14 to index + 1 and counts +0x18/+0x1C; a null stage
// shrinks +0x14 by one when it was index + 1. Returns 0. Completion: 100%.
pub fn set_pass_stage(
    pools: &mut ShaderRuntimePools,
    pass: PassId,
    index: u32,
    stage: Option<StageId>,
) -> u32 {
    // 00760015..00760061: past the blend stages the stage is stored first
    // (with a temporary reference) and the generic path then runs on it.
    if index >= u32::from(pools.max_texture_blend_stages) {
        if index >= pools.max_sampler_stages {
            return 1;
        }
        add_stage_reference(pools, stage);
        set_stage_slot(pools, pass, index, stage);
        release_stage_reference(pools, stage);
    }
    // 00760063..007600B0 the stage currently in the slot.
    let old = pools
        .pass(pass)
        .stages
        .get(index as usize)
        .copied()
        .flatten();
    if let Some(old) = old {
        let textured = pools.stage(old).texture.is_some();
        let record = pools.pass_mut(pass);
        record.stage_count = record.stage_count.wrapping_sub(1);
        if textured {
            record.textured_stage_count = record.textured_stage_count.wrapping_sub(1);
        }
    }
    // 007600B2..007600DE.
    add_stage_reference(pools, stage);
    set_stage_slot(pools, pass, index, stage);
    if let Some(id) = stage {
        release_stage_reference(pools, stage);
        pools.stage_mut(id).stage = index; // 007600E1
        let textured = pools.stage(id).texture.is_some();
        let record = pools.pass_mut(pass);
        if record.stage_end == index {
            record.stage_end = index + 1;
        }
        record.stage_count = record.stage_count.wrapping_add(1);
        if textured {
            record.textured_stage_count = record.textured_stage_count.wrapping_add(1);
        }
        return 0;
    }
    // 00760134..00760162: the reloaded slot is the null just stored, so the
    // end shrinks by exactly one.
    let record = pools.pass_mut(pass);
    if record.stage_end == index + 1 && record.stage_end != 0 {
        record.stage_end -= 1;
    }
    0
}

// IDA: 00771640 SetFilterMode(stage, mode): MAG/MIN/MIPFILTER from the
// 00B420EC/00B420E8/00B420F0 triple `mode` (RendererState::texture_stage_
// globals). Completion: 100%.
pub fn set_texture_stage_filter_mode(
    renderer: &RendererState,
    pools: &mut ShaderRuntimePools,
    stage: StageId,
    mode: u32,
) {
    // 00771640: 00B420EC/00B420E8/00B420F0 + 12 * mode.
    let table = &renderer.texture_stage_globals;
    let at = |address: u32| {
        let index = ((address - 0x00B4_20E8) / 4 + mode * 3) as usize;
        table.get(index).copied().unwrap_or(0)
    };
    let states = pools.stage_states_mut(stage);
    states.set_sampler_state(5, at(0x00B4_20EC), false, false);
    states.set_sampler_state(6, at(0x00B4_20E8), false, false);
    states.set_sampler_state(7, at(0x00B4_20F0), false, false);
}

// IDA: 00801110 BSShader texture-stage defaults(stage, index, address, filter):
// +0 = index; stage states TEXCOORDINDEX (no slot), COLOROP 1, COLORARG1 2,
// ALPHAOP 1, ALPHAARG1 2 (unsaved); +0x5A = 0; TEXTURETRANSFORMFLAGS 0;
// sampler ADDRESSU/V = address, MAG/MIN/MIPFILTER = filter (unsaved, no
// map); then 00771640(stage, 2). Completion: 100%.
pub fn set_texture_stage_defaults(
    renderer: &RendererState,
    pools: &mut ShaderRuntimePools,
    stage: StageId,
    index: u32,
    address: u32,
    filter: u32,
) {
    // 00801119..0080116E.
    pools.stage_mut(stage).stage = index;
    let states = pools.stage_states_mut(stage);
    states.set_stage_state(11, index, false); // D3DTSS_TEXCOORDINDEX (no slot)
    states.set_stage_state(1, 1, false); // COLOROP DISABLE
    states.set_stage_state(2, 2, false); // COLORARG1 TEXTURE
    states.set_stage_state(4, 1, false); // ALPHAOP DISABLE
    states.set_stage_state(5, 2, false); // ALPHAARG1 TEXTURE
    pools.stage_mut(stage).field_5a = 0;
    let states = pools.stage_states_mut(stage);
    states.set_stage_state(24, 0, false); // TEXTURETRANSFORMFLAGS
    // 00801181..008011C1.
    states.set_sampler_state(1, address, false, false);
    states.set_sampler_state(2, address, false, false);
    states.set_sampler_state(5, filter, false, false);
    states.set_sampler_state(6, filter, false, false);
    states.set_sampler_state(7, filter, false, false);
    set_texture_stage_filter_mode(renderer, pools, stage, 2);
}

// IDA: 0076FB00 the 28-byte declaration entry: +0/+4 = 0, +8 parameter
// (NiShaderDeclaration::ShaderParameter, -1 = unused), +0xC parameter type
// (17 = none), +0x10, +0x14 D3DDECLUSAGE, +0x18 usage index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderDeclarationEntry {
    pub field_00: u32,
    pub field_04: u32,
    pub parameter: u32,
    pub parameter_type: u32,
    pub field_10: u32,
    pub usage: u32,
    pub usage_index: u32,
}

impl Default for ShaderDeclarationEntry {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            field_00: 0,
            field_04: 0,
            parameter: 0xFFFF_FFFF,
            parameter_type: 17,
            field_10: 0,
            usage: 0,
            usage_index: 0,
        }
    }
}

// IDA: NiDX9ShaderDeclaration (0x38, vtable A89C74): NiObject base,
// 0076E270 +8..+0x24 = 0, +0x28 byte 0; 0076E320 Initialize(renderer,
// entries, streams): +8 renderer, +0xC renderer +0x8B0, +0x10 device (AddRef),
// +0x1C entry count, +0x20 stream count, +0x24 16-byte stream records
// (0076E260) each with entry count x 28-byte entries (0076FB00) at +8; the
// subclass adds +0x2C = 0, +0x30 cached D3D declaration = 0, +0x34 byte 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderDeclaration {
    pub original_vtable: u32,
    pub entry_count: u32,
    pub stream_count: u32,
    pub streams: Vec<Vec<ShaderDeclarationEntry>>,
    pub dirty: u8, // +0x28
    pub field_2c: u32,
    pub cached_declaration: u32, // +0x30 (released by 0076FC40)
    pub field_34: u8,
}

// IDA: 00771090 NiDX9ShaderDeclaration_Create(renderer, entries, streams):
// null when streams > renderer +0x6DC; FormHeap(0x38), 0076E270, the
// subclass fields, 0076E320. Completion: 100%.
#[must_use]
pub fn create_shader_declaration(
    renderer: &RendererState,
    entry_count: u32,
    stream_count: u32,
) -> Option<ShaderDeclaration> {
    // 00771090..007710A6.
    if stream_count > renderer.max_streams_6dc {
        return None;
    }
    // 007710B8..007710E0 then 0076E320.
    Some(ShaderDeclaration {
        original_vtable: 0x00A8_9C74,
        entry_count,
        stream_count,
        streams: vec![
            vec![ShaderDeclarationEntry::default(); entry_count as usize];
            stream_count as usize
        ],
        dirty: 0,
        field_2c: 0,
        cached_declaration: 0,
        field_34: 0,
    })
}

// IDA: 0076FC40 vt+0x4C SetEntry(stream, index, register, parameter, type,
// usage, usageIndex, field10): false when index >= +0x1C or stream >= +0x20;
// the register argument is not stored; an entry that differs takes the
// values (parameter 1 with a type other than 16 becomes type 2), sets +0x28
// and releases the cached declaration +0x30; an equal entry changes nothing.
// True otherwise. Completion: 100%.
#[allow(clippy::too_many_arguments)] // 0076FC40 vt+0x4C argument order.
pub fn set_declaration_entry(
    declaration: &mut ShaderDeclaration,
    stream: u32,
    index: u32,
    _register_index: u32, // not stored by 0076FC40
    parameter: u32,
    parameter_type: u32,
    usage: u32,
    usage_index: u32,
    field_10: u32,
) -> bool {
    if index >= declaration.entry_count || stream >= declaration.stream_count {
        return false;
    }
    let entry = &mut declaration.streams[stream as usize][index as usize];
    if entry.parameter == parameter
        && entry.parameter_type == parameter_type
        && entry.usage == usage
        && entry.usage_index == usage_index
        && entry.field_10 == field_10
    {
        return true;
    }
    entry.parameter = parameter;
    entry.parameter_type = parameter_type;
    entry.usage = usage;
    entry.usage_index = usage_index;
    entry.field_10 = field_10;
    if parameter == 1 && parameter_type != 16 {
        entry.parameter_type = 2;
    }
    declaration.dirty = 1;
    declaration.cached_declaration = 0;
    true
}

// IDA: 0076FB30 vt+0x50 SetEntry(index, register, parameter, type, stream):
// parameter 0..14 maps to (usage, usage index) 0 (0,0) 1 (1,0) 2 (2,0)
// 3 (3,0) 4 (10,0) 5..12 (5, 0..7) 13 (6,0) 14 (7,0), then vt+0x4C(stream,
// index, register, parameter, type, usage, usage index, 0); other parameters
// return false. Completion: 100%.
pub fn set_declaration_entry_by_parameter(
    declaration: &mut ShaderDeclaration,
    index: u32,
    register_index: u32,
    parameter: u32,
    parameter_type: u32,
    stream: u32,
) -> bool {
    let (usage, usage_index) = match parameter {
        0..=3 => (parameter, 0),
        4 => (10, 0),
        5..=12 => (5, parameter - 5),
        13 => (6, 0),
        14 => (7, 0),
        _ => return false,
    };
    set_declaration_entry(
        declaration,
        stream,
        index,
        register_index,
        parameter,
        parameter_type,
        usage,
        usage_index,
        0,
    )
}

// IDA: 009A84B0 NiD3DShaderConstantMapEntry (0x38, vtable AB348C): byte +8 = 1,
// +0xC key (009A8520 copy), +0x10 the predefined mapping id (not initialized
// by 009A84B0; the host uses 0), +0x14 flags, +0x18 Gamebryo's uiExtra, +0x1C
// register, +0x20 register count, +0x24 variable name
// (009A85C0 copy), +0x28 data size, +0x2C stride, +0x30 data, byte +0x34
// owns the data. The host records the data as the original reference text
// ("this+0x100" or a global address) because host objects do not live at
// the original addresses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderConstantMapEntry {
    pub original_vtable: u32,
    pub field_08: u8,
    pub key: String,
    pub predefined_id: u32,
    pub flags: u32,
    pub extra: u32,
    pub register_index: u32,
    pub register_count: u32,
    pub variable: String,
    pub data_size: u32,
    pub data_stride: u32,
    pub data_reference: String,
    pub owns_data: u8,
}

impl Default for ShaderConstantMapEntry {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: 0x00AB_348C,
            field_08: 1,
            key: String::new(),
            predefined_id: 0,
            flags: 0,
            extra: 0,
            register_index: 0,
            register_count: 0,
            variable: String::new(),
            data_size: 0,
            data_stride: 0,
            data_reference: String::new(),
            owns_data: 0,
        }
    }
}

// IDA: NiD3DShaderConstantMap (0x34, 009A46A0): +8 = 2, +0xC NiTArray<
// NiPointer<NiD3DShaderConstantMapEntry>> (capacity 0, grow 1), byte +0x1C =
// 1, +0x20 = 0, +0x24 last result, +0x28 device, +0x2C renderer, +0x30
// render state (009A8BD0); the subclasses NiD3DSCM_Pixel (009A4750, vtable
// AB297C, NiRTTI 00BAA8D8) and NiD3DSCM_Vertex (009A8490, vtable AB2BAC,
// NiRTTI 00BAA920).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderConstantMap {
    pub original_vtable: u32,
    pub pixel_rtti: bool, // RTTI 00BAA8D8 (else 00BAA920)
    pub field_08: u32,
    pub entries: Vec<Option<Arc<ShaderConstantMapEntry>>>, // +0x10 data
    pub entry_capacity: u16,                               // +0x14
    pub entry_size: u16,                                   // +0x16
    pub entry_count: u16,                                  // +0x18
    pub entry_grow: u16,                                   // +0x1A
    pub field_1c: u8,
    pub field_20: u32,
    pub last_result: u32, // +0x24
}

/// IDA: N/A a constant map's place in `BsShaderContext::constant_maps`: a host substitution for a shared
/// NiPointer, since slots share maps (00808DF0) that later builders still fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConstantMapId(pub u32);

// IDA: 009A4750 / 009A8490 (FormHeap(0x34) by the callers). Completion: 100%.
#[must_use]
pub fn create_shader_constant_map(pixel_rtti: bool) -> ShaderConstantMap {
    ShaderConstantMap {
        original_vtable: if pixel_rtti { 0x00AB_297C } else { 0x00AB_2BAC },
        pixel_rtti,
        field_08: 2,
        entries: Vec::new(),
        entry_capacity: 0,
        entry_size: 0,
        entry_count: 0,
        entry_grow: 1,
        field_1c: 1,
        field_20: 0,
        last_result: 0,
    }
}

// IDA: 009A9AD0 vt+0x4C -> 009A9A00: the first empty slot, else an append.
fn insert_constant_entry(map: &mut ShaderConstantMap, entry: Arc<ShaderConstantMapEntry>) -> u32 {
    let mut slot = map.entry_size as usize;
    for index in 0..map.entry_size as usize {
        if map.entries[index].is_none() {
            slot = index;
            break;
        }
    }
    if slot == map.entry_size as usize {
        if map.entry_size >= map.entry_capacity {
            map.entry_capacity = map.entry_size + map.entry_grow;
        }
        map.entry_size += 1;
    }
    if map.entries.len() <= slot {
        map.entries.resize(slot + 1, None);
    }
    map.entries[slot] = Some(entry);
    map.entry_count += 1;
    0
}

// IDA: 009A2480(source, out, 0x104, &count): the name and an optional "_D"
// register count (strchr '_', isdigit, the end; then strtok_s twice).
fn parse_predefined_key(key: &str, count: &mut u32) -> String {
    let bytes = key.as_bytes();
    let Some(underscore) = bytes.iter().position(|&b| b == b'_') else {
        return key.to_owned();
    };
    if underscore + 1 >= bytes.len()
        || !bytes[underscore + 1].is_ascii_digit()
        || underscore + 2 != bytes.len()
    {
        return key.to_owned();
    }
    let not_underscore = |from: usize| (from..bytes.len()).find(|&i| bytes[i] != b'_');
    let first = not_underscore(0).unwrap_or(bytes.len());
    let Some(first_end) = (first..bytes.len()).find(|&i| bytes[i] == b'_') else {
        return key.to_owned();
    };
    let Some(second) = not_underscore(first_end) else {
        return key.to_owned();
    };
    let second_end = (second..bytes.len())
        .find(|&i| bytes[i] == b'_')
        .unwrap_or(bytes.len());
    *count = atol(&bytes[second..second_end]) as u32; // 009A253C calls atol.
    key[first..first_end].to_owned()
}

// IDA: 009A48B0 the predefined mapping type.
const fn predefined_type(id: u32, count: u32) -> u32 {
    if (id >= 1 && id <= 24) || (id >= 37 && id <= 68) {
        return if count != 3 { 9 } else { 8 };
    }
    if (id >= 25 && id <= 32) || id == 34 || id == 36 {
        return 9;
    }
    if id == 33 || id == 35 {
        return 8;
    }
    if id >= 69 && id <= 87 {
        return 7;
    }
    0
}

// IDA: 009A49A0 NiD3DSCM_Vertex vt+0x50.
fn setup_vertex_predefined_entry(entry: &mut ShaderConstantMapEntry) -> u32 {
    let mut count = 4;
    let name = parse_predefined_key(&entry.key, &mut count);
    let Some(&(id, _)) = PREDEFINED_CONSTANT_NAMES
        .iter()
        .find(|(_, mapping)| stricmp(name.as_bytes(), mapping.as_bytes()) == 0)
    else {
        return 0x8000_0040;
    };
    entry.flags = (entry.flags & 0xFFFF_FF00) | predefined_type(id, count);
    if id == 0 {
        return 0x8000_0040;
    }
    if id < 0x45 {
        let high = (entry.extra >> 16) & 0xFFFF; // word +0x1A
        entry.register_count = match id {
            0x21 | 0x23 => 3 * high,
            0x22 | 0x24 => 4 * high,
            0x19..=0x20 => 4,
            _ => count,
        };
    }
    entry.predefined_id = id;
    if id == 0x50 || id == 0x51 {
        entry.owns_data = 0;
        entry.data_size = 16;
        entry.data_stride = 16;
        entry.data_reference = if id == 0x50 { "00B329C4" } else { "00B329D4" }.to_owned();
    }
    0
}

// IDA: 009A8800 vt+0x1C AddPredefinedEntry(key, extra, register, variable): a
// new entry (+0x18 extra, +0x1C register, the variable, +0x14 0x20000000),
// then vt+0x50: NiD3DSCM_Vertex 009A49A0 parses "Name_D" (009A2480: the first
// '_' followed by one digit and the end gives the register count D, else 4),
// finds the name in the 00B32590 table (_stricmp; missing -> 0x80000040),
// stores the type 009A48B0(id, count) in the flags' low byte (00B4294C
// identity, built by the 009A84B0 entry constructor), rejects id 0, sets
// +0x20 for ids below 0x45 (0x21/0x23: 3 * (extra >> 16), 0x22/0x24: 4 *
// (extra >> 16), 0x19..0x20: 4, else the count), +0x10 = id, and for
// TaylorSin/TaylorCos (0x50/0x51) size 16, stride 16, data 00B329C4/00B329D4;
// a nonzero result drops the entry, else vt+0x4C. No duplicate-key check
// (that belongs to vt+0x18). 100% for NiD3DSCM_Vertex.
// TODO: [ ] Port NiD3DSCM_Pixel's vt+0x50 (009A26F0) with its first caller;
//       it returns 0x80000040 (host) here.
pub fn add_predefined_constant_entry(
    map: &mut ShaderConstantMap,
    key: &str,
    extra: u32,
    register_index: u32,
    variable: &str,
) -> u32 {
    let mut entry = ShaderConstantMapEntry {
        key: key.to_owned(),
        extra,
        register_index,
        variable: variable.to_owned(),
        flags: 0x2000_0000,
        ..ShaderConstantMapEntry::default()
    };
    // vt+0x50: NiD3DSCM_Pixel 009A26F0 is a host 0x80000040 (not reached).
    map.last_result = if map.pixel_rtti {
        0x8000_0040
    } else {
        setup_vertex_predefined_entry(&mut entry)
    };
    if map.last_result != 0 {
        return map.last_result;
    }
    map.last_result = insert_constant_entry(map, Arc::new(entry));
    map.last_result
}

// IDA: 009A8660 vt+0x18 AddEntry(key, flags, extra, register, count, variable,
// size, stride, data, copy): +0x24 = 0; an existing key (vt+0x38 009A8DD0,
// _stricmp over [0, size)) gives 0x80000020; flags 0x1xxxxxxx run vt+0x24
// 009A8C40: the size picks the low type (8 -> 5, 0xC -> 6, 0x10 -> 7, 0x24 ->
// 8, 0x20/0x30/0x40 -> 9, else flags | 0x10000000); the extra is the caller's
// when the size is not a multiple of 4, the stride is not 4 or the type
// (00B428D8 identity table) is 1 or 3, else 1 for a NiD3DSCM_Vertex (exact
// RTTI 00BAA920) and 2 for a NiD3DSCM_Pixel; then vt+0x4C 009A9AD0 (009A9A00:
// the first empty slot or an append growing by +0x1A). Flags 0x2xxxxxxx run
// vt+0x1C(key, extra, register, variable). The 0x3/0x4/0x5/0x6 families are
// not reached by the ported classes and return 0x80000040 (host). 100% for
// the 0x1 and 0x2 families.
// TODO: [ ] Port the 0x3/0x4/0x5/0x6 families (vt+0x20/+0x28/+0x2C/+0x30)
//       with their first caller.
#[allow(clippy::too_many_arguments)] // 009A8660 vt+0x18 argument order.
pub fn add_shader_constant_entry(
    map: &mut ShaderConstantMap,
    key: &str,
    flags: u32,
    extra: u32,
    register_index: u32,
    count: u32,
    variable: &str,
    size: u32,
    stride: u32,
    data_reference: &str,
    copy: bool,
) -> u32 {
    // 009A866E..009A8681 the existing-key check (009A8DD0).
    map.last_result = 0;
    if find_shader_constant_entry(map, key).is_some() {
        map.last_result = 0x8000_0020;
        return map.last_result;
    }
    if (flags & 0xF000_0000) == 0x2000_0000 {
        // vt+0x1C(key, extra, register, variable)
        return add_predefined_constant_entry(map, key, extra, register_index, variable);
    }
    if (flags & 0xF000_0000) != 0x1000_0000 {
        map.last_result = 0x8000_0040; // host: family not reached by the ported classes
        return map.last_result;
    }
    // 009A8C40.
    let parameter_type = match size {
        0x08 => (flags & 0x0FFF_FFFA) | 0x1000_0005,
        0x0C => (flags & 0x0FFF_FFF9) | 0x1000_0006,
        0x10 => (flags & 0x0FFF_FFF8) | 0x1000_0007,
        0x24 => (flags & 0x0FFF_FFF7) | 0x1000_0008,
        0x20 | 0x30 | 0x40 => (flags & 0x0FFF_FFF6) | 0x1000_0009,
        _ => (flags & 0x0FFF_FFFF) | 0x1000_0000,
    };
    // 00B428D8 (00783C70) and 00B42914 (00783D70 through 009A2450) are the
    // identity type tables for 0..13.
    let low = parameter_type & 0xFF;
    let resolved_extra = if (size & 3) == 0 && stride == 4 && low != 1 && low != 3 {
        if map.pixel_rtti { 2 } else { 1 }
    } else {
        extra
    };
    let entry = ShaderConstantMapEntry {
        key: key.to_owned(), // 009A8520 (an empty key stays null)
        flags: parameter_type,
        extra: resolved_extra,
        register_index,
        register_count: count,
        variable: variable.to_owned(),
        data_size: size,
        data_stride: stride,
        owns_data: u8::from(copy),
        data_reference: data_reference.to_owned(),
        ..ShaderConstantMapEntry::default()
    };
    map.last_result = insert_constant_entry(map, Arc::new(entry));
    map.last_result
}

// IDA: 009A8DD0 vt+0x38 GetEntry(key): the first entry in [0, size) whose key
// matches (_stricmp), else null. Completion: 100%.
#[must_use]
pub fn find_shader_constant_entry(
    map: &ShaderConstantMap,
    key: &str,
) -> Option<Arc<ShaderConstantMapEntry>> {
    map.entries
        .iter()
        .take(map.entry_size as usize)
        .flatten()
        .find(|entry| stricmp(key.as_bytes(), entry.key.as_bytes()) == 0)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `verify_shader_runtime`, except the two path checks, which are in `bs::shader_programs`.
    #[test]
    fn verify_shader_runtime() {
        assert_eq!(sampler_state_slot(1), 0);
        assert_eq!(sampler_state_slot(2), 1);
        assert_eq!(sampler_state_slot(3), 0xFFFF);
        assert_eq!(sampler_state_slot(5), 2);
        assert_eq!(sampler_state_slot(7), 4);
        assert_eq!(texture_stage_state_slot(1), 0);
        assert_eq!(texture_stage_state_slot(6), 5);
        assert_eq!(texture_stage_state_slot(7), 6);
        assert_eq!(texture_stage_state_slot(11), 0xFFFF);
        assert_eq!(texture_stage_state_slot(24), 7);
        let mut pools = ShaderRuntimePools {
            max_texture_blend_stages: 8,
            max_sampler_stages: 16,
            ..ShaderRuntimePools::default()
        };
        // Free-list order: take [0], move the last item into it.
        let order: Vec<PassId> = (0..9).map(|_| pools.acquire_pass()).collect();
        assert_eq!(
            pools.pass_slab_starts,
            [0, 8],
            "The pass pool grows by 8 then 16"
        );
        assert_eq!(pools.pass_counters.records, 24);
        assert_eq!(pools.pass_counters.next_batch, 32);
        assert_eq!(pools.pass_counters.free_count, 15);
        // 00760600/007606A0 order.
        assert_eq!(order[0], PassId(0));
        assert_eq!(order[1], PassId(7));
        assert_eq!(order[2], PassId(6));
        assert_eq!(order[7], PassId(1));
        assert_eq!(order[8], PassId(pools.pass_slab_starts[1]));
        assert_eq!(pools.pass(order[0]).references, 1);
        assert_eq!(pools.pass(order[0]).acquired, 1);
        assert_eq!(pools.pass(order[0]).stages.len(), 8);
        // A released pass returns to the end of the free list.
        release_pass_reference(&mut pools, Some(order[0]));
        assert_eq!(pools.pass_counters.free_count, 16);
        assert_eq!(pools.free_passes.last(), Some(&order[0]));
        // Stage records acquire their 184-byte groups when constructed.
        let stage = pools.acquire_texture_stage();
        assert_eq!(pools.stage_group_counters.records, 8);
        assert_eq!(pools.stage_group_counters.free_count, 0);
        assert_eq!(pools.stage_states(stage).acquired, 1);
        assert_eq!(pools.stage(stage).references, 1);
        // 00772FF0: a move from the unsaved list counts the saved list twice.
        let group = pools.stage_states_mut(stage);
        group.set_stage_state(1, 4, false);
        assert_eq!(group.unsaved_stage_count, 1);
        assert_eq!(group.unsaved_stage_values[0], 4);
        group.set_stage_state(1, 5, true);
        assert_eq!(
            group.saved_stage_count, 2,
            "the double count of 00773030..0077304C"
        );
        assert_eq!(group.unsaved_stage_count, 0);
        assert_eq!(group.saved_stage_flags[0], 1);
        assert_eq!(group.unsaved_stage_flags[0], 0);
        group.set_stage_state(11, 3, false);
        assert_eq!(group.unsaved_stage_count, 0, "TEXCOORDINDEX has no slot");
        // 00773100 use-map counting.
        group.set_sampler_state(1, 3, false, true);
        group.set_sampler_state(1, 4, false, true);
        assert_eq!(group.unsaved_sampler_count, 1);
        assert_eq!(group.mapped_sampler_count, 1);
        group.set_sampler_state(1, 4, true, false);
        assert_eq!(
            group.saved_sampler_count, 1,
            "the sampler move drops the map flag"
        );
        assert_eq!(group.unsaved_sampler_count, 0);
        assert_eq!(group.mapped_sampler_count, 0);
        // 00772CD0: head insertion, update in place, move between lists.
        let states = pools.acquire_render_state_group();
        set_render_state(&mut pools, states, 0x1B, 0, false);
        set_render_state(&mut pools, states, 0x0F, 1, false);
        set_render_state(&mut pools, states, 0x1B, 1, false);
        let unsaved = pools.render_state_entries(states, false);
        assert_eq!(unsaved.len(), 2);
        assert_eq!(unsaved[0].state, 0x0F);
        assert_eq!(unsaved[1].state, 0x1B);
        assert_eq!(unsaved[1].value, 1);
        assert_eq!(pools.render_entry_counters.records, 16);
        set_render_state(&mut pools, states, 0x1B, 2, true);
        assert_eq!(pools.render_groups[states.0 as usize].unsaved_count, 1);
        assert_eq!(pools.render_groups[states.0 as usize].saved_count, 1);
        assert_eq!(pools.render_state_entries(states, true)[0].value, 2);
        // 00760010 SetStage and 00801110 defaults.
        let mut renderer = RendererState::default();
        for (index, value) in renderer.texture_stage_globals.iter_mut().enumerate() {
            *value = index as u32 + 100;
        }
        let pass = order[1];
        let second = pools.acquire_texture_stage();
        set_texture_stage_defaults(&renderer, &mut pools, second, 2, 3, 2);
        assert_eq!(pools.stage(second).stage, 2);
        let group = pools.stage_states(second);
        assert_eq!(group.unsaved_stage_count, 5);
        assert_eq!(group.unsaved_stage_values[0], 1);
        assert_eq!(group.unsaved_stage_values[1], 2);
        assert_eq!(group.unsaved_stage_values[7], 0);
        assert_eq!(group.unsaved_sampler_values[0], 3);
        assert_eq!(group.unsaved_sampler_values[2], 107); // 00B420EC + 24
        assert_eq!(group.unsaved_sampler_values[3], 106);
        assert_eq!(group.unsaved_sampler_values[4], 108);
        let end = pools.pass(pass).stage_end;
        assert_eq!(set_pass_stage(&mut pools, pass, end, Some(second)), 0);
        assert_eq!(pools.pass(pass).stage_end, 1);
        assert_eq!(pools.pass(pass).stage_count, 1);
        assert_eq!(pools.pass(pass).stages[0], Some(second));
        assert_eq!(pools.stage(second).stage, 0);
        assert_eq!(pools.stage(second).references, 2);
        assert_eq!(pools.pass(pass).stage_array_size, 1);
        assert_eq!(set_pass_stage(&mut pools, pass, 0, None), 0);
        assert_eq!(pools.pass(pass).stage_end, 0);
        assert_eq!(pools.pass(pass).stage_count, 0);
        assert_eq!(pools.stage(second).references, 1);
        assert_eq!(
            set_pass_stage(&mut pools, pass, 20, Some(second)),
            1,
            "past the sampler stages"
        );
        // 0076FB30/0076FC40.
        renderer.max_streams_6dc = 1;
        assert!(
            create_shader_declaration(&renderer, 5, 2).is_none(),
            "00771090 checks the stream count"
        );
        let mut declaration = create_shader_declaration(&renderer, 5, 1).unwrap();
        assert_eq!(declaration.streams[0].len(), 5);
        assert_eq!(declaration.streams[0][0].parameter, 0xFFFF_FFFF);
        assert_eq!(declaration.streams[0][0].parameter_type, 17);
        assert!(set_declaration_entry_by_parameter(
            &mut declaration,
            2,
            5,
            4,
            3,
            0
        ));
        assert_eq!(
            declaration.streams[0][2].usage, 10,
            "0076FB30 parameter 4 is COLOR"
        );
        assert_eq!(declaration.dirty, 1);
        assert!(set_declaration_entry_by_parameter(
            &mut declaration,
            3,
            7,
            6,
            1,
            0
        ));
        assert_eq!(declaration.streams[0][3].usage, 5);
        assert_eq!(declaration.streams[0][3].usage_index, 1);
        assert_eq!(declaration.streams[0][3].parameter_type, 1);
        assert!(set_declaration_entry(
            &mut declaration,
            0,
            1,
            0,
            1,
            3,
            3,
            0,
            0
        ));
        assert_eq!(
            declaration.streams[0][1].parameter_type, 2,
            "0076FC40 parameter 1 forces type 2"
        );
        assert!(!set_declaration_entry_by_parameter(
            &mut declaration,
            0,
            0,
            15,
            2,
            0
        ));
        assert!(!set_declaration_entry(
            &mut declaration,
            1,
            0,
            0,
            0,
            2,
            0,
            0,
            0
        ));
    }

    /// `verify_constant_maps`: the 009A8660/009A8C40/009A8800/009A49A0 constant-map rules.
    #[test]
    fn verify_constant_maps() {
        let mut pixel = create_shader_constant_map(true);
        let mut vertex = create_shader_constant_map(false);
        assert_eq!(pixel.original_vtable, 0x00AB_297C);
        assert_eq!(vertex.original_vtable, 0x00AB_2BAC);
        // Family 0x1: the size picks the type through masks that keep the low bits
        // (0x10000007 with 12 bytes stays 0x10000007); aligned float data takes the map's extra.
        assert_eq!(
            add_shader_constant_entry(
                &mut pixel,
                "Time",
                0x1000_0007,
                0,
                0,
                1,
                "",
                16,
                4,
                "this+0x100",
                false
            ),
            0
        );
        assert_eq!(
            add_shader_constant_entry(
                &mut vertex,
                "Velocity",
                0x1000_0007,
                0,
                11,
                1,
                "",
                12,
                4,
                "this+0xF4",
                false
            ),
            0
        );
        assert_eq!(
            add_shader_constant_entry(
                &mut vertex,
                "Params",
                0x1000_0004,
                0,
                12,
                1,
                "",
                4,
                4,
                "this+0xDC",
                false
            ),
            0
        );
        let time = find_shader_constant_entry(&pixel, "TIME").unwrap();
        let velocity = find_shader_constant_entry(&vertex, "velocity").unwrap();
        let params = find_shader_constant_entry(&vertex, "Params").unwrap();
        assert_eq!((time.flags, time.extra), (0x1000_0007, 2));
        assert_eq!((velocity.flags, velocity.extra), (0x1000_0007, 1));
        assert_eq!((params.flags, params.extra), (0x1000_0004, 1));
        assert_eq!(
            add_shader_constant_entry(
                &mut pixel,
                "time",
                0x1000_0007,
                0,
                3,
                1,
                "",
                16,
                4,
                "",
                false
            ),
            0x8000_0020
        );
        assert_eq!(pixel.entry_size, 1);
        // Family 0x2: the predefined table, register counts and types.
        assert_eq!(
            add_shader_constant_entry(
                &mut vertex,
                "WorldViewProjTranspose",
                0x2000_0009,
                0,
                0,
                4,
                "",
                0,
                0,
                "",
                false
            ),
            0
        );
        assert_eq!(
            add_shader_constant_entry(
                &mut vertex,
                "BoneMatrix3",
                0x2000_0009,
                0x12_0000,
                42,
                54,
                "",
                0,
                0,
                "",
                false
            ),
            0
        );
        assert_eq!(
            add_predefined_constant_entry(&mut vertex, "WorldTranspose_3", 0x2000_0009, 4, ""),
            0
        );
        assert_eq!(
            add_predefined_constant_entry(&mut vertex, "TaylorSin", 0, 20, ""),
            0
        );
        let wvp = find_shader_constant_entry(&vertex, "WorldViewProjTranspose").unwrap();
        let bones = find_shader_constant_entry(&vertex, "BoneMatrix3").unwrap();
        let world3 = find_shader_constant_entry(&vertex, "WorldTranspose_3").unwrap();
        let taylor = find_shader_constant_entry(&vertex, "TaylorSin").unwrap();
        assert_eq!(
            (wvp.predefined_id, wvp.flags, wvp.register_count),
            (23, 0x2000_0009, 4)
        );
        assert_eq!(
            (bones.predefined_id, bones.flags, bones.register_count),
            (33, 0x2000_0008, 54)
        );
        assert_eq!(
            (
                world3.predefined_id,
                world3.flags,
                world3.register_count,
                world3.extra
            ),
            (17, 0x2000_0008, 3, 0x2000_0009)
        );
        assert_eq!(
            (taylor.predefined_id, taylor.data_size, taylor.data_stride),
            (80, 16, 16)
        );
        assert_eq!(taylor.data_reference, "00B329C4");
        let before = vertex.entry_size;
        assert_eq!(
            add_predefined_constant_entry(&mut vertex, "NotAMapping", 0, 0, ""),
            0x8000_0040
        );
        assert_eq!(vertex.entry_size, before);
        assert_eq!(
            add_predefined_constant_entry(&mut pixel, "WorldTranspose", 0, 0, ""),
            0x8000_0040
        );
    }
}
