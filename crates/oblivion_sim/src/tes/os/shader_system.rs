//! The shader system globals the WinMain shader wiring (`crate::tes::winmain::winmain_shader`, Phase 05 row 05-B) fills, the
//! two parameter setters 007B4870/007B4830, the bodies of the two callbacks stored at 0040E8C1/0040E8E0 (D5) and the
//! 004E0CF0 form-type switch. 00403160 lives in `tes::os_globals`. `ShaderSystemGlobals` records the store order.
//! Runtime evidence: P05-RUN-001 (Phase05-Run001-Window-2026-10-06.md).

use bevy_ecs::prelude::{Entity, Resource};
use oblivion_core::x87::stored_float;

// IDA: one store of the window, in order: destination VA and width in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderGlobalStore {
    pub address: u32,
    pub width: u8,
}

// IDA: the globals 0040E898..0040EAA6, 007B4870 and 007B4830 write. Every
// one is unloaded .data (0 before the window; P05-RUN-001 WINDOW_START).
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct ShaderSystemGlobals {
    // 0040E8C1..0040E8E0.
    pub warning_callback: u32,   // 00B42E8C (identity 00405150)
    pub image_space_effects: u8, // 00B42F3E (byte copy of 00B06CC4)
    pub texture_palette: bool, // 00B42EBC (copy of 00B35300): the `BsTexturePalette` resource's presence
    pub game_time_callback: u32, // 00B42D78 (identity 00405290)
    // 007B4870 GetHitShader_ApplyParameters.
    pub hit_tex_offset: f32,     // 00B46128
    pub blocked_tex_offset: f32, // 00B4612C
    pub blur_amount: f32,        // 00B46130
    // 0040E905..0040E932.
    pub local_tree_mip_bias: f32, // 00B430A8
    pub lod_tree_mip_bias: f32,   // 00B430A4
    pub use_hard_drive_cache: u8, // 00B430AE
    pub display_lod_land: u8,     // 00B43077
    pub local_map_shader: u8,     // 00B42D40
    // Shared by both arms.
    pub specular_multiplier: f32, // 00B2C2BC g_blurShaderSpecularMultiplierRuntime
    pub emissive_multiplier: f32, // 00B2C2C0 g_blurShaderEmissiveMultiplierRuntime
    pub blend_type: i32,          // 00B2C1E4 g_blurShaderBlendTypeRuntime
    pub sun_brightness: f32,      // 00B43154
    pub sky_brightness: f32,      // 00B43158
    // HDR arm: the 0x40-byte block 00B431E8..00B43224 (exterior/interior).
    pub bright_clamp: f32,               // 00B431E8
    pub bright_clamp_interior: f32,      // 00B431EC
    pub bright_scale: f32,               // 00B431F0
    pub bright_scale_interior: f32,      // 00B431F4
    pub blur_radius_hdr: f32,            // 00B431F8
    pub blur_radius_hdr_interior: f32,   // 00B431FC
    pub eye_adapt_speed: f32,            // 00B43200
    pub eye_adapt_speed_interior: f32,   // 00B43204
    pub emissive_hdr_mult: f32,          // 00B43208
    pub emissive_hdr_mult_interior: f32, // 00B4320C
    pub upper_lum_clamp: f32,            // 00B43210
    pub upper_lum_clamp_interior: f32,   // 00B43214
    pub target_lum: f32,                 // 00B43218
    pub target_lum_interior: f32,        // 00B4321C
    pub blur_passes_hdr: i32,            // 00B43220
    pub blur_passes_hdr_interior: i32,   // 00B43224
    // Blur arm: 007B4830 BlurShader_ApplyParameters.
    pub blur_passes: i32,        // 00B2C1E8
    pub blur_radius: f32,        // 00B2C1EC
    pub alpha_add_interior: f32, // 00B2C2B0
    pub alpha_add_exterior: f32, // 00B2C2B4
    pub blur_texture_size: i32,  // 00B2C2B8
    // Host record of the stores in order (fixtures).
    pub store_trace: Vec<ShaderGlobalStore>,
}

/// IDA: N/A one store of a window: the value, its destination and the width the original wrote.
#[macro_export]
macro_rules! shader_store {
    ($globals:expr, $field:ident, $value:expr, $address:expr, $width:expr) => {{
        $globals.$field = $value;
        $globals
            .store_trace
            .push($crate::tes::shader_system::ShaderGlobalStore {
                address: $address,
                width: $width,
            });
    }};
}

