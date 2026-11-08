//! NiSearchPath and BSSearchPath (0x210 bytes), owned by NiStream +0x1E8.

use bevy_ecs::prelude::*;

/// IDA: 00747xxx NiSearchPath's fixed-buffer width.
pub const NI_SEARCH_PATH_BUFFER: usize = 0x104;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchPathClass {
    /// IDA: vtable A853C4 (007478C0). Its 007479F0 PathParts walk is not ported.
    NiSearchPath,
    /// IDA: vtable A363E8 (00430D60).
    BsSearchPath,
}

// IDA: N/A a helper, with no address of its own.
fn c_prefix(text: &str) -> &str {
    text.split('\0').next().unwrap_or(text)
}

/// IDA: 007478C0: vtable, +0x04 index, +0x08 file path, +0x10C reference path. Completion: 100% of the
/// state, SetFilePath, Reset and BSSearchPath's GetNextSearchPath.
/// TODO: [ ] The base 007479F0 walk, if a stream created before 0040E63D ever searches.
#[derive(Clone, Debug)]
pub struct StreamSearchPath {
    class: SearchPathClass,
    index_04: u32,
    file_path_08: String,
    has_reference_path: bool,
    unsupported_base_walk: bool,
}

impl StreamSearchPath {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub const fn new(class: SearchPathClass) -> Self {
        Self {
            class,
            index_04: 0,
            file_path_08: String::new(),
            has_reference_path: false,
            unsupported_base_walk: false,
        }
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn class(&self) -> SearchPathClass {
        self.class
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index_04
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn file_path(&self) -> &str {
        &self.file_path_08
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn has_reference_path(&self) -> bool {
        self.has_reference_path
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn unsupported_base_walk(&self) -> bool {
        self.unsupported_base_walk
    }

    /// IDA: 007478F0: null or empty clears it; otherwise strncpy_s(+0x08, 0x104, source, 0x103).
    pub fn set_file_path(&mut self, source: Option<&str>) {
        let text = source.map_or("", c_prefix);
        self.file_path_08 = text[..text.len().min(NI_SEARCH_PATH_BUFFER - 1)].to_owned();
    }

    /// IDA: 00747930: only presence is kept, because BSSearchPath never reads the directory.
    pub fn set_reference_path(&mut self, full_path: Option<&str>) {
        self.has_reference_path = full_path.is_some_and(|path| !c_prefix(path).is_empty());
    }

    /// IDA: 0046C9C0 vtable +0x04 Reset.
    pub const fn reset(&mut self) {
        self.index_04 = 0;
    }

    /// IDA: 00430D30: exactly one candidate per Reset, the file path, possibly empty. The base class
    /// reports `None` and sets `unsupported_base_walk`.
    pub fn next_search_path(&mut self) -> Option<String> {
        if self.class != SearchPathClass::BsSearchPath {
            self.unsupported_base_walk = true;
            return None;
        }
        if self.index_04 != 0 {
            return None;
        }
        self.index_04 += 1;
        Some(self.file_path_08.clone())
    }
}

/// IDA: 00B40334 the creator slot: null in the image; WinMain 0040E63D stores 00430D60.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchPathCreatorSlot {
    pub bs_search_path_installed: bool,
}

impl SearchPathCreatorSlot {
    /// IDA: 00711EF0 NiStream_CreateSearchPath: the slot, or a base NiSearchPath.
    #[must_use]
    pub const fn create(&self) -> StreamSearchPath {
        StreamSearchPath::new(if self.bs_search_path_installed {
            SearchPathClass::BsSearchPath
        } else {
            SearchPathClass::NiSearchPath
        })
    }
}

/// IDA: 007023E0's candidate loop on a BSSearchPath.
#[must_use]
pub fn bs_search_path_candidates(file_path: &str) -> Vec<String> {
    let mut search = StreamSearchPath::new(SearchPathClass::BsSearchPath);
    search.set_file_path(Some(file_path));
    search.reset();
    let mut candidates = Vec::new();
    while let Some(next) = search.next_search_path() {
        candidates.push(next);
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from `gamebryo_search_path`.
    #[test]
    fn bs_search_path_yields_one_candidate() {
        let mut search = StreamSearchPath::new(SearchPathClass::BsSearchPath);
        search.set_file_path(Some("textures\\sky\\CloudsClear.dds"));
        search.reset();
        assert_eq!(
            search.next_search_path().as_deref(),
            Some("textures\\sky\\CloudsClear.dds")
        );
        assert_eq!(search.index(), 1);
        assert!(search.next_search_path().is_none());
        search.reset();
        assert!(search.next_search_path().is_some(), "Reset re-arms it");

        search.set_file_path(None);
        search.reset();
        assert_eq!(
            search.next_search_path().as_deref(),
            Some(""),
            "one empty candidate"
        );

        search.set_file_path(Some(&"a".repeat(0x200)));
        assert_eq!(search.file_path().len(), 0x103);
        search.set_file_path(Some("abc\0def"));
        assert_eq!(search.file_path(), "abc");

        assert_eq!(bs_search_path_candidates("a.dds"), ["a.dds"]);

        let mut base = StreamSearchPath::new(SearchPathClass::NiSearchPath);
        base.set_file_path(Some("x.dds"));
        base.reset();
        assert!(base.next_search_path().is_none());
        assert!(base.unsupported_base_walk());
    }

    /// IDA: the 00B40334 slot and the 00711EF0 branch.
    #[test]
    fn creator_slot_selects_the_class() {
        let mut slot = SearchPathCreatorSlot::default();
        assert_eq!(slot.create().class(), SearchPathClass::NiSearchPath);
        slot.bs_search_path_installed = true;
        assert_eq!(slot.create().class(), SearchPathClass::BsSearchPath);
    }
}
