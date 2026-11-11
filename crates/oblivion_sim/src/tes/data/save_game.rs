//! Phase 18: PlayerCharacter_SaveInitialState (006632A0), the player's save written to a FormHeap buffer before a game starts.
//!
//! The chain is the save game's own: 00661540 -> 005E7EF0 -> 00659C30 -> 004E2D60, then the process's 006324F0 -> 00651140 -> 00658CB0 -> 006471A0 -> 0060D1F0. Format version 125 (`g_saveLoadGame` +0x7C) takes every `>=` gate up to 0x7B, and skips `0045A170`'s block headers and 0x7E's arm; `byte_B05BAC` is 0, so there's no logging. An address's other arms are stops: a flag other than 4, or a list with an item. The only reader, 00663340 RestoreInitialState, isn't ported.
//!
//! Evidence: disassembly; `run-i-phase18/initial-state.bin`, byte for byte, in `the_port_matches_the_phase_18_rows`, but for bytes the exe never stores: the process's +0x88 and +0xD8..+0xDF, the controller's +0x2E0..+0x2EF and +0x31C, the actor's +0x84 and +0xD8, and the player's +0x5A8. They're heap bytes, the same in two runs, and the port writes its zeros. Fields the port has no slot for write literal zeros: +0xD4..+0xDC, +0x2E0, +0x31C.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::bs::character_controller::BhkCharacterController;
use crate::tes::actor_process::{
    BaseProcess, HighProcess, LowProcess, MiddleHighProcess, MiddleLowProcess,
};
use crate::tes::form::FormIdentity;

// IDA: 0045B9A0 and 0045F7A0: +0x14 past each write. +0x7D is 0 through this call, so 0045F7A0 stores raw IDs and not 0045E0D0's.
#[derive(Default)]
pub(crate) struct SaveWriter {
    pub(crate) bytes: Vec<u8>,
}

impl SaveWriter {
    // IDA: N/A a helper over `Vec`.
    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    // IDA: N/A a helper over `Vec`.
    pub(crate) fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    // IDA: N/A a helper over `Vec`.
    pub(crate) fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    // IDA: N/A a helper over `Vec`.
    pub(crate) fn f32(&mut self, value: f32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    // IDA: N/A a helper over `Vec`.
    pub(crate) fn words(&mut self, values: &[u32]) {
        for value in values {
            self.u32(*value);
        }
    }

    // IDA: N/A a helper over `Vec`.
    pub(crate) fn floats(&mut self, values: &[f32]) {
        for value in values {
            self.f32(*value);
        }
    }
}

// IDA: a form slot a first start holds null, written as its ID of 0. Anything else is an arm no run reached.
pub(crate) fn null_id(word: u32, address: &'static str) -> Result<u32, GameError> {
    if word == 0 {
        Ok(0)
    } else {
        Err(GameError::from_legacy_message(address))
    }
}

// IDA: a `{item, next}` list head a first start holds empty.
pub(crate) fn empty_list(head: [u32; 2], address: &'static str) -> Result<(), GameError> {
    if head == [0, 0] {
        Ok(())
    } else {
        Err(GameError::from_legacy_message(address))
    }
}

// IDA: `*(form + 0x0C)`, or 0 for a null.
pub(crate) fn form_id(world: &World, form: Option<Entity>) -> u32 {
    form.and_then(|form| world.get::<FormIdentity>(form))
        .map_or(0, |identity| identity.form_id_0c)
}

// IDA: N/A the process's five class levels, read together.
pub(crate) struct ProcessView<'a> {
    pub(crate) base: &'a BaseProcess,
    pub(crate) low: &'a LowProcess,
    pub(crate) middle_low: &'a MiddleLowProcess,
    pub(crate) middle_high: &'a MiddleHighProcess,
    pub(crate) high: &'a HighProcess,
}

// IDA: N/A the character controller, process vt+0x18C: 006348E0's +0x118.
pub(crate) fn controller<'a>(
    world: &'a World,
    process: &ProcessView<'_>,
) -> Option<&'a BhkCharacterController> {
    process
        .middle_high
        .char_controller_118
        .and_then(|entity| world.get(entity))
}