// IDA: 007B4870 GetHitShader_ApplyParameters(float hitTexOffset, float
// blockedTexOffset, float blurAmount): three fstp in argument order. Completion: 100%.
pub fn get_hit_shader_apply_parameters(
    globals: &mut ShaderSystemGlobals,
    hit_tex_offset: f32,
    blocked_tex_offset: f32,
    blur_amount: f32,
) {
    // 007B4870..007B4888: fld [esp+4] / fstp hitTexOffset, then the other two.
    crate::shader_store!(globals, hit_tex_offset, hit_tex_offset, 0x00B4_6128, 4);
    crate::shader_store!(
        globals,
        blocked_tex_offset,
        blocked_tex_offset,
        0x00B4_612C,
        4
    );
    crate::shader_store!(globals, blur_amount, blur_amount, 0x00B4_6130, 4);
}

// IDA: 007B4830 BlurShader_ApplyParameters(int blendType, int blurPasses,
// float blurRadius, float alphaAddInterior, float alphaAddExterior, int
// blurTextureSize): stores radius, alphaAddInterior, blendType, blurPasses,
// alphaAddExterior, textureSize in that order and returns blendType. Completion: 100%.
pub fn blur_shader_apply_parameters(
    globals: &mut ShaderSystemGlobals,
    blend_type: i32,
    blur_passes: i32,
    blur_radius: f32,
    alpha_add_interior: f32,
    alpha_add_exterior: f32,
    blur_texture_size: i32,
) -> i32 {
    // 007B483C..007B4865 in store order; eax = blendType at 007B486B.
    crate::shader_store!(globals, blur_radius, blur_radius, 0x00B2_C1EC, 4);
    crate::shader_store!(
        globals,
        alpha_add_interior,
        alpha_add_interior,
        0x00B2_C2B0,
        4
    );
    crate::shader_store!(globals, blend_type, blend_type, 0x00B2_C1E4, 4);
    crate::shader_store!(globals, blur_passes, blur_passes, 0x00B2_C1E8, 4);
    crate::shader_store!(
        globals,
        alpha_add_exterior,
        alpha_add_exterior,
        0x00B2_C2B4,
        4
    );
    crate::shader_store!(
        globals,
        blur_texture_size,
        blur_texture_size,
        0x00B2_C2B8,
        4
    );
    blend_type
}

/// IDA: 0040517D getObjectPtrFromNiNode(node)->GetEditorName().
pub type EditorNameService<'a> = Box<dyn Fn(Entity) -> Option<Vec<u8>> + 'a>;
/// IDA: 004A7A60 DebugMessage_Warning's sink.
pub type WarningSink<'a> = Box<dyn FnMut(&[u8]) + 'a>;

/// IDA: see `shader_system_log_node_warning`: getObjectPtrFromNiNode(node)->GetEditorName(), and the sink.
#[derive(Default)]
pub struct ShaderWarningServices<'a> {
    pub editor_name: Option<EditorNameService<'a>>,
    pub warning: Option<WarningSink<'a>>,
}

// IDA: 00405150 ShaderSystem_LogNodeWarning(char* format, NiNode* node), the
// callback stored in 00B42E8C:
// - null node -> DebugMessage_Warning(format);
// - getObjectPtrFromNiNode(node) null, or its GetEditorName (vt+0xD4) null ->
//   DebugMessage_Warning(format);
// - otherwise strlen(format) + strlen(name) + 5 >= 0xFF calls the stripped
//   assert stub ReturnZero_SharedVirtual("Shader error string is too
//   long.", ".\\Misc\\Main.cpp", 0x475) (no effect), then sprintf(buffer,
//   "%s :: %s", name, format) into char[256] and an inline strcat of format
//   (the text appears twice), then DebugMessage_Warning(buffer).
// DebugMessage_Warning 004A7A60 is DebugLog_Dispatch(1, text, va) (host log;
// recorded substitution: the original passes the text as a printf format
// with no arguments). 100% of the branches; the reference lookup is pending.
// TODO: [ ] Phase 09+: getObjectPtrFromNiNode and TESObjectREFR GetEditorName
//       (no reference exists before TES_Constructor).
// TODO: [ ] Phase 09+: getObjectPtrFromNiNode and TESObjectREFR GetEditorName (no reference exists before
//       TES_Constructor).
pub fn shader_system_log_node_warning(
    format: &[u8],
    node: Option<Entity>,
    services: &mut ShaderWarningServices<'_>,
) -> Vec<u8> {
    // 00405164..00405176. 0040517D getObjectPtrFromNiNode, 00405199 GetEditorName (vt+0xD4).
    let name = node.and_then(|node| {
        services
            .editor_name
            .as_ref()
            .and_then(|editor_name| editor_name(node))
    });
    let text = if let Some(name) = name {
        // 004051AF..004051EA: the length check reaches only the stripped assert
        // stub ReturnZero_SharedVirtual; the host string cannot overflow the
        // original char[256] (recorded difference for texts of 251+ characters).
        // 004051FA..0040520A sprintf(buffer, "%s :: %s", name, format), then
        // 0040520F..00405245 the inline strcat(buffer, format).
        let mut text = name;
        text.extend_from_slice(b" :: ");
        text.extend_from_slice(format);
        text.extend_from_slice(format);
        text
    } else {
        format.to_vec()
    };
    // 004A7A60 DebugMessage_Warning -> 004941A0 DebugLog_Dispatch(1, ...). Host substitution: the original passes the
    // text as a printf format with no arguments.
    if let Some(warning) = services.warning.as_mut() {
        warning(&text);
    }
    text
}

