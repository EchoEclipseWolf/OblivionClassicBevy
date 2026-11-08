//! Ported from `tests/startup/phase04_tests.cpp`, with expected values unchanged. A value the C++ test took from an original
//! run lives in `fixtures/rows/phase04.rows`, whose every row `phase04_rows_match_their_evidence` checks against
//! `fixtures/evidence/`. `verify_window`, `verify_os_globals` and the FaceGen checks belong to the OS globals port.

use std::path::{Path, PathBuf};

use recomp_core::fixtures;

use super::super::device_caps::{
    CAPS_FIELDS, CapSource, DeviceCapsFixture, PinnedRendererPlatform,
};
use super::super::registration::{
    NiProcessDefaults, SingletonTail, initialize_default_image_converter,
    release_default_image_converter, set_device_image_converter_default,
};
use super::*;
use crate::tes::settings::{SettingCollections, register_static_settings};

fn bits(value: f32) -> u32 {
    value.to_bits()
}

fn evidence(tag: &str) -> Vec<u8> {
    let file = match tag {
        "a2" => "phase04-run001-2026-10-05.records.txt",
        "a3" => "phase04-run001-attempt3-2026-10-05.records.txt",
        "r2" => "phase04-run002-2026-10-06.records.txt",
        "ri" => "phase04-run001-2026-10-05.RendererInfo.txt",
        other => panic!("unknown evidence tag {other}"),
    };
    let path = fixtures::root().join("evidence").join(file);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"))
}

