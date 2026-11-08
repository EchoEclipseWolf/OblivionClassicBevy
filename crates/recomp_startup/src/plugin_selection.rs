//! Phase 10: the plugin selection of 00406360 Startup_InitializeGameData, 00406687..00406773, after the Phase 09 world update 00406682 and before the Phase 11 FormHeap_Allocate(0x804) at 00406774: the ten
//! sTestFile1..10:General settings, then 00404B00 over <00B3F178>.\Plugins.txt, then the Oblivion.esm fallback. "Selecting" a plugin is 0044FAD0 TESFile_SetLoaded(1) on the 00447C50 lookup result; nothing else
//! is stored. The first reader of the Loaded bit is 0044F3D0 DataHandler_LoadSelectedFiles (Phases 11 and 12), so only the caller and the tests see this phase. The window holds five direct callsites and no
//! indirect one.
//!
//! Recorded host substitutions: `TextModeFile` for the original's statically linked CRT stream, reading the whole file at the open, so a read error is a null stream where the original's `fgets` would end the
//! loop; the opened path resolves case-insensitively on Linux; the FileName overflow is reported and the full path kept. Plugins.txt is only read. The Phase 04 archive reader of the same file (0042F760) is a
//! separate original read.
//! Evidence: evidence/phase10-scope-2026-10-06/disasm-window.txt, P10-RUN-001.

use bevy_ecs::prelude::*;
use recomp_core::crt::TextModeFile;
use recomp_core::host_path;
use recomp_core::reader::StreamError;
use recomp_game::tes::data_handler::{DataHandler, lookup_file_by_name};
use recomp_game::tes::paths::GamePaths;
use recomp_game::tes::settings::SettingCollection;
use recomp_game::tes::tes_file::{TesFileId, c_bytes};

use crate::path_setup::original_plugins_path_overflow;

/// IDA: 00B02C88 g_pluginsFileName (char*, image data -> 00A2FB94); its only reference is the 0040673A read.
pub const PLUGINS_FILE_NAME: &str = ".\\Plugins.txt";
/// IDA: 00404B6D the mode 00404B75 passes to fopen. `TextModeFile` is this mode.
pub const PLUGINS_OPEN_MODE: &str = "r";
/// IDA: 00A2FF74 aOblivionEsm, the 0040675D fallback name.
pub const FALLBACK_PLUGIN_NAME: &str = "Oblivion.esm";
/// IDA: 00406689..004066D1 the ten stack slots: &Setting sTestFile1..10:General at 00B02C90 + 8 * i (CRT initializers 009D8290..009D85F0, default 00A2F7EC "").
pub const TEST_FILE_SETTINGS: [u32; 10] = [
    0x00B0_2C90,
    0x00B0_2C98,
    0x00B0_2CA0,
    0x00B0_2CA8,
    0x00B0_2CB0,
    0x00B0_2CB8,
    0x00B0_2CC0,
    0x00B0_2CC8,
    0x00B0_2CD0,
    0x00B0_2CD8,
];
/// IDA: 00404B90 / 00404C19 the fgets count, the size of the char[512] line buffer at esp+0x110.
pub const PLUGIN_LINE_BUFFER: usize = 0x200;
/// IDA: N/A how many bytes of the line buffer a match keeps, as the P10-RUN-001 MATCH records dumped them.
pub const PLUGIN_BUFFER_HEAD: usize = 24;

/// IDA: N/A host trace of one 00404C07 call (the P10-RUN-001 MATCH record).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginListMatch {
    /// The looked-up line.
    pub line: Vec<u8>,
    pub file: TesFileId,
    /// TESFile +0x3DC before the call.
    pub flags_before: u32,
    /// The found byte (esp+0xB) before the call.
    pub found_before: bool,
    /// The start of the line buffer, with what earlier, longer lines left behind the terminator.
    pub buffer_head: [u8; PLUGIN_BUFFER_HEAD],
    /// How many leading buffer bytes `fgets` has written so far. Past them the original's stack bytes are uninitialized and this port's are zero.
    pub buffer_written: usize,
}

/// IDA: N/A the observable outcome of 00404B00: the return byte plus host trace counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginListRead {
    /// FileName as the original builds it (`plugins_file_name`).
    pub file_name: String,
    /// `original_plugins_path_overflow`, reported through the warning.
    pub path_overflow: bool,
    /// 00404B7F the stream is non-null.
    pub opened: bool,
    /// The `fgets` lines the loop handled.
    pub lines: u32,
    /// The 00404C07 TESFile_SetLoaded(1) calls.
    pub matched: u32,
    /// The return byte (al).
    pub found: bool,
    pub trace: Vec<PluginListMatch>,
}

