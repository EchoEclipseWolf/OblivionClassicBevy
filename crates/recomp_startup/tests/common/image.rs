//! IDA: N/A a form's bytes rebuilt from its components, to compare with a row's raw dump, as `phase11_player.rs` does with its `Image`. A cell is a byte, an unmodelled byte, or a pointer the run's bytes are only tested for null-ness. Row printers walk ordered indexes, so nothing here sees an `Entity`.

use bevy_ecs::prelude::*;
use recomp_game::tes::data_handler::{DataHandler, TesDescription, TesTexture};
use recomp_game::tes::form::{FormIdentity, ModReferences, OriginalVtable};
use recomp_game::tes::form_components::{FormRef, TesFullName};
use recomp_game::tes::tes_file::BsStringT;

#[derive(Clone, Debug)]
enum Cell {
    Byte(u8),
    /// A byte the original leaves as FormHeap or padding bytes: not compared.
    Unset,
    /// The first byte of a 4-byte pointer, compared by null-ness. The text is what the DATA row behind it must start with.
    Pointer {
        role: &'static str,
        present: bool,
        text: Option<Vec<u8>>,
        file_name: Option<String>,
    },
    /// The other three bytes of a pointer.
    PointerTail,
    /// A pointer whose DATA row must start with these cells.
    PointerBlock {
        role: &'static str,
        present: bool,
        block: Vec<BlockCell>,
    },
}

/// IDA: N/A a cell of the block behind a pointer, against the 0x40 bytes its DATA row holds.
#[derive(Clone, Debug)]
pub enum BlockCell {
    Byte(u8),
    /// A byte the original leaves unset.
    Unset,
    /// A word holding a pointer, by null-ness.
    Word(bool),
}

/// IDA: N/A a block's dwords as bytes.
#[must_use]
pub fn block_of(bytes: &[u8]) -> Vec<BlockCell> {
    bytes.iter().map(|byte| BlockCell::Byte(*byte)).collect()
}

pub struct Image {
    cells: Vec<Cell>,
}

impl Image {
    #[must_use]
    pub fn new(size: usize) -> Self {
        Self {
            cells: vec![Cell::Unset; size],
        }
    }

    pub fn u8(&mut self, at: usize, value: u8) {
        self.cells[at] = Cell::Byte(value);
    }

    pub fn u16(&mut self, at: usize, value: u16) {
        for (index, byte) in value.to_le_bytes().into_iter().enumerate() {
            self.u8(at + index, byte);
        }
    }

    pub fn u32(&mut self, at: usize, value: u32) {
        for (index, byte) in value.to_le_bytes().into_iter().enumerate() {
            self.u8(at + index, byte);
        }
    }

    pub fn i32(&mut self, at: usize, value: i32) {
        self.u32(at, value as u32);
    }

    pub fn f32(&mut self, at: usize, value: f32) {
        self.u32(at, value.to_bits());
    }

    pub fn bytes(&mut self, at: usize, bytes: &[u8]) {
        for (index, byte) in bytes.iter().enumerate() {
            self.u8(at + index, *byte);
        }
    }

    pub fn pointer(&mut self, at: usize, role: &'static str, present: bool) {
        self.pointer_cell(at, role, present, None, None);
    }

    fn pointer_cell(
        &mut self,
        at: usize,
        role: &'static str,
        present: bool,
        text: Option<Vec<u8>>,
        file_name: Option<String>,
    ) {
        self.cells[at] = Cell::Pointer {
            role,
            present,
            text,
            file_name,
        };
        for offset in 1..4 {
            self.cells[at + offset] = Cell::PointerTail;
        }
    }

    /// IDA: BSStringT {+0 char*, +4 u16 length, +6 u16 capacity}.
    pub fn bs_string(&mut self, at: usize, role: &'static str, text: &BsStringT) {
        self.pointer_cell(at, role, text.data_00.is_some(), text.data_00.clone(), None);
        self.u16(at + 4, text.length_04);
        self.u16(at + 6, text.capacity_06);
    }

    /// IDA: a slot holding a raw form ID until InitItem.
    pub fn form_ref(&mut self, at: usize, role: &'static str, slot: FormRef) {
        match slot {
            FormRef::Raw(id) => self.u32(at, id),
            FormRef::Resolved(form) => self.pointer(at, role, form.is_some()),
        }
    }

    /// IDA: TESForm's head, +0x00..+0x18: the +0x05..+0x07 bytes stay unset; +0x10 is the mod reference list, whose head item's DATA row is the TESFile.
    pub fn form_head(&mut self, world: &World, form: Entity) {
        let identity = world.get::<FormIdentity>(form).unwrap();
        self.u32(0x00, world.get::<OriginalVtable>(form).unwrap().0);
        self.u8(0x04, identity.form_type_04);
        self.u32(0x08, identity.flags_08);
        self.u32(0x0C, identity.form_id_0c);
        let references = &world.get::<ModReferences>(form).unwrap().0;
        let file_name = references
            .first()
            .and_then(|&index| compiled_file_name(world, index));
        self.pointer_cell(
            0x10,
            "mod reference item",
            !references.is_empty(),
            None,
            file_name,
        );
        self.pointer(0x14, "mod reference next", references.len() > 1);
    }

    pub fn full_name(&mut self, at: usize, component: &TesFullName) {
        self.u32(at, component.original_vtable);
        self.bs_string(at + 4, "full name", &component.name_04);
    }

    pub fn description(&mut self, at: usize, component: &TesDescription) {
        self.u32(at, component.original_vtable);
        self.u32(at + 4, component.offset_04);
    }