/// The 0xB00-byte NiDX9Renderer attempt 3 dumped at 0049851E.
fn renderer_dump() -> Vec<u8> {
    let text = String::from_utf8_lossy(&evidence("a3")).into_owned();
    let start = text.find("rmem=").expect("the 0049851E snapshot") + 5;
    let hex = &text[start..];
    let end = hex
        .find(|c: char| !c.is_ascii_hexdigit())
        .unwrap_or(hex.len());
    let bytes: Vec<u8> = (0..end / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    assert_eq!(bytes.len(), 0xB00, "the whole renderer object");
    bytes
}

fn dump_u32(dump: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(dump[offset..offset + 4].try_into().unwrap())
}

fn rows() -> Vec<Vec<String>> {
    fixtures::ledger_relaxed(
        "phase04",
        "P04-RUN-001|RDEFAULT #3",
        1,
        "where the table sits in the run's dump: the slot's value is in the column after it",
    );
    fixtures::ledger_relaxed(
        "phase04",
        "P04-RUN-001|EVID #3",
        1,
        "the name the row's evidence goes by, not a value",
    );
    fixtures::rows("phase04")
        .into_iter()
        // The file's other block, the window and FaceGen rows, is verbatim record lines with a space after the run ID.
        .filter(|row| row.starts_with("P04-RUN-") && row.as_bytes().get(11) == Some(&b'|'))
        .map(|row| row.split('|').map(str::to_owned).collect())
        .collect()
}

fn number(text: &str) -> u32 {
    u32::from_str_radix(text, 16).unwrap_or_else(|_| panic!("hex: {text}"))
}

/// An `EVID` row's value: `run|EVID|file|key|value|needle`.
fn evidence_value(key: &str) -> String {
    rows()
        .into_iter()
        .find(|row| row[1] == "EVID" && row[3] == key)
        .unwrap_or_else(|| panic!("no row {key}"))[4]
        .clone()
}

fn evidence_values(prefix: &str) -> Vec<(String, String)> {
    rows()
        .into_iter()
        .filter(|row| row[1] == "EVID" && row[3].starts_with(prefix))
        .map(|row| (row[3].clone(), row[4].clone()))
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Every row names evidence that says what it says, so a row cannot drift from the original run.
#[test]
fn phase04_rows_match_their_evidence() {
    let dump = renderer_dump();
    let rows = rows();
    assert!(!rows.is_empty());
    for row in &rows {
        assert!(
            row[0] == "P04-RUN-001" || row[0] == "P04-RUN-002",
            "a run ID: {row:?}"
        );
        match row[1].as_str() {
            "EVID" => assert!(
                contains(&evidence(&row[2]), row[5].as_bytes()),
                "{row:?} is not in its evidence"
            ),
            "RMEM32" => assert_eq!(
                dump_u32(&dump, number(&row[2]) as usize),
                number(&row[3]),
                "{row:?}"
            ),
            "RMEM8" => assert_eq!(
                u32::from(dump[number(&row[2]) as usize]),
                number(&row[3]),
                "{row:?}"
            ),
            "RTABLE" => {
                let base = number(&row[2]) as usize;
                let mask = (0..22).fold(0, |mask, case| {
                    mask | (u32::from(dump_u32(&dump, base + 4 * case) != 0) << case)
                });
                assert_eq!(mask, number(&row[3]), "{row:?}");
            }
            "RDEFAULT" => {
                let table = number(&row[3]) as usize;
                let case = number(&row[4]) as usize;
                assert_eq!(
                    dump_u32(&dump, number(&row[2]) as usize),
                    dump_u32(&dump, table + 4 * case),
                    "{row:?}"
                );
            }
            other => panic!("unknown row kind {other}"),
        }
    }
}

fn pinned_platform() -> PinnedRendererPlatform {
    PinnedRendererPlatform::new(DeviceCapsFixture::pinned())
}

fn pinned_settings() -> RendererSettings {
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    collections
        .ini
        .set_path(fixtures::root().join("config/Oblivion.ini"));
    assert!(collections.ini.open_ini());
    collections.ini.read_all();
    collections.ini.close();
    renderer_settings_from(&collections.ini)
}

fn pinned_inputs() -> RendererCreationInputs {
    RendererCreationInputs {
        main_window: Some(WindowHandle(0x0871_1BAC)),
        instance: 0x0040_0000,
        window_class: "Oblivion".to_owned(),
        my_games_root: PathBuf::from("/virtual/My Games/Oblivion"),
        adapter_formats: vec![D3DFMT_X8R8G8B8, D3DFMT_R5G6B5, D3DFMT_X1R5G5B5],
        settings: pinned_settings(),
    }
}

fn state_field(state: &RendererState, offset: usize) -> u32 {
    match offset {
        0x5BC => state.adapter_5bc,
        0x5C0 => state.device_type_5c0,
        0x5C4 => state.behavior_flags_5c4,
        0x5C8 => u32::from(state.software_vertex_processing_5c8),
        0x5C9 => u32::from(state.mixed_vertex_processing_5c9),
        0x5E0 => state.texture_caps_flags_5e0,
        0x6D8 => state.max_vertex_blend_matrices_6d8,
        0x6DC => state.max_streams_6dc,
        0x6E0 => state.pixel_shader_version_6e0,
        0x6E4 => state.vertex_shader_version_6e4,
        0x6E8 => u32::from(state.mipmap_cube_maps_6e8),
        0x6E9 => u32::from(state.dynamic_textures_6e9),
        0x874 => state.default_format_index_874,
        0x894 => state.max_render_targets_894,
        0x898 => u32::from(state.independent_bit_depths_898),
        0x899 => u32::from(state.mrt_post_pixel_shader_blending_899),
        0x914..=0x938 => state.shader_version_table_914[(offset - 0x914) / 4],
        0xA58 => state.create_width_a58,
        0xA5C => state.create_height_a5c,
        0xA60 => state.create_use_flags_a60,
        0xA6C => state.create_adapter_a6c,
        0xA70 => state.create_device_desc_a70 as u32,
        0xA74 => state.create_frame_buffer_format_a74 as u32,
        0xA78 => state.create_depth_stencil_format_a78 as u32,
        0xA7C => state.create_presentation_interval_a7c as u32,
        0xA80 => state.create_swap_effect_a80 as u32,
        0xA84 => state.create_frame_buffer_mode_a84,
        0xA88 => state.create_back_buffer_count_a88,
        0xA8C => state.create_refresh_rate_a8c,
        0xA90 => u32::from(state.non_standard_aspect_a90),
        other => panic!("no field at +{other:X}"),
    }
}

fn format_table_mask(table: &[Option<PixelFormat>; 22]) -> u32 {
    (0..22).fold(0, |mask, case| {
        mask | (u32::from(table[case].is_some()) << case)
    })
}

/// The renderer the pinned caps give, against the object the original had at 0049851E.
#[test]
fn pinned_caps_reproduce_the_renderer_dump() {
    let mut platform = pinned_platform();
    let created = initialize_renderer(&pinned_inputs(), &mut platform);
    assert!(created.succeeded, "{}", created.failure_message);
    let state = &created.state;
    let mut checked = 0;
    for row in rows() {
        match row[1].as_str() {
            "RMEM32" | "RMEM8" => {
                assert_eq!(
                    state_field(state, number(&row[2]) as usize),
                    number(&row[3]),
                    "{row:?}"
                );
                checked += 1;
            }
            "RTABLE" => {
                let table = match number(&row[2]) {
                    0x6F4 => 0,
                    0x74C => 1,
                    0x7A4 => 2,
                    0x7FC => 3,
                    other => panic!("no table at {other:X}"),
                };
                assert_eq!(
                    format_table_mask(&state.pixel_format_tables[table]),
                    number(&row[3]),
                    "{row:?}"
                );
                checked += 1;
            }
            "RDEFAULT" => {
                let index = (number(&row[2]) as usize - 0x854) / 4;
                assert_eq!(
                    state.default_formats_854[index],
                    Some(number(&row[4]) as usize),
                    "{row:?}"
                );
                checked += 1;
            }
            _ => {}
        }
    }
    assert!(checked >= 40, "only {checked} dump rows");
    // 00498591 runs after the dump: 007641A0's default there, 00B350DC's black now.
    assert_eq!(
        dump_u32(&renderer_dump(), 0x5D4),
        0xFF80_8080,
        "007641A0 default"
    );
    assert_eq!(state.background_color_5d4, 0xFF00_0000);
    // 0049838F..: the 00B34FA0 block the snapshot holds.
    assert_eq!(created.globals.present_interval, 1);
    assert_eq!(created.globals.multisample, 0);
    assert_eq!(created.globals.width, 0x780);
    assert_eq!(created.globals.height, 0x438);
}

/// Every D3DCAPS9 dword in the fixture is the one in the dump.
#[test]
fn device_caps_match_the_renderer_dump() {
    let dump = renderer_dump();
    let fixture = DeviceCapsFixture::pinned();
    for (index, (name, field)) in CAPS_FIELDS.iter().enumerate() {
        let mut caps = fixture.caps;
        assert_eq!(
            *field(&mut caps),
            dump_u32(&dump, 0x284 + 4 * index),
            "{name}"
        );
    }
    let recorded = |wanted| {
        CAPS_FIELDS
            .iter()
            .filter(|field| {
                fixture
                    .sources
                    .iter()
                    .any(|(name, source)| name == field.0 && *source == wanted)
            })
            .count()
    };
    assert_eq!(recorded(CapSource::Recorded), 76);
    assert_eq!(recorded(CapSource::Derived), 0);
    assert_eq!(recorded(CapSource::Assumed), 0);
}

/// The pinned caps through `initialize_renderer` give RendererInfo-run001.txt byte for byte: the text with LF ends, and the
/// file itself once the CRT's text mode has run over it.
#[test]
fn pinned_caps_reproduce_renderer_info() {
    let mut platform = pinned_platform();
    let inputs = pinned_inputs();
    let created = initialize_renderer(&inputs, &mut platform);
    assert!(created.succeeded, "{}", created.failure_message);
    assert!(created.renderer_info_written);
    assert_eq!(
        created.renderer_info_path,
        Path::new("/virtual/My Games/Oblivion/RendererInfo.txt")
    );
    let recorded = evidence("ri");
    let without_cr: Vec<u8> = recorded.iter().copied().filter(|&b| b != b'\r').collect();
    assert_eq!(
        String::from_utf8_lossy(&created.renderer_info_text),
        String::from_utf8_lossy(&without_cr)
    );
    assert_eq!(crt_text_mode_output(&created.renderer_info_text), recorded);
    assert_eq!(
        platform.written,
        [(
            created.renderer_info_path.clone(),
            created.renderer_info_text.clone()
        )]
    );
}

/// The device sequence the pinned caps give, against what P04-RUN-001 recorded.
#[test]
fn pinned_caps_reproduce_the_recorded_device_calls() {
    let mut platform = pinned_platform();
    let created = initialize_renderer(&pinned_inputs(), &mut platform);
    let calls = &created.device_calls;
    let first = |site: u32| {
        calls
            .iter()
            .find(|entry| entry.site == site)
            .unwrap_or_else(|| panic!("no call at {site:08X}"))
    };
    // 00498413 and the window adjustment.
    assert_eq!(
        first(0x0049_8413).call,
        DeviceCall::CreateWindow {
            ex_style: 0,
            style: 0x5000_0000,
            x: 0,
            y: 0,
            width: 0x780,
            height: 0x438
        }
    );
    assert_eq!(
        first(0x0049_8459).call,
        DeviceCall::AdjustWindowRect {
            rect: [0, 0, 0x780, 0x438],
            style: 0x14CA_0000,
            menu: false
        }
    );
    assert_eq!(
        first(0x0049_8488).call,
        DeviceCall::SetWindowPos {
            x: 0x31,
            y: 0x21,
            width: 0x786,
            height: 0x455,
            flags: 0x40
        }
    );
    // 0076A545: the d5 dump, with the handle left out.
    let window = created.child_window;
    assert_eq!(
        first(0x0076_A545).call,
        DeviceCall::CreateDevice {
            adapter: 0,
            device_type: 1,
            behavior_flags: 0x54,
            parameters: PresentParameters {
                back_buffer_width: 0x780,
                back_buffer_height: 0x438,
                back_buffer_format: 0x16,
                back_buffer_count: 1,
                multi_sample_type: 0,
                multi_sample_quality: 0,
                swap_effect: 1,
                device_window: window,
                windowed: true,
                enable_auto_depth_stencil: true,
                auto_depth_stencil_format: 0x4B,
                flags: 0,
                full_screen_refresh_rate: 0,
                presentation_interval: 0x8000_0000,
            },
        }
    );
    assert_eq!(created.creation.attempts, 1);
    assert_eq!(created.creation.result, D3D_OK);
    // 007607E3: the default texture.
    assert_eq!(
        first(0x0076_07E3).call,
        DeviceCall::CreateTexture {
            width: 2,
            height: 1,
            levels: 1,
            usage: 0,
            format: 0x17,
            pool: 1
        }
    );
    let log = platform.log.lock().unwrap();
    assert_eq!(log.unlocked[0], [0x00, 0x00, 0xFF, 0xFF]);
    drop(log);
    // 00763D55: the first eight CheckDeviceFormat rows the records' site cap kept, in order.
    let format_checks: Vec<u32> = calls
        .iter()
        .filter(|entry| entry.site == 0x0076_3D55)
        .map(|entry| match entry.call {
            DeviceCall::CheckDeviceFormat { format, .. } => format,
            _ => panic!("not a format check"),
        })
        .collect();
    assert_eq!(format_checks.len(), 4 * 20);
    let recorded: Vec<u32> = evidence_values("FORMAT_CHECK")
        .iter()
        .map(|(_, value)| number(value))
        .collect();
    assert_eq!(recorded.len(), 8);
    assert_eq!(format_checks[..8], recorded[..]);
    // The enumerator's order: GetAdapterCount, the adapter, Release; then the persistent interface.
    let sites: Vec<u32> = calls
        .iter()
        .map(|entry| entry.site)
        .filter(|site| matches!(site, 0x0076_3DFB | 0x0076_3E22 | 0x0076_3E50 | 0x0076_1E2F))
        .collect();
    assert_eq!(
        sites,
        [0x0076_3DFB, 0x0076_3E22, 0x0076_3E50, 0x0076_1E2F],
        "00763DE0 caches the creator and the system; 00761DF0 only calls the creator"
    );
    // 007755B1 walks 00B29944: 70 and 71 first, as recorded.
    let depth: Vec<u32> = calls
        .iter()
        .filter(|entry| entry.site == 0x0077_55B1)
        .take(2)
        .map(|entry| match entry.call {
            DeviceCall::CheckDeviceFormat { format, .. } => format,
            _ => panic!("not a format check"),
        })
        .collect();
    assert_eq!(depth, [0x46, 0x47]);
    // The uncached and cached render states the records hold.
    let render_states: Vec<(u32, u32, u32)> = calls
        .iter()
        .filter_map(|entry| match entry.call {
            DeviceCall::SetRenderState { state, value } => Some((entry.site, state, value)),
            _ => None,
        })
        .collect();
    assert_eq!(
        render_states.iter().filter(|c| c.0 == 0x0077_FE8D).count(),
        103
    );
    let cached: Vec<(u32, u32)> = render_states
        .iter()
        .filter(|c| c.0 == 0x0077_B027)
        .map(|c| (c.1, c.2))
        .collect();
    assert_eq!(cached, cached_render_state_rows());
    let probe_nvcpl = calls
        .iter()
        .find(|entry| entry.site == 0x0049_88FC)
        .expect("the driver is nv");
    assert_eq!(probe_nvcpl.result, 0, "LoadLibraryA(NVCPL.dll) returned 0");
}

fn cached_render_state_rows() -> Vec<(u32, u32)> {
    evidence_values("CACHED_RS")
        .iter()
        .map(|(_, value)| {
            let (state, set) = value.split_once(',').unwrap();
            (number(state), number(set))
        })
        .collect()
}

/// IDA: 00572DC0.
#[test]
fn verify_ui_state() {
    let mut globals = RendererUiGlobals::default();
    globals.records[1].value = 9;
    globals.records[2].flag = 1;
    let state = initialize_renderer_ui_state(&mut globals);
    assert_eq!(
        (state.unknown_00, state.unknown_04),
        (0, 0),
        "The 8-byte UI state must be zero"
    );
    assert_eq!(
        globals.store_trace,
        [4, 0xFFFF_FFF8, 0, 4, 0xFFFF_FFF8, 0, 4, 0xFFFF_FFF8, 0],
        "00572DD0..00572DD6 store order must be float, byte, dword"
    );
    for record in &globals.records {
        assert_eq!(record.flag, 0);
        assert_eq!(record.value, 0);
        assert_eq!(bits(record.scale), 0, "UI state records must be cleared");
    }
    assert_eq!(evidence_value("UI_STATE"), "0000000000000000");
}

/// IDA: 0040C280/0040C1B0/0040C2F0/0040C220 and 0040E86A..0040E88E. `register_winmain_renderer_callbacks` belongs to
/// `recomp_startup`'s WinMain port, which game code cannot reach, so its three stores (0040E865 byte, then the two
/// registrations in order, lost device first) are written out here.
#[test]
fn verify_callback_pairs() {
    let mut pairs = RendererCallbackPairs::default();
    let hint = true;
    pairs.add_lost_device(ORIGINAL_RETURN_TRUE_STUB, 0);
    pairs.add_reset(ORIGINAL_TRANSITION_CALLBACK, 0);
    assert!(hint, "00B3F709 must be set before the registrations");
    assert_eq!(
        ORIGINAL_RETURN_TRUE_STUB,
        number(&evidence_value("LOST_DEVICE_FUNCTION"))
    );
    assert_eq!(
        ORIGINAL_TRANSITION_CALLBACK,
        number(&evidence_value("RESET_FUNCTION"))
    );
    assert_eq!(pairs.lost_device_functions.slots, [0x0097_7C50]);
    assert_eq!(pairs.lost_device_functions.size, 1);
    assert_eq!(pairs.lost_device_functions.used, 1);
    assert_eq!(pairs.lost_device_data.slots, [0]);
    assert_eq!(
        pairs.lost_device_data.used, 0,
        "Lost-device pair differs from P04-RUN-001 (+0xAB8/+0xAC8)"
    );
    assert_eq!(pairs.reset_functions.slots, [0x0040_5440]);
    assert_eq!(pairs.reset_data.slots, [0]);
    assert_eq!(
        pairs.reset_data.used, 0,
        "Reset pair differs from P04-RUN-001 (+0xA98/+0xAA8)"
    );
    assert_eq!(pairs.add_lost_device(0x1111, 5), 1);
    assert_eq!(pairs.add_lost_device(0x2222, 6), 2);
    assert_eq!(pairs.lost_device_functions.slots.len(), 3);
    assert_eq!(
        pairs.lost_device_data.used, 2,
        "0040C280 must grow by the grow step and return the index"
    );
    assert!(pairs.remove_lost_device(0x1111));
    assert_eq!(pairs.lost_device_functions.size, 3);
    assert_eq!(pairs.lost_device_functions.slots[1], 0);
    assert_eq!(
        pairs.lost_device_functions.used, 2,
        "00405020 must clear a middle slot without shrinking"
    );
    assert!(pairs.remove_lost_device(0x2222));
    assert_eq!(
        pairs.lost_device_functions.size, 2,
        "00405020 must shrink when the last slot is removed"
    );
    assert!(
        !pairs.remove_reset(0x3333),
        "Removing an absent function must fail"
    );
    assert_eq!(
        pairs.add_lost_device(0x4444, 0),
        2,
        "The first free index is the size, not a hole"
    );
}

/// IDA: 0076BEF0 over the 22 007637D0 statics (cases 0..5, 8, 9 confirmed by the P04-RUN-001 CheckDeviceFormat arguments).
#[test]
fn verify_pixel_formats() {
    let table = original_texture_pixel_formats();
    let expected = [
        D3DFMT_X1R5G5B5,
        D3DFMT_R5G6B5,
        D3DFMT_X8R8G8B8,
        D3DFMT_A1R5G5B5,
        D3DFMT_A4R4G4B4,
        D3DFMT_A8R8G8B8,
        D3DFMT_UNKNOWN,
        D3DFMT_UNKNOWN,
        D3DFMT_DXT1,
        D3DFMT_DXT3,
        D3DFMT_DXT5,
        D3DFMT_V8U8,
        D3DFMT_L6V5U5,
        D3DFMT_X8L8V8U8,
        D3DFMT_L8,
        D3DFMT_A8,
        D3DFMT_R16F,
        D3DFMT_G16R16F,
        D3DFMT_A16B16G16R16F,
        D3DFMT_R32F,
        D3DFMT_G32R32F,
        D3DFMT_A32B32G32R32F,
    ];
    for (i, format) in table.iter().enumerate() {
        assert_eq!(
            pixel_format_to_d3d(format),
            expected[i],
            "0076BEF0 mapping differs for 007637D0 case {i}"
        );
    }
    let recorded = evidence_values("FORMAT_CHECK");
    let recorded_cases = [0, 1, 2, 3, 4, 5, 8, 9];
    for ((_, value), case) in recorded.iter().zip(recorded_cases) {
        assert_eq!(expected[case], number(value), "case {case}");
    }
    assert_eq!(pixel_format_component_mask(&table[1], 0), 0xF800);
    assert_eq!(pixel_format_component_mask(&table[1], 2), 0x1F);
    assert_eq!(pixel_format_component_mask(&table[2], 0), 0xFF_0000);
    assert_eq!(
        pixel_format_component_mask(&table[2], 3),
        0,
        "00700B60 masks differ"
    );
    let mut hinted = table[0];
    hinted.renderer_hint_0c = D3DFMT_A8;
    assert_eq!(
        pixel_format_to_d3d(&hinted),
        D3DFMT_A8,
        "A renderer hint must win"
    );
    assert_eq!(
        pixel_format_to_d3d(&original_rgb24_pixel_format()),
        D3DFMT_R8G8B8,
        "00B25E48 must map to R8G8B8"
    );
}

/// IDA: 0076C23B..0076C299, read from the exact executable: not in the C++ test, whose port names 0x50 wrongly.
#[test]
fn depth_formats_take_the_original_values() {
    let d16 = super::super::pixel_data::d3d_pixel_format_static(0x00B2_6BC8).unwrap();
    assert_eq!(pixel_format_to_d3d(&d16), 80);
    let d15s1 = super::super::pixel_data::d3d_pixel_format_static(0x00B2_6C58).unwrap();
    assert_eq!(pixel_format_to_d3d(&d15s1), 73);
    let d32 = super::super::pixel_data::d3d_pixel_format_static(0x00B2_6C10).unwrap();
    assert_eq!(pixel_format_to_d3d(&d32), 71);
    let d24s8 = super::super::pixel_data::d3d_pixel_format_static(0x00B2_6CA0).unwrap();
    assert_eq!(pixel_format_to_d3d(&d24s8), 75);
    let d24x8 = super::super::pixel_data::d3d_pixel_format_static(0x00B2_6D30).unwrap();
    assert_eq!(pixel_format_to_d3d(&d24x8), 77);
    let d24x4s4 = super::super::pixel_data::d3d_pixel_format_static(0x00B2_6D78).unwrap();
    assert_eq!(pixel_format_to_d3d(&d24x4s4), 79);
}

/// IDA: 007623D0, 00761CE0, 00761DA0.
#[test]
fn verify_device_helpers() {
    assert_eq!(expand_device_type(0), Some((D3DDEVTYPE_HAL, 0x50)));
    assert_eq!(expand_device_type(6), Some((D3DDEVTYPE_REF, 0x80)));
    assert_eq!(expand_device_type(7), None, "007623D0 table differs");
    assert_eq!(renderer_multisample_type(-1), D3DMULTISAMPLE_NONMASKABLE);
    assert_eq!(renderer_multisample_type(1), D3DMULTISAMPLE_NONE);
    assert_eq!(renderer_multisample_type(4), 4);
    assert_eq!(
        renderer_multisample_type(17),
        D3DMULTISAMPLE_NONE,
        "00761CE0 differs"
    );
    assert_eq!(renderer_present_interval(0), D3DPRESENT_INTERVAL_IMMEDIATE);
    assert_eq!(renderer_present_interval(3), D3DPRESENT_INTERVAL_THREE);
    assert_eq!(
        renderer_present_interval(5),
        D3DPRESENT_INTERVAL_DEFAULT,
        "00761DA0 differs"
    );
}

/// IDA: 0077BB00 -> 00778F60 -> 0077B7B0 and 00776240 (P04-RUN-001 records).
#[test]
fn verify_render_states() {
    let mut defaults = original_render_state_defaults();
    let mut cache = RenderStateCache::default();
    let caps = D3dCaps9 {
        raster_caps: D3DPRASTERCAPS_FOGTABLE | D3DPRASTERCAPS_WFOG,
        texture_address_caps: D3DPTADDRESSCAPS_BORDER,
        ..D3dCaps9::default()
    };
    let calls = render_state_initialization_calls(&mut defaults, &mut cache, &caps, 8, true);
    assert_eq!(
        calls.len(),
        103 + 8 * (7 + 5) + 8 + 2,
        "Device state call count differs"
    );
    let render = |state, value| DeviceStateCall {
        kind: DeviceStateKind::RenderState,
        stage: 0,
        state,
        value,
    };
    assert_eq!(calls[0], render(7, 1));
    assert_eq!(calls[1], render(8, 3));
    assert_eq!(
        calls[102],
        render(0xD1, 1),
        "0077FE50 must set the 103 defaults with ZENABLE patched to 1"
    );
    for (i, (_, value)) in evidence_values("UNCACHED_RS").iter().enumerate() {
        let (state, set) = value.split_once(',').unwrap();
        assert_eq!(calls[i], render(number(state), number(set)), "row {i}");
    }
    let stage0 = [4, 2, 1, 2, 2, 1, 0];
    let stage1 = [1, 2, 1, 1, 2, 1, 0];
    for stage in 0..8u32 {
        let base = 103 + stage as usize * 12;
        for i in 0..7 {
            assert_eq!(
                calls[base + i],
                DeviceStateCall {
                    kind: DeviceStateKind::TextureStageState,
                    stage,
                    state: i as u32 + 1,
                    value: if stage == 0 { stage0[i] } else { stage1[i] },
                },
                "0077FF40 texture stage states differ"
            );
        }
        let samplers = [(1, 1), (2, 1), (5, 1), (6, 1), (7, 0)];
        for (i, (state, value)) in samplers.into_iter().enumerate() {
            assert_eq!(
                calls[base + 7 + i],
                DeviceStateCall {
                    kind: DeviceStateKind::SamplerState,
                    stage,
                    state,
                    value,
                },
                "0077FF40 sampler states differ"
            );
        }
    }
    let cached = cached_render_state_rows();
    assert_eq!(cached.len(), 10);
    for (i, (state, value)) in cached.into_iter().enumerate() {
        assert_eq!(
            calls[199 + i],
            render(state, value),
            "Cached SetRenderState changes differ from P04-RUN-001"
        );
    }
    assert_eq!(defaults.texture_stage_states[0].1, 1);
    assert_eq!(defaults.texture_stage_states[3].1, 1);
    assert_eq!(cache.fog_color_98, 0xFF00_0000);
    assert_eq!(
        cache.flags & 7,
        7,
        "0077FF40 write-back, fog colour or render-state flags differ"
    );
}

/// IDA: 00762640 with the 0076A8CB..0076A923 arguments.
#[test]
fn verify_camera() {
    let data = camera_data(
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        &CameraFrustum {
            left: -1.0,
            right: 1.0,
            top: 1.0,
            bottom: -1.0,
            near_plane: 0.1,
            far_plane: 1.0,
            orthographic: false,
        },
        &CameraPort {
            left: 0.0,
            right: 1.0,
            top: 1.0,
            bottom: 0.0,
        },
        false,
        1920,
        1080,
    );
    let v = &data.view;
    assert!(
        v._11 == 0.0
            && v._12 == 0.0
            && v._13 == 1.0
            && v._22 == 1.0
            && v._31 == 1.0
            && v._44 == 1.0
            && bits(v._41) == 0x8000_0000
            && bits(v._43) == 0x8000_0000,
        "View matrix differs (columns right/up/dir, negated zero translation)"
    );
    let p = &data.projection;
    assert!(
        p._11 == 1.0
            && p._22 == 1.0
            && bits(p._31) == 0x8000_0000
            && bits(p._32) == 0x8000_0000
            && bits(p._33) == 0x3F8E_38E4
            && bits(p._43) == 0xBDE3_8E3A
            && p._34 == 1.0
            && p._44 == 0.0,
        "Projection matrix differs from the 24-bit-precision result"
    );
    assert!(
        data.viewport.x == 0
            && data.viewport.y == 0
            && data.viewport.width == 1920
            && data.viewport.height == 1080
            && data.viewport.min_z == 0.0
            && data.viewport.max_z == 1.0
            && bits(data.far_minus_near) == 0x3F66_6666,
        "Viewport or near/far differs"
    );
}

fn path_inputs(
    version: (u32, u32),
    force_1x: bool,
    driver: Option<&[u8]>,
    slots: i32,
) -> RenderPathInputs {
    RenderPathInputs {
        pixel_shader_version: version.0,
        vertex_shader_version: version.1,
        force_1x,
        have_adapter: driver.is_some(),
        driver: driver.unwrap_or_default().to_vec(),
        ps20_instructions: slots,
    }
}

/// IDA: 007B45F0 and its readers.
#[test]
fn verify_render_path() {
    let mut nv = RendererGlobals::default();
    select_render_path(
        &mut nv,
        &path_inputs((0x300, 0x300), false, Some(b"nvldumd.dll"), 512),
    );
    assert_eq!(
        (
            nv.render_path,
            nv.ps2x_path,
            nv.shaders_30,
            nv.shader_model_class,
            nv.shader_flags
        ),
        (
            evidence_value("RENDER_PATH").parse().unwrap(),
            5,
            true,
            8,
            47
        ),
        "NVIDIA 512-slot path must be 5 (P04-RUN-001)"
    );
    let mut nv96 = RendererGlobals::default();
    select_render_path(
        &mut nv96,
        &path_inputs((0x300, 0x300), false, Some(b"nv4_disp.dll"), 96),
    );
    assert_eq!(
        (nv96.render_path, nv96.shaders_30, nv96.shader_model_class),
        (3, false, 6),
        "NVIDIA 96-slot path must be 3"
    );
    let mut ati = RendererGlobals::default();
    select_render_path(
        &mut ati,
        &path_inputs((0x200, 0x200), false, Some(b"ati2dvag.dll"), 512),
    );
    assert_eq!(
        (ati.render_path, ati.ps2x_path, ati.shaders_30),
        (6, 6, true),
        "ATI 512-slot path must be 6"
    );
    let mut other = RendererGlobals::default();
    select_render_path(
        &mut other,
        &path_inputs((0x200, 0x200), false, Some(b"igdumd32.dll"), 512),
    );
    assert_eq!(
        (other.render_path, other.ps2x_path, other.shaders_30),
        (2, 2, false),
        "Other vendors must use path 2"
    );
    let mut old = RendererGlobals::default();
    select_render_path(&mut old, &path_inputs((0x103, 0x101), false, None, 0));
    assert_eq!(
        (old.render_path, old.shader_flags, old.shader_model_class),
        (1, 15, 2),
        "Shader model 1.x must use path 1"
    );
    let mut forced = RendererGlobals::default();
    select_render_path(
        &mut forced,
        &path_inputs((0x300, 0x300), true, Some(b"nvldumd.dll"), 512),
    );
    assert_eq!(forced.render_path, 1, "bForce1XShaders must force path 1");
    let mut none = RendererGlobals::default();
    select_render_path(&mut none, &path_inputs((0, 0), true, None, 0));
    assert_eq!(
        none.render_path, 0,
        "No shaders keep path 0 even when forced"
    );
    nv.hdr = true;
    nv.fp16_filtering = true;
    assert_eq!(
        shader_package(&nv),
        evidence_value("SHADER_PACKAGE").parse::<i32>().unwrap(),
        "Shader package differs"
    );
    assert_eq!(render_path_name(5), "BSSM_SV_2_A");
    assert_eq!(vertex_shader_target(5), Some("vs_2_0"));
    assert_eq!(pixel_shader_target(&nv, true), Some("ps_2_a"));
}

/// IDA: 00498A7E..00498D4B against the file P04-RUN-001 captured. The C++ test spelled the expected text out; here it is that
/// file, less the CRs of the CRT's text mode.
#[test]
fn verify_renderer_info() {
    let mut g = RendererGlobals::default();
    select_render_path(
        &mut g,
        &path_inputs((0x300, 0x300), false, Some(b"nvldumd.dll"), 512),
    );
    g.fp16_blending = true;
    g.fp16_filtering = true;
    g.hdr = true;
    g.bloom = false;
    g.refraction = true;
    g.hair_20 = true;
    let s = RendererSettings {
        image_space_effects: true,
        water_shader: true,
        water_reflections: true,
        water_displacements: true,
        water_hi_res: true,
        ..RendererSettings::default()
    };
    let info = RendererInfoInputs {
        description: b"NVIDIA GeForce RTX 4090".to_vec(),
        driver: b"nvldumd.dll".to_vec(),
        ps_version: 0x300,
        vs_version: 0x300,
        ps20_instructions: 512,
        non_pow2: true,
        multisample_type: 0,
    };
    let expected: Vec<u8> = evidence("ri").into_iter().filter(|&b| b != b'\r').collect();
    assert_eq!(
        String::from_utf8_lossy(&renderer_info_text(&g, &s, &info)),
        String::from_utf8_lossy(&expected),
        "RendererInfo.txt text differs from the P04-RUN-001 file"
    );
    assert_eq!(
        renderer_failure_message("Unknown error creating the Gamebryo Renderer."),
        "Failed to initialize renderer.\nUnknown error creating the Gamebryo Renderer.",
        "004052F0 failure text differs"
    );
}

/// IDA: the 00498340 setting reads from the 03-E collection.
#[test]
fn verify_renderer_settings() {
    let folder = std::env::temp_dir().join(format!(
        "oblivion-recomp-phase04-settings-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&folder).unwrap();
    let ini_path = folder.join("Oblivion.ini");
    std::fs::write(
        &ini_path,
        "[Display]\niSize W=1920\niSize H=1080\nbFull Screen=0\niLocation X=49\n\
         iLocation Y=33\niPresentInterval=1\nfGamma=1.0000\n\
         [BlurShaderHDR]\nbDoHighDynamicRange=1\n",
    )
    .unwrap();
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    collections.ini.set_path(&ini_path);
    assert!(collections.ini.open_ini(), "INI collection must open");
    collections.ini.read_all();
    collections.ini.close();
    let s = renderer_settings_from(&collections.ini);
    std::fs::remove_dir_all(&folder).unwrap();
    assert!(
        s.width == 1920
            && s.height == 1080
            && !s.full_screen
            && s.location_x == 49
            && s.location_y == 33
            && s.present_interval == 1
            && s.high_dynamic_range
            && s.use_flags == 0x28
            && s.frame_buffer_format == 3
            && s.depth_stencil_format == 0x4B
            && s.back_buffer_count == 1
            && s.pixel_shader_version == 0x103
            && s.vertex_shader_version == 0x101,
        "Renderer settings must come from the setting collection and .data constants: {s:?}"
    );
}

/// IDA: 00761270 choice and conversion for the FaceGen pixel data.
#[test]
fn verify_source_texture_choice() {
    let mut state = RendererState::default();
    for i in 0..22 {
        if i == 6 || i == 7 {
            continue;
        }
        let mut format = original_texture_pixel_formats()[i];
        format.renderer_hint_0c = pixel_format_to_d3d(&format);
        format.extra_10 = i as u32;
        state.pixel_format_tables[0][i] = Some(format);
    }
    let grey = vec![0x80u8; 32 * 32 * 3];
    let rgb24 = original_rgb24_pixel_format();
    let x8 = prepare_renderer_source_texture(&state, &rgb24, 32, 32, &grey, [6, 3, 2], false, true);
    assert_eq!(x8.upload.table_case, Some(2));
    assert_eq!(x8.upload.d3d_format, D3DFMT_X8R8G8B8);
    assert_eq!(
        x8.upload.d3d_format,
        evidence_value("FACEGEN_FORMAT").parse::<u32>().unwrap()
    );
    assert_eq!(x8.texels.len(), 4096);
    assert_eq!(
        x8.texels[..4],
        [0x80, 0x80, 0x80, 0x00],
        "RGB24 must become X8R8G8B8 with a zero X byte (0071D960)"
    );
    assert_eq!(
        hex_of(&x8.texels[..16]),
        evidence_value("FACEGEN_TEXELS"),
        "P04-RUN-002 row0"
    );
    state.pixel_format_tables[0][2] = None;
    let a8 = prepare_renderer_source_texture(&state, &rgb24, 32, 32, &grey, [6, 3, 2], false, true);
    assert_eq!(a8.upload.table_case, Some(5));
    assert_eq!(a8.upload.d3d_format, D3DFMT_A8R8G8B8);
    assert_eq!(
        a8.texels[3], 0xFF,
        "Without X8R8G8B8 the next 00773BA0 candidate is A8R8G8B8 with alpha 0xFF"
    );
    state.pixel_format_tables[0][5] = None;
    let r5 = prepare_renderer_source_texture(&state, &rgb24, 32, 32, &grey, [6, 3, 2], false, true);
    assert_eq!(r5.upload.table_case, Some(1));
    assert_eq!(r5.upload.d3d_format, D3DFMT_R5G6B5);
    assert_eq!(r5.texels.len(), 2048);
    assert_eq!(
        r5.texels[..2],
        [0x10, 0x84],
        "The 16-bit fallback must use 0071D7F0 (0x8410 for grey 0x80)"
    );
    let mip = prepare_renderer_source_texture(&state, &rgb24, 32, 32, &grey, [6, 3, 2], true, true);
    assert!(!mip.upload.device_texture);
    assert_eq!(
        mip.upload.d3d_format, D3DFMT_UNKNOWN,
        "Mipmap generation stays carried (00B3F708 is cleared before FaceGen)"
    );
    let base =
        prepare_renderer_source_texture(&state, &rgb24, 32, 32, &grey, [6, 3, 2], false, false);
    assert_eq!(
        base.upload.table_case, None,
        "The Phase 03 base converter accepts only equal formats"
    );
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// IDA: the 04-E-7 failure chain without a usable D3D9: 004980D0 (00763DE0 returns null) stores "No D3D Device description
/// found.", 00498340 replaces it with "Unknown error creating the Gamebryo Renderer.", then 0049864C retries the enumerator
/// and leaves "GetAdapterDesc() failed."; 004052F0 shows "Failed to initialize renderer.\n%s". Each enumerator call loads
/// D3D9.DLL again while the creator stays unresolved.
#[test]
fn verify_renderer_failure_chain() {
    let inputs = RendererCreationInputs {
        settings: RendererSettings {
            width: 640,
            height: 480,
            ..RendererSettings::default()
        },
        ..RendererCreationInputs::default()
    };
    let mut platform = pinned_platform();
    platform.d3d9_library = false;
    let missing = initialize_renderer(&inputs, &mut platform);
    let count = |created: &RendererCreation, site: u32| {
        created
            .device_calls
            .iter()
            .filter(|entry| entry.site == site)
            .count()
    };
    assert!(!missing.succeeded);
    assert_eq!(count(&missing, 0x0076_3DFB), 2, "00763DE0 loads D3D9.DLL");
    assert!(missing.device_calls.iter().all(|entry| entry.call
        == DeviceCall::LoadLibrary {
            name: "D3D9.DLL".to_owned()
        }));
    assert_eq!(missing.child_window, None);
    assert_eq!(missing.globals.failure_text, "GetAdapterDesc() failed.");
    assert_eq!(
        missing.failure_message, "Failed to initialize renderer.\nGetAdapterDesc() failed.",
        "A missing D3D9.DLL must end in the 0049864C GetAdapterDesc failure"
    );
    let mut platform = pinned_platform();
    platform.direct3d_create9 = false;
    let no_interface = initialize_renderer(&inputs, &mut platform);
    assert!(!no_interface.succeeded);
    assert_eq!(
        count(&no_interface, 0x0076_3E10),
        1,
        "00763DE0 resolves Direct3DCreate9 once"
    );
    assert_eq!(
        count(&no_interface, 0x0076_3E22),
        2,
        "the resolved creator is called again"
    );
    assert_eq!(
        no_interface.globals.failure_text, "GetAdapterDesc() failed.",
        "A null IDirect3D9 must take the same failure chain (creator resolved once)"
    );
}

/// Not in the C++ test: 00498340's other failures, each through the seam. These are made-up inputs, not rows.
#[test]
fn renderer_failure_branches() {
    let run = |edit: &dyn Fn(&mut DeviceCapsFixture, &mut RendererCreationInputs)| {
        let mut fixture = DeviceCapsFixture::pinned();
        let mut inputs = pinned_inputs();
        edit(&mut fixture, &mut inputs);
        let mut platform = PinnedRendererPlatform::new(fixture);
        initialize_renderer(&inputs, &mut platform)
    };
    // 004980D0's texts are replaced at 004985F9 when the original goes on, and the adapter path ends each differently.
    let bad_adapter = run(&|_, inputs| inputs.settings.adapter = 1);
    assert_eq!(bad_adapter.globals.failure_text, "GetAdapterDesc() failed.");
    let no_windowed = run(&|fixture, _| fixture.check_device_type_result = E_FAIL);
    assert!(!no_windowed.succeeded);
    assert_eq!(
        no_windowed
            .device_calls
            .iter()
            .filter(|e| e.site == 0x0049_8413)
            .count(),
        0
    );
    assert_eq!(
        no_windowed.failure_message,
        "Failed to initialize renderer.\nUnknown error creating the Gamebryo Renderer.",
        "0049874B faults in the original here; the host keeps the 004985F9 text"
    );
    let no_tnl = run(&|fixture, _| fixture.caps.dev_caps &= !D3DDEVCAPS_HWTRANSFORMANDLIGHT);
    assert!(!no_tnl.succeeded);
    assert!(no_tnl.d3d9.is_none(), "004980D0 fails before 0076BD40");
    let old_shaders = run(&|fixture, _| fixture.caps.pixel_shader_version = 0xFFFF_0101);
    assert!(!old_shaders.succeeded);
    let no_modes = run(&|fixture, _| fixture.mode_counts.clear());
    assert!(!no_modes.succeeded);
    // 00762110: fewer than two stages.
    let one_stage = run(&|fixture, _| fixture.caps.max_texture_blend_stages = 1);
    assert!(!one_stage.succeeded);
    assert_eq!(
        one_stage.renderer_error,
        "Creation failed: Device does not support single-pass multi-texture"
    );
    assert!(one_stage.device.is_none(), "0076BD40 drops the device");
    assert_eq!(
        one_stage.failure_message,
        "Failed to initialize renderer.\nUnknown error creating the Gamebryo Renderer."
    );
    assert!(one_stage.renderer_info_written, "the tail still reports");
    assert!(
        !contains(&one_stage.renderer_info_text, b"Multisample Type"),
        "00497E10(null) faults in the original; the host stops before that line"
    );
    let no_caps = run(&|fixture, _| fixture.device_caps_result = E_FAIL);
    assert_eq!(
        no_caps.renderer_error,
        "Creation failed: Could not query device caps"
    );
    // 0076A5BE: CreateDevice failing walks device descriptions 0..3 and then stops without a text of its own.
    let no_device = run(&|fixture, _| fixture.create_device_result = E_FAIL);
    assert!(!no_device.succeeded);
    assert_eq!(no_device.creation.attempts, 4);
    assert_eq!(no_device.renderer_error, "");
    assert_eq!(
        no_device.creation.behavior_flags & 0x20,
        0x20,
        "description 3"
    );
    // A later description succeeding keeps going, with its behaviour flags.
    let mut platform = pinned_platform();
    platform.create_device_results = vec![E_FAIL];
    let second = initialize_renderer(&pinned_inputs(), &mut platform);
    assert!(second.succeeded);
    assert_eq!(second.creation.attempts, 2);
    assert_eq!(
        second.state.behavior_flags_5c4, 0x40,
        "description 1, without 0x20's multithread bit"
    );
    // 0076A58B: a failure that changed the back buffer count retries the same description once.
    let mut platform = pinned_platform();
    platform.create_device_results = vec![E_FAIL];
    platform.failed_back_buffer_count = Some(2);
    let retried = initialize_renderer(&pinned_inputs(), &mut platform);
    assert!(retried.succeeded);
    assert_eq!(retried.creation.attempts, 2);
    assert_eq!(retried.creation.parameters.back_buffer_count, 2);
    assert_eq!(
        retried.state.behavior_flags_5c4, 0x54,
        "still description 0"
    );
    assert!(retried.device_calls.iter().any(|e| e.site == 0x0076_A5B2));
    // fopen failing leaves the renderer up.
    let mut platform = pinned_platform();
    platform.file_opens = false;
    let unwritten = initialize_renderer(&pinned_inputs(), &mut platform);
    assert!(unwritten.succeeded);
    assert!(!unwritten.renderer_info_written);
    assert!(unwritten.renderer_info_text.is_empty());
}

/// Not in the C++ test: FaceGen's RGB24 upload through the boundary, against the P04-RUN-002 CreateTexture and UnlockRect.
#[test]
fn facegen_upload_matches_the_second_run() {
    let mut platform = pinned_platform();
    let mut created = initialize_renderer(&pinned_inputs(), &mut platform);
    let grey = vec![0x80u8; 32 * 32 * 3];
    let upload =
        created.upload_source_texture(&original_rgb24_pixel_format(), 32, 32, &grey, [6, 3, 2]);
    assert!(upload.device_texture);
    assert_eq!(upload.table_case, Some(2));
    assert_eq!(
        created
            .device_calls
            .iter()
            .rfind(|entry| entry.site == 0x0076_07E3)
            .unwrap()
            .call,
        DeviceCall::CreateTexture {
            width: 32,
            height: 32,
            levels: 1,
            usage: 0,
            format: evidence_value("FACEGEN_FORMAT").parse().unwrap(),
            pool: 1
        }
    );
    let last = created
        .device_calls
        .iter()
        .rfind(|entry| entry.site == 0x0076_09AE)
        .unwrap();
    assert_eq!(
        last.call,
        DeviceCall::UnlockRect {
            pitch: evidence_value("FACEGEN_PITCH").parse().unwrap(),
            bytes: 128 * 32
        }
    );
    let log = platform.log.lock().unwrap();
    assert_eq!(
        hex_of(&log.unlocked[1][..16]),
        evidence_value("FACEGEN_TEXELS")
    );
}

/// Not in the C++ test: 00774550 through the seam, and its no-format and replacement statuses.
#[test]
fn cube_texture_upload() {
    let mut platform = pinned_platform();
    let mut created = initialize_renderer(&pinned_inputs(), &mut platform);
    let rgba = super::super::pixel_data::d3d_pixel_format_static(0x00B2_5E00).unwrap();
    let mut data = construct_pixel_data(2, 2, &rgba, 1, 6);
    for (i, byte) in data.pixels_50.iter_mut().enumerate() {
        *byte = i as u8;
    }
    let mut out = TextureRendererData::default();
    let mut statistics = TextureMemoryStatistics::default();
    let status = created.create_cube_texture(&data, [6, 3, 2], &mut out, &mut statistics);
    assert_eq!(status, RendererTextureStatus::Created);
    assert_eq!(out.original_vtable, 0x00A8_9F80);
    assert_eq!(out.table_case, Some(5));
    assert_eq!(out.d3d_format, D3DFMT_A8R8G8B8);
    assert_eq!(out.levels_5c, 1);
    assert_eq!(out.pixel_bytes_68, 16 * 6);
    assert_eq!(out.texture_bytes_60, 4 * 2 * 2 * 6);
    assert_eq!(statistics.cube_bytes, 96);
    assert_eq!(statistics.cube_padding, 0x1000 - 96);
    let log = platform.log.lock().unwrap();
    // After the default texture's upload, six faces; face 1 starts at its stride, and R and B swap into A8R8G8B8.
    assert_eq!(log.unlocked.len(), 7);
    assert_eq!(log.unlocked[2][..4], [18, 17, 16, 19]);
    drop(log);

    let not_square = construct_pixel_data(4, 2, &rgba, 1, 6);
    assert_eq!(
        created.create_cube_texture(&not_square, [6, 3, 2], &mut out, &mut statistics),
        RendererTextureStatus::ReplacementTexture
    );
    created.state.pixel_format_tables[2] = [None; 22];
    assert_eq!(
        created.create_cube_texture(&data, [6, 3, 2], &mut out, &mut statistics),
        RendererTextureStatus::NoCubeFormats
    );
}

/// IDA: 00498614..00498647 -> 0071B290 and its 0071B280 reader.
#[test]
fn verify_image_converter_default() {
    let mut world = World::new();
    initialize_default_image_converter(&mut world);
    let defaults = |world: &World| *world.resource::<NiProcessDefaults>();
    assert!(
        defaults(&world).image_converter_b3fd28.is_some()
            && !defaults(&world).device_image_converter_default,
        "0071B190 must leave the base converter as the default"
    );
    set_device_image_converter_default(&mut world);
    assert!(
        defaults(&world).image_converter_b3fd28.is_none()
            && defaults(&world).device_image_converter_default,
        "0071B290 must drop the base converter for the device converter"
    );
    release_default_image_converter(&mut world);
    assert!(
        !defaults(&world).device_image_converter_default,
        "0071B240 must clear the default converter"
    );
    let mut platform = pinned_platform();
    let created = initialize_renderer(&pinned_inputs(), &mut platform);
    assert!(
        created.device_image_converter_default,
        "00498647 ran (P04-RUN-002)"
    );
}

/// IDA: 00764630 -> 007013A0 with the default 2D format (P04-RUN-001: case 1, R5G6B5, after 007637D0 stored the D3D format
/// and the case).
#[test]
fn verify_default_pixel_format() {
    let mut format = original_texture_pixel_formats()[1];
    format.renderer_hint_0c = pixel_format_to_d3d(&format);
    format.extra_10 = 1;
    let expected: [u32; 17] = [
        0x0000_1001,
        0,
        0,
        0x17,
        1,
        2,
        0,
        0x105,
        1,
        0,
        0x106,
        0,
        0,
        0x105,
        19,
        5,
        0x100,
    ];
    assert_eq!(
        pixel_format_words(&format),
        expected,
        "The NiPixelFormat image must match the 0x44-byte layout"
    );
    let image: String = expected
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        image,
        evidence_value("DEFAULT_PIXEL_FORMAT"),
        "P04-RUN-002 at 00B3F718"
    );

    let mut world = World::new();
    let slot = world.spawn_empty().id();
    let mut tail = SingletonTail::default();
    assert!(
        tail.set_object_slot(3, Some(slot)),
        "Slot 3 must accept an owner"
    );
    tail.set_default_pixel_format(&expected);
    assert_eq!(tail.descriptor_words_b3f718, expected);
    assert_eq!(
        tail.object_slots_b3f800[3], None,
        "007013A0 must replace 00B3F718 and clear the ten 00B3F800 slots"
    );
}