// IDA: 00405290 GameTime_QueryByMode(bool frame, bool hours), the callback
// stored in 00B42D78:
// - frame: InterfaceManager_IsMenuMode() ? 0.0 : TimeInfo.frameTime (+0x0C);
// - else hours: TimeGlobals 00B332E0 gameHour (+0x0C TESGlobal, value +0x24;
//   12.0f when null, 00402BD0) * 60.0 * 60.0 rounded to float (fstp);
// - else TimeInfo +0x10 as an unsigned dword (fild, +4294967296.0f if
//   negative).
// InterfaceManager_IsMenuMode 00578F60 returns false while the singleton is
// null (it is created in Phase 07). 100% of the branches; the two later
// services are explicit inputs.
// TODO: [ ] Phase 07: InterfaceManager menu mode; Phase 09: TimeGlobals.
#[derive(Default)]
pub struct GameTimeServices<'a> {
    pub is_menu_mode: Option<Box<dyn Fn() -> bool + 'a>>, // 00578F60
    pub game_hour: Option<f32>,                           // TESGlobal value, None = no global
    pub frame_time: f32,                                  // 00B33E9C
    pub timer: u32,                                       // 00B33EA0
}

// IDA: 00405290; see `GameTimeServices`.
// TODO: [ ] Phase 07: InterfaceManager menu mode; Phase 09: TimeGlobals.
#[must_use]
pub fn game_time_query_by_mode(frame: bool, hours: bool, services: &GameTimeServices<'_>) -> f64 {
    // 00405290 cmp byte [esp+4], 0.
    if frame {
        // 00405297 InterfaceManager_IsMenuMode: fldz, else fld TimeInfo+0x0C.
        let menu = services
            .is_menu_mode
            .as_ref()
            .is_some_and(|is_menu_mode| is_menu_mode());
        return if menu {
            0.0
        } else {
            f64::from(services.frame_time)
        };
    }
    // 004052AA cmp byte [esp+8], 0.
    if hours {
        // 00402BD0 TimeGlobals_GetGameHour: gameHour->value or 12.0f
        // (00A2F918), then two fmul by 60.0 (00A2FCC8) under the post-
        // CreateDevice 24-bit x87 precision (P04-RUN-002 control word 007F),
        // fstp dword and fld: float arithmetic. Completion: 100%.
        let hour = services.game_hour.unwrap_or(12.0);
        let minutes = hour * 60.0;
        return f64::from(minutes * 60.0);
    }
    // 004052CE..004052DD: fild TimeInfo+0x10, +4294967296.0f (00A2FC78) when
    // negative; the fadd rounds to 24 bits under the 007F control word.
    let value = services.timer as i32;
    if value >= 0 {
        return f64::from(value);
    }
    f64::from(stored_float(f64::from(value) + 4_294_967_296.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tes::os_globals::{
        DIERR_NOTACQUIRED, InputFlushResult, OsCall, OsInputGlobals, OsTraceEntry, S_OK,
        ScriptedOsPlatform, discard_buffered_input_events,
    };
    use crate::tes::settings::{SettingCollections, register_static_settings};
    use oblivion_core::fixtures;
    use std::cell::RefCell;

    /// `verify_warning_callback`: 00405150 ShaderSystem_LogNodeWarning (D5).
    #[test]
    fn verify_warning_callback() {
        let sink = RefCell::new(Vec::<Vec<u8>>::new());
        let mut services = ShaderWarningServices {
            editor_name: None,
            warning: Some(Box::new(|text| sink.borrow_mut().push(text.to_vec()))),
        };
        assert_eq!(
            shader_system_log_node_warning(b"plain", None, &mut services),
            b"plain"
        );
        // Only the node's identity reaches the callbacks; it is never dereferenced.
        let node = Some(Entity::from_raw_u32(1).unwrap());
        assert_eq!(
            shader_system_log_node_warning(b"no ref", node, &mut services),
            b"no ref"
        );
        services.editor_name = Some(Box::new(|_| Some(b"RefName".to_vec())));
        assert_eq!(
            shader_system_log_node_warning(b"Bad shader", node, &mut services),
            b"RefName :: Bad shaderBad shader",
            "the named form must be sprintf plus the inline strcat"
        );
        drop(services);
        let sink = sink.into_inner();
        assert_eq!(
            sink.len(),
            3,
            "every branch must reach DebugMessage_Warning once"
        );
        assert_eq!(sink[2], b"RefName :: Bad shaderBad shader");
    }

    /// `verify_game_time_callback`: 00405290 GameTime_QueryByMode (D5).
    #[test]
    fn verify_game_time_callback() {
        let mut services = GameTimeServices {
            frame_time: 0.0166,
            timer: 1234,
            ..GameTimeServices::default()
        };
        assert_eq!(
            game_time_query_by_mode(true, false, &services),
            f64::from(0.0166f32)
        );
        services.is_menu_mode = Some(Box::new(|| true));
        assert_eq!(
            game_time_query_by_mode(true, true, &services),
            0.0,
            "menu mode must return 0.0"
        );
        assert_eq!(
            game_time_query_by_mode(false, true, &services),
            43200.0,
            "a null gameHour global uses 12.0 hours"
        );
        let hour = 7.3f32;
        services.game_hour = Some(hour);
        assert_eq!(
            game_time_query_by_mode(false, true, &services),
            f64::from((hour * 60.0) * 60.0),
            "hour mode must round each product to float"
        );
        assert_eq!(game_time_query_by_mode(false, false, &services), 1234.0);
        services.timer = 0xFFFF_FFFF;
        assert_eq!(
            game_time_query_by_mode(false, false, &services),
            4_294_967_296.0,
            "a negative dword must add 2^32 rounded to float"
        );
    }

    /// `verify_input_flush`: 0040EAB1..0040EAC1 and 00403160 (D6), through `tes::os_globals`' one port.
    #[test]
    fn verify_input_flush() {
        let mut platform = ScriptedOsPlatform::default();
        let none = discard_buffered_input_events(None, &mut platform);
        assert!(
            !none.called && !none.device,
            "a null input pointer must skip 00403160"
        );
        let mut settings = SettingCollections::default();
        register_static_settings(&mut settings);
        let mut input = OsInputGlobals::new(
            crate::tes::os_globals::HostInstance(1),
            &settings,
            &mut platform,
        )
        .unwrap();
        platform.trace.clear();
        let flushed = discard_buffered_input_events(Some(&input), &mut platform);
        assert_eq!(
            flushed,
            InputFlushResult {
                called: true,
                device: true,
                result: DIERR_NOTACQUIRED
            }
        );
        // GetDeviceData(20, NULL, &INFINITE, 0).
        assert_eq!(
            platform.trace,
            [OsTraceEntry {
                site: 0x0040_3183,
                call: OsCall::GetDeviceData {
                    device: ScriptedOsPlatform::KEYBOARD,
                    element_size: 20,
                    count: 0xFFFF_FFFF,
                    flags: 0,
                },
            }]
        );
        // P05-RUN-001's result, ignored by the original.
        let row = fixtures::rows("phase05")
            .into_iter()
            .find(|row| row.starts_with("SHD|P05-RUN-001|FLUSH|RESULT|"))
            .expect("the FLUSH row");
        assert_eq!(
            format!("{:08X}", flushed.result),
            row.split('|').nth(4).unwrap()
        );
        input.keyboard_0028 = None;
        platform.trace.clear();
        let no_device = discard_buffered_input_events(Some(&input), &mut platform);
        assert_eq!(
            no_device,
            InputFlushResult {
                called: true,
                device: false,
                result: S_OK
            },
            "a null +0x28 device must return without a call"
        );
        assert!(platform.trace.is_empty());
    }
}