/// IDA: N/A the observable outcome of the 00406687..00406773 window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginSelection {
    /// bl after 00406738.
    pub test_file_selected: bool,
    /// The 0040671E calls.
    pub test_file_lookups: u32,
    /// 0040674A 00404B00.
    pub list: PluginListRead,
    /// 00406757 reached: al == 0 and bl == 0.
    pub fallback_taken: bool,
    /// 0040676F ran.
    pub fallback_selected: bool,
}

// IDA: 00404B1E..00404B73 FileName = 00B3F178 + fileName, in the original's form. 00B3F178 ends with '\' (LocalAppData + "\Oblivion\"), or is ".\" when the bootstrap value is 0. A host root stays host text, so
// this string is for the report and the comparison with the run; `host_path::resolve` makes the path the host opens. Completion: 100%.
#[must_use]
pub fn plugins_file_name(paths: &GamePaths, file_name: &str) -> String {
    if paths.use_my_games {
        format!(
            "{}\\{file_name}",
            paths.local_data_directory.to_string_lossy()
        )
    } else {
        format!(".\\{file_name}")
    }
}

// IDA: N/A 00404B96 / 00404C1F fgets(Buffer, 0x200, stream) into the char[512]: a null return leaves the buffer as it was, otherwise the line and a NUL. Completion: 100%.
fn fgets_into(
    stream: &mut TextModeFile,
    buffer: &mut [u8; PLUGIN_LINE_BUFFER],
    written: &mut usize,
) -> bool {
    let Some(line) = stream.fgets(PLUGIN_LINE_BUFFER) else {
        return false;
    };
    buffer[..line.len()].copy_from_slice(&line);
    buffer[line.len()] = 0;
    *written = (*written).max(line.len() + 1);
    true
}

// IDA: 00404B00 Startup_SelectPluginsFromList(root, fileName) (char __stdcall, retn 8, GS cookie), called at 0040674A with 00B3F178 and [00B02C88]. FileName = root + fileName, inline byte copies with no bound;
// fopen(FileName, "r") in text mode; a null stream returns 0. A failing first fgets(Buffer, 0x200) closes the stream and returns 0. Per line: skip when Buffer[0] == '#' or strlen <= 1; a trailing '\n' becomes
// NUL; g_dataHandler is reloaded and 00447C50 looks the line up; a match gets 0044FAD0(1) and found = 1. At the end of the file it closes the stream and returns found. From the CRT text mode: CRLF reads as LF,
// a lone CR stays in the name, Ctrl-Z ends the read; blanks are not trimmed, a last line without '\n' is looked up as it is, a line of 511 or more characters is looked up in pieces, and duplicates set the flag
// again. A null DataHandler stops the host at the first looked-up line.
// Completion: 100% of the order. The original's overflow of FileName is reported, not reproduced: past 260 bytes it runs into the line buffer (from 773 bytes also over the GS cookie, ending the process at
// 00404C45), and such a path also exceeds MAX_PATH for its CreateFileA.
pub fn select_plugins_from_list(
    world: &mut World,
    paths: &GamePaths,
    file_name: &str,
    warning: &mut dyn FnMut(String),
) -> Result<PluginListRead, StreamError> {
    let mut result = PluginListRead {
        file_name: plugins_file_name(paths, file_name),
        // 00404B1E..00404B73 FileName.
        path_overflow: original_plugins_path_overflow(paths, file_name),
        ..PluginListRead::default()
    };
    if result.path_overflow {
        warning(format!(
            "Plugins.txt path exceeds the original char[260] FileName buffer of 00404B00 ({}); the host keeps the full path",
            result.file_name
        ));
    }
    // 00404B1C bl = 0; 00404B2A found (esp+0xB) = 0.
    let mut found = false;
    // 00404B75 fopen(FileName, "r"); 00404B7F a null stream returns bl (0). A directory fails here: Windows' fopen refuses one, and Linux's refuses at the read.
    let path = host_path::resolve(&paths.local_data_directory, file_name);
    let Ok(mut stream) = std::fs::File::open(path).and_then(TextModeFile::from_reader) else {
        return Ok(result);
    };
    result.opened = true;
    let mut buffer = [0_u8; PLUGIN_LINE_BUFFER];
    let mut written = 0;
    // 00404B96 the first fgets: null closes the stream (00404C30) and returns bl (0).
    if fgets_into(&mut stream, &mut buffer, &mut written) {
        // 00404BA6 bl = '#' for the loop.
        loop {
            result.lines += 1;
            // 00404BB0..00404BC9 the inline strlen, up to a NUL the line itself may hold.
            let length = c_bytes(&buffer).len();
            // 00404BCB Buffer[0] == '#', 00404BD4 length <= 1 (unsigned): skip.
            if buffer[0] != b'#' && length > 1 {
                // 00404BD9 a trailing '\n' becomes NUL (cl is 0 after the strlen loop).
                if buffer[length - 1] == b'\n' {
                    buffer[length - 1] = 0;
                }
                // 00404BEC g_dataHandler reloaded; 00404BFA 00447C50(Buffer).
                let line = c_bytes(&buffer);
                if let Some(id) = lookup_file_by_name(world.get_resource(), Some(line))? {
                    let mut data = world.resource_mut::<DataHandler>();
                    let file = data.file_store.get_mut(id)?;
                    result.trace.push(PluginListMatch {
                        line: line.to_vec(),
                        file: id,
                        flags_before: file.flags_3dc,
                        found_before: found,
                        buffer_head: buffer[..PLUGIN_BUFFER_HEAD].try_into().expect("24 bytes"),
                        buffer_written: written,
                    });
                    // 00404C07 0044FAD0(1); 00404C0C found = 1.
                    file.set_loaded(true);
                    found = true;
                    result.matched += 1;
                }
            }
            // 00404C1F the next fgets; null ends the loop.
            if !fgets_into(&mut stream, &mut buffer, &mut written) {
                break;
            }
        }
        // 00404C2B bl = found.
        result.found = found;
    }
    // 00404C30 fclose; 00404C40 al = bl. The stream holds no handle here.
    Ok(result)
}

