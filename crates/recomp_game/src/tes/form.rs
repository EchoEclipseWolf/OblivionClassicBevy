//! TESForm's state as components. Original object offsets are in the field names.

use bevy_ecs::prelude::*;

/// IDA: 0046C470 TESForm's identity: +0x04 form type, +0x08 flags, +0x0C form ID.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormIdentity {
    pub form_type_04: u8,
    pub flags_08: u32,
    pub form_id_0c: u32,
}

/// IDA: the most-derived vtable the constructor stores at +0x00. Evidence only, never called; the
/// rows assert it. Completion: 100%.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OriginalVtable(pub u32);

/// IDA: TESForm +0x10 the files that touched the form, as mod indexes, in list order.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct ModReferences(pub Vec<u8>);

impl ModReferences {
    /// IDA: 0046B6CC TESForm_UpdateModReferenceList(file): a master clears the list and is
    /// prepended; a file already present returns; otherwise it goes after the last entry.
    /// Completion: 90%.
    /// TODO: [ ] The ghost-parent test (M4). `update_mod_reference_list` stops on a ghost file.
    pub fn update(&mut self, mod_index: u8, is_master: bool) {
        if is_master {
            self.0.clear();
            self.0.push(mod_index);
        } else if !self.0.contains(&mod_index) {
            self.0.push(mod_index);
        }
    }
}

impl ModReferences {
    /// IDA: 0046B6DD..0046B74B UpdateModReferenceList(null): remove the last non-empty entry (the head when none): its next node is pulled in, or its item cleared.
    /// Completion: 100%.
    pub fn remove_last(&mut self) {
        self.0.pop();
    }
}

/// IDA: 0044DD69 the flag that keeps a form's map entry, and marks a partial form.
pub const FORM_FLAG_PARTIAL: u32 = 0x4000;