    pub fn texture(&mut self, at: usize, component: &TesTexture) {
        self.u32(at, component.original_vtable);
        self.bs_string(at + 4, "texture path", &component.path_04);
    }

    /// IDA: N/A a pointer with the block behind it.
    pub fn pointer_block(
        &mut self,
        at: usize,
        role: &'static str,
        present: bool,
        block: Vec<BlockCell>,
    ) {
        self.cells[at] = Cell::PointerBlock {
            role,
            present,
            block,
        };
        for offset in 1..4 {
            self.cells[at + offset] = Cell::PointerTail;
        }
    }

    /// IDA: N/A (bytes compared exactly, pointers by null-ness, unset), for a report.
    #[must_use]
    pub fn coverage(&self) -> (usize, usize, usize) {
        let mut counts = (0, 0, 0);
        for cell in &self.cells {
            match cell {
                Cell::Byte(_) => counts.0 += 1,
                Cell::Pointer { .. } | Cell::PointerBlock { .. } => counts.1 += 1,
                Cell::Unset => counts.2 += 1,
                Cell::PointerTail => {}
            }
        }
        counts
    }

    /// IDA: N/A the image's first bytes, for a dump that stops short of the object.
    pub fn truncate(&mut self, size: usize) {
        self.cells.truncate(size);
    }

    /// IDA: N/A compares the image with a row's raw bytes. `data` answers a pointer's DATA row by its offset. Every byte is checked, and every failure listed.
    pub fn compare(&self, raw: &[u8], tag: &str, data: &dyn Fn(u32) -> Option<Vec<u8>>) {
        assert_eq!(self.cells.len(), raw.len(), "{tag}: image size");
        self.compare_start(raw, tag, data);
    }

    /// IDA: N/A as `compare`, over as much of the image as `raw` holds, for a dump of a form's first 0x40 bytes. A pointer cut off by the dump's end isn't compared.
    pub fn compare_start(&self, raw: &[u8], tag: &str, data: &dyn Fn(u32) -> Option<Vec<u8>>) {
        let mut failures = Vec::new();
        for (at, cell) in self
            .cells
            .iter()
            .enumerate()
            .take(raw.len().saturating_sub(3))
        {
            match cell {
                Cell::Byte(byte) if *byte != raw[at] => {
                    failures.push(format!("+{at:X} host {byte:02X} run {:02X}", raw[at]));
                }
                Cell::Pointer {
                    role,
                    present,
                    text,
                    file_name,
                } => {
                    let word = u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
                    if (word != 0) != *present {
                        failures.push(format!(
                            "+{at:X} {role}: host present={present} run {word:08X}"
                        ));
                    }
                    if !present {
                        continue;
                    }
                    let behind = data(at as u32);
                    if let (Some(text), Some(behind)) = (text, &behind) {
                        // The dump holds 0x40 bytes, so a longer string compares by its start.
                        let run = behind.split(|&byte| byte == 0).next().unwrap();
                        let matches =
                            text.starts_with(run) && (run.len() == 0x40 || text.len() == run.len());
                        if !matches {
                            failures.push(format!(
                                "+{at:X} {role}: host {:?} run {:?}",
                                String::from_utf8_lossy(text),
                                String::from_utf8_lossy(run)
                            ));
                        }
                    }
                    // A mod reference's DATA row is the TESFile's first 0x40 bytes: its +0x1C name.
                    if let (Some(name), Some(behind)) = (file_name, &behind) {
                        let run = behind[0x1C..].split(|&byte| byte == 0).next().unwrap();
                        if run != name.as_bytes() {
                            failures.push(format!(
                                "+{at:X} {role}: host {name} run {:?}",
                                String::from_utf8_lossy(run)
                            ));
                        }
                    }
                }
                Cell::PointerBlock {
                    role,
                    present,
                    block,
                } => {
                    let word = u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
                    if (word != 0) != *present {
                        failures.push(format!(
                            "+{at:X} {role}: host present={present} run {word:08X}"
                        ));
                    }
                    if !present {
                        continue;
                    }
                    if let Some(behind) = data(at as u32) {
                        compare_block(block, &behind, at, role, &mut failures);
                    }
                }
                _ => {}
            }
        }
        assert!(failures.is_empty(), "{tag}: {}", failures.join("; "));
    }
}

// IDA: N/A a block against the bytes the run dumped, as far as the dump goes.
fn compare_block(
    block: &[BlockCell],
    behind: &[u8],
    at: usize,
    role: &str,
    failures: &mut Vec<String>,
) {
    let mut cursor = 0;
    for cell in block {
        match cell {
            BlockCell::Byte(byte) => {
                if cursor < behind.len() && behind[cursor] != *byte {
                    failures.push(format!(
                        "+{at:X} {role} block +{cursor:X}: host {byte:02X} run {:02X}",
                        behind[cursor]
                    ));
                }
                cursor += 1;
            }
            BlockCell::Unset => cursor += 1,
            BlockCell::Word(present) => {
                if cursor + 4 <= behind.len() {
                    let word = u32::from_le_bytes(behind[cursor..cursor + 4].try_into().unwrap());
                    if (word != 0) != *present {
                        failures.push(format!(
                            "+{at:X} {role} block +{cursor:X}: host present={present} run {word:08X}"
                        ));
                    }
                }
                cursor += 4;
            }
        }
    }
}

// IDA: N/A the file at a mod index, by the compiled array.
fn compiled_file_name(world: &World, index: u8) -> Option<String> {
    let data = world.resource::<DataHandler>();
    let id = (*data.mods_by_id_8d4.get(usize::from(index))?)?;
    Some(data.file_store.get(id)?.name_1c.clone())
}