// IDA: N/A 0044FAD0(1) on a looked-up file.
fn select_file(world: &mut World, id: TesFileId) -> Result<(), StreamError> {
    world
        .resource_mut::<DataHandler>()
        .file_store
        .get_mut(id)?
        .set_loaded(true);
    Ok(())
}

// IDA: 00406687..00406773 of 00406360 in original order: bl = 0; for edi = 0..9 the sTestFile slot (00B02C90 + 8 * edi) is skipped when the slot is null (never in the original, whose slots are static addresses;
// the host keeps it for a setting its store does not hold), its value char* is null or strlen(value) == 0; otherwise 00447C50(g_dataHandler, value) and, on a match, 0044FAD0(1) and bl = 1. Then
// 00404B00(00B3F178, [00B02C88]); its ecx is 00406360's stale ecx, and the callee is __stdcall. When al == 0 and bl == 0: 00447C50(g_dataHandler, "Oblivion.esm") and 0044FAD0(1) on a match. A null DataHandler
// stops the host at the first lookup, where the original faults.
// Completion: 100% of the order.
// TODO: [ ] Phases 11 and 12: 0044F3D0 DataHandler_LoadSelectedFiles reads the Loaded bit this window sets.
pub fn run_plugin_selection(
    world: &mut World,
    ini: &SettingCollection,
    paths: &GamePaths,
    warning: &mut dyn FnMut(String),
) -> Result<PluginSelection, StreamError> {
    let mut result = PluginSelection::default();
    // 00406687 bl = 0; 00406689..004066D1 the ten setting slots.
    let mut selected = false;
    for address in TEST_FILE_SETTINGS {
        // 004066E0 edx = slot; 004066E4 a null slot is skipped.
        let Some(slot) = ini.find_address(address) else {
            continue;
        };
        // 004066E8 a null value char* is skipped.
        let Some(text) = slot.text.as_deref() else {
            continue;
        };
        // 004066F7..0040670B strlen(value) == 0 is skipped.
        let value = c_bytes(text);
        if value.is_empty() {
            continue;
        }
        // 00406717 ecx = g_dataHandler; 0040671E 00447C50(value).
        result.test_file_lookups += 1;
        if let Some(id) = lookup_file_by_name(world.get_resource(), Some(value))? {
            // 0040672B 0044FAD0(1); 00406730 bl = 1.
            select_file(world, id)?;
            selected = true;
        }
    }
    result.test_file_selected = selected;
    // 0040673A..0040674A 00404B00(00B3F178, [00B02C88]).
    result.list = select_plugins_from_list(world, paths, PLUGINS_FILE_NAME, warning)?;
    // 0040674F al == 0 and 00406753 bl == 0: the Oblivion.esm fallback.
    if !result.list.found && !selected {
        result.fallback_taken = true;
        // 00406757 ecx = g_dataHandler; 00406762 00447C50("Oblivion.esm").
        let name = FALLBACK_PLUGIN_NAME.as_bytes();
        if let Some(id) = lookup_file_by_name(world.get_resource(), Some(name))? {
            // 0040676F 0044FAD0(1).
            select_file(world, id)?;
            result.fallback_selected = true;
        }
    }
    Ok(result)
}
