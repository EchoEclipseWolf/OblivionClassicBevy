//! Phase 07: the FontInfo text layout the text tiles' vt+8 builder (00592390) runs for their width and height traits:
//! 00575610 (layout block), 00575B40 (token substitution, wrapping,
//! integer line metrics) and the 00576670 pen loop with the 00573F10 advance. Main thread, 24-bit x87: `f32`
//! arithmetic, __ftol2_sse truncation. Evidence:
//! evidence/phase07-window-2026-10-06 (layout-disasm).

use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::fonts::{FONT_GLYPH_BASE, FONT_GLYPH_STRIDE, FontInfo, font_data_float};
use super::game_settings::GameSettingCollection;
use super::tile::lift_text;

/// IDA: N/A host helper. Completion: 100%.
fn glyph_field(font: &FontInfo, glyph: u8, field: usize) -> f32 {
    font_data_float(
        &font.font_data_38,
        FONT_GLYPH_BASE + usize::from(glyph) * FONT_GLYPH_STRIDE + field,
    )
}

/// IDA: 00575F8E.. / 00576919..: 0x91 and 0x92 -> '\'', 0x93 and 0x94 -> '"'. Completion: 100%.
const fn map_quote(c: u8) -> u8 {
    match c {
        0x91 | 0x92 => 39,
        0x93 | 0x94 => 34,
        _ => c,
    }
}

/// IDA: 00575FCD..00575FDA: ftol(g+0x30 + g+0x24). Completion: 100%.
fn layout_advance(font: &FontInfo, glyph: u8) -> i32 {
    x87::ftol(glyph_field(font, glyph, 0x30) + glyph_field(font, glyph, 0x24))
}

/// IDA: 005761CF..00576207: `movsx eax, byte` sign-extends the wrap's previous byte, then `ecx = eax * 7`, `[ebp + ecx * 8 +
/// 0x158]` and `+0x128 + 0x24`: a byte of 0x80 or more indexes before the glyph table, into the font block's header. The
/// same offset here. An offset before the block is a host stop: the original reads whatever precedes its heap block. The
/// C++ differs: it indexes unsigned. Completion: 100%.
fn signed_layout_advance(font: &FontInfo, byte: u8) -> Result<i32, StreamError> {
    let offset = FONT_GLYPH_BASE as i32 + i32::from(byte as i8) * FONT_GLYPH_STRIDE as i32;
    let offset = usize::try_from(offset)
        .ok()
        .filter(|offset| offset + 0x34 <= font.font_data_38.len())
        .ok_or(StreamError(
            "005761E9: the wrap's previous byte indexes outside the font block",
        ))?;
    Ok(x87::ftol(
        font_data_float(&font.font_data_38, offset + 0x30)
            + font_data_float(&font.font_data_38, offset + 0x24),
    ))
}

/// IDA: 0067B1E0 the +0x20 line width list's append: a width of 0 returns at once (67b1e7), so a blank line takes no
/// entry and later lines shift down. The C++ differs: it keeps the zero. Completion: 100%.
fn push_line_width(widths: &mut Vec<i32>, width: i32) {
    if width != 0 {
        widths.push(width);
    }
}

/// IDA: 00575C82..00575E59 the first 00575B40 pass: "&name;" tokens. The name is the bytes after '&' up to ';', '\n',
/// the break character or the end (at most 0x7F; the last byte is
/// dropped, which removes the ';'). The GMST map (00B35574) result is compared with the 29 control settings 00B399D0,
/// then their string values (_mbscmp). A match is replaced by
/// 0057C240(index); any other '&' is copied. Completion: 100% of the scan and match order.
/// TODO: [ ] 0057C240 control key names (the menu input phase); a match is a host stop.
fn substitute_control_tokens(
    collection: &GameSettingCollection,
    text: &[u8],
    break_char: u8,
) -> Result<Vec<u8>, StreamError> {
    let mut out = Vec::with_capacity(text.len());
    let controls = collection.control_settings();
    for (index, &c) in text.iter().enumerate() {
        if c != b'&' {
            out.push(c);
            continue;
        }
        let mut name = Vec::new();
        let mut count = 0usize;
        if index + 1 < text.len() && text[index + 1] != 0 {
            while count < 0x7F {
                let current = text[index + count];
                if current == b';' || current == b'\n' || current == break_char {
                    break;
                }
                name.push(text.get(index + count + 1).copied().unwrap_or(0));
                count += 1;
                if index + count + 1 >= text.len() || text[index + count + 1] == 0 {
                    break;
                }
            }
            if count != 0 {
                name.truncate(count - 1);
            }
        }
        let definition = collection
            .find(&lift_text(&name))
            .map(|setting| setting.definition);
        let by_setting = definition.is_some_and(|definition| {
            controls
                .iter()
                .flatten()
                .any(|&control| std::ptr::eq(control, definition))
        });
        let by_value = controls
            .iter()
            .flatten()
            .any(|control| control.text.is_some_and(|value| value == &name[..]));
        if by_setting || by_value {
            return Err(StreamError(
                "00575DC4: 0057C240 control key names are carried",
            ));
        }
        out.push(c);
    }
    Ok(out)
}

/// IDA: the 00575610 + 00575B40 + 00576670 result: the wrapped text, width (max line pixels), height, line count.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextLayoutResult {
    pub text: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub lines: i32,
    pub measured_width: f32,
    /// One quad per byte of `text`, line breaks included, as 00573F10 fills the shape's data.
    pub glyphs: Vec<TextGlyph>,
}

/// IDA: 00573F10 FontInfo_AdvancePen's writes for one glyph: four vertices in the shape's x and z (y stays the pen's 0) and their UVs, in the data's order: top left, bottom left, top right, bottom right. Completion: 100%.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextGlyph {
    pub positions: [[f32; 2]; 4],
    pub uvs: [[f32; 2]; 4],
}

/// IDA: 00575610 + 00575B40 + the 00576670 measurement. The Phase 07 texts hold no control token (P07-RUN-001: no
/// 0057C240 call). Completion: 100% of the integer and float order.
/// TODO: [ ] The source +0x10 / +0x14 line window (005762D1): its only way in is 00576670's fifth argument, and the one
/// call of 00576670 in the exe, 00592714, pushes 0 (5926fb `push ebx`), so nothing reaches it.
#[allow(clippy::too_many_lines, clippy::assign_op_pattern)] // 00576919's operand order: the glyph first.
pub fn font_layout_text(
    font: &FontInfo,
    collection: &GameSettingCollection,
    text: &[u8],
    mut wrap_width: i32,
    max_lines_in: i32,
    break_char: u8,
    justify: i32,
) -> Result<TextLayoutResult, StreamError> {
    let mut result = TextLayoutResult::default();
    if font.font_data_38.len() < FONT_GLYPH_BASE + 256 * FONT_GLYPH_STRIDE {
        // The original dereferences the missing block.
        return Ok(result);
    }
    // 00575B77..00575B91.
    if wrap_width <= 0 {
        wrap_width = i32::MAX;
    }
    let max_lines = if max_lines_in <= 0 {
        i32::MAX
    } else {
        max_lines_in
    };
    let extra = if font.id_08 == 3 { 6 } else { 0 };
    let line_height = font_data_float(&font.font_data_38, 0);
    let mut height = x87::ftol(font_data_float(&font.font_data_38, 0x850));
    let mut lines = 1;
    let mut widest = 0;
    let mut x = 0;
    let mut break_position = 0usize;
    let mut space_x = 0;
    let mut after_space = 0;
    let mut hyphen = false;
    let mut line_widths: Vec<i32> = Vec::new();
    // Pass 1 (00575C82..00575E87).
    let source = substitute_control_tokens(collection, text, break_char)?;
    let mut out: Vec<u8> = Vec::with_capacity(source.len() + 8);
    let add_line = |height: i32| x87::ftol(line_height + extra as f32 + height as f32);
    for &c in &source {
        if c == 0 {
            break;
        }
        if c == 11 {
            continue;
        }
        if c == break_char {
            out.push(c);
            height = add_line(height);
            push_line_width(&mut line_widths, x);
            if widest <= x {
                widest = x;
            }
            x = 0;
            lines += 1;
            break_position = 0;
        } else if c == 9 {
            x = 75 * (x / 75 + 1);
            continue;
        } else {
            let mut glyph = map_quote(c);
            let advance = layout_advance(font, glyph);
            x += advance;
            if glyph == b' ' {
                space_x = x - advance;
                after_space = x;
                hyphen = false;
                break_position = out.len();
            } else if glyph == b'~' {
                space_x = x;
                after_space = x;
                hyphen = true;
                x -= advance;
                break_position = out.len();
            }
            let mut pushed = false;
            if x > wrap_width {
                let last;
                if break_position != 0 {
                    if !hyphen {
                        if break_position == out.len() {
                            glyph = break_char;
                        } else {
                            out[break_position] = break_char;
                        }
                        height = add_line(height);
                        push_line_width(&mut line_widths, space_x);
                        if widest <= space_x {
                            widest = space_x;
                        }
                        lines += 1;
                        x -= after_space;
                        break_position = 0;
                        if glyph != b'~' {
                            out.push(glyph);
                        }
                        pushed = true;
                        last = 0;
                    } else {
                        hyphen = false;
                        out.splice(break_position..break_position, *b"-\n");
                        height = x87::ftol(extra as f32 + line_height + height as f32);
                        x -= layout_advance(font, b'-');
                        push_line_width(&mut line_widths, x);
                        if widest <= x {
                            widest = x;
                        }
                        // 005760D0 reads the byte before the buffer (heap header); the host uses 0.
                        last = 0;
                    }
                } else {
                    let previous = out.last().copied().unwrap_or(0);
                    if let Some(back) = out.last_mut() {
                        *back = b'-';
                    }
                    out.push(b'\n');
                    out.push(previous);
                    height = x87::ftol(extra as f32 + line_height + height as f32);
                    x -= layout_advance(font, b'-');
                    push_line_width(&mut line_widths, x);
                    if widest <= x {
                        widest = x;
                    }
                    last = previous;
                }
                if !pushed {
                    lines += 1;
                    break_position = 0;
                    x = signed_layout_advance(font, last)? + layout_advance(font, glyph);
                }
            }
            if !pushed && glyph != b'~' {
                out.push(glyph);
            }
        }
        // check_max.
        if max_lines > 0 && lines > max_lines && !out.is_empty() {
            let mut end = out.len();
            while end > 0 && (end >= out.len() || out[end] != break_char) {
                end -= 1;
            }
            out.truncate(end);
            height = x87::ftol(height as f32 - (line_height + extra as f32));
            break;
        }
    }
    if out.is_empty() {
        // 00576317: an empty result becomes one space with the space metrics.
        out.push(b' ');
        lines = 1;
        height = x87::ftol(font_data_float(&font.font_data_38, 0x850));
        x = x87::ftol(font_data_float(&font.font_data_38, 0x84C));
    }
    push_line_width(&mut line_widths, x);
    if widest <= x {
        widest = x;
    }
    result.text = out;
    result.width = widest;
    result.height = height;
    result.lines = lines;
    // 00576670 pen loop: each line starts at its justification offset (4 right: -width, 2 center: -(width / 2)
    // truncated, 00573D20 gives -1 past the list, and index 0 is the inline head, 0 when empty); 00573F10 advances x by g+0x2C,
    // then by g+0x24 plus g+0x30 when g+0x24 > 0. The width becomes ftol(max(width, x - start)) after each glyph, where
    // start is line 0's offset.
    let line_offset = |line: usize| -> f32 {
        let past = if line == 0 { 0 } else { -1 };
        let stored = line_widths.get(line).copied().unwrap_or(past);
        match justify {
            4 => (-stored) as f32,
            2 => (-(stored / 2)) as f32,
            _ => 0.0,
        }
    };
    let mut line_index = 0;
    let mut pen = line_offset(0);
    let start = pen;
    let mut width = widest;
    // 00576670's `a10`, 1 at the only call (00592714): v45 = 0 - ((+0x2C - line) + (+0x2C - line) + extra), the same
    // sum `build_text` stores as the shape's translate.
    let mut lift = (font.top_extent_2c - line_height) * 2.0 + extra as f32;
    lift = 0.0 - lift;
    for &raw in &result.text {
        // 00576829: a break restarts the pen at the line's justification offset (0 for other justifications) and the
        // break glyph itself is still advanced (00576919).
        if raw == break_char {
            pen = if justify == 4 || justify == 2 {
                line_index += 1;
                line_offset(line_index)
            } else {
                0.0
            };
            // v45 - (extra + line height), each at 24 bits.
            lift -= extra as f32 + line_height;
        } else if raw == 9 {
            pen = 75.0 - x87::float_fmod(pen, 75.0) + pen;
        }
        let glyph = map_quote(raw);
        pen = glyph_field(font, glyph, 0x2C) + pen;
        let glyph_width = glyph_field(font, glyph, 0x24);
        // 00573F10: the four vertices and UVs, before the pen moves on.
        let top = glyph_field(font, glyph, 0x34) + lift;
        let bottom = top - glyph_field(font, glyph, 0x28);
        let right = glyph_width + pen;
        result.glyphs.push(TextGlyph {
            positions: [[pen, top], [pen, bottom], [right, top], [right, bottom]],
            uvs: [
                [
                    glyph_field(font, glyph, 0x04),
                    glyph_field(font, glyph, 0x08),
                ],
                [
                    glyph_field(font, glyph, 0x14),
                    glyph_field(font, glyph, 0x18),
                ],
                [
                    glyph_field(font, glyph, 0x0C),
                    glyph_field(font, glyph, 0x10),
                ],
                [
                    glyph_field(font, glyph, 0x1C),
                    glyph_field(font, glyph, 0x20),
                ],
            ],
        });
        let spacing = if glyph_width > 0.0 {
            glyph_field(font, glyph, 0x30)
        } else {
            0.0
        };
        pen = glyph_width + spacing + pen;
        let span = pen - start;
        let current = width as f32;
        width = x87::ftol(if current < span { span } else { current });
    }
    result.measured_width = width as f32;
    result.width = width;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tes::fonts::FONT_DATA_SIZE;

    fn zero_font() -> FontInfo {
        FontInfo {
            font_data_38: vec![0; FONT_DATA_SIZE],
            ..FontInfo::default()
        }
    }

    // The C++ verify_game_settings' last check: 00575B40 copies a non-control "&name;" token.
    #[test]
    fn a_non_control_token_is_copied() {
        let collection = GameSettingCollection::default();
        let layout = font_layout_text(
            &zero_font(),
            &collection,
            b"&sMagicEffectItemFor;",
            0,
            0,
            b'\n',
            1,
        )
        .unwrap();
        assert_eq!(layout.text, b"&sMagicEffectItemFor;");
    }

    #[test]
    fn a_control_token_is_a_host_stop() {
        let collection = GameSettingCollection::default();
        let result = font_layout_text(
            &zero_font(),
            &collection,
            b"&sUActnForward;",
            0,
            0,
            b'\n',
            1,
        );
        assert!(result.is_err());
    }

    // 00576317: an empty text becomes one space. A missing data block returns the default.
    #[test]
    fn empty_and_missing_paths() {
        let collection = GameSettingCollection::default();
        let layout = font_layout_text(&zero_font(), &collection, b"", 0, 0, b'\n', 1).unwrap();
        assert_eq!(layout.text, b" ");
        assert_eq!(layout.lines, 1);
        let missing =
            font_layout_text(&FontInfo::default(), &collection, b"x", 0, 0, b'\n', 1).unwrap();
        assert_eq!(missing, TextLayoutResult::default());
    }

    // 0067B1E0 skips a zero: "a\n\nb" has the widths [10, 10], so the second line takes the third line's offset and the
    // third is past the list (-1, halved to 0). Line by line at justify 2: -5, -5, 0, so the span reaches 25. With a kept
    // zero the offsets are -5, 0, -5 and the span 20.
    #[test]
    fn a_blank_line_takes_no_width_entry() {
        let collection = GameSettingCollection::default();
        let mut font = zero_font();
        for glyph in 0..256usize {
            let offset = FONT_GLYPH_BASE + glyph * FONT_GLYPH_STRIDE + 0x24;
            font.font_data_38[offset..offset + 4].copy_from_slice(&10.0f32.to_le_bytes());
        }
        let layout = font_layout_text(&font, &collection, b"a\n\nb", 0, 0, b'\n', 2).unwrap();
        assert_eq!(layout.width, 25);
    }

    // 005761CF sign-extends: byte 0xFF indexes the glyph before table entry 0, whose +0x30 lands at 0x120 of the block.
    // Unsigned, the same wrap reads table entry 0xFF and gives 20.
    #[test]
    fn a_wrap_indexes_the_previous_byte_signed() {
        let collection = GameSettingCollection::default();
        let mut font = zero_font();
        let entry = FONT_GLYPH_BASE + 0xFF * FONT_GLYPH_STRIDE + 0x24;
        font.font_data_38[entry..entry + 4].copy_from_slice(&10.0f32.to_le_bytes());
        font.font_data_38[0x120..0x124].copy_from_slice(&100.0f32.to_le_bytes());
        let layout = font_layout_text(&font, &collection, &[0xFF, 0xFF], 15, 0, b'\n', 1).unwrap();
        assert_eq!(layout.width, 110);
        // 0x80 is 0x128 - 128 * 0x38: before the block, so a host stop.
        assert!(font_layout_text(&font, &collection, &[0x80, 0x80], 0, 0, b'\n', 1).is_ok());
        let mut wide = zero_font();
        let entry = FONT_GLYPH_BASE + 0x80 * FONT_GLYPH_STRIDE + 0x24;
        wide.font_data_38[entry..entry + 4].copy_from_slice(&10.0f32.to_le_bytes());
        assert!(font_layout_text(&wide, &collection, &[0x80, 0x80], 15, 0, b'\n', 1).is_err());
    }

    #[test]
    fn wrap_breaks_at_the_last_space() {
        let collection = GameSettingCollection::default();
        let mut font = zero_font();
        for glyph in 0..256usize {
            let offset = FONT_GLYPH_BASE + glyph * FONT_GLYPH_STRIDE + 0x24;
            font.font_data_38[offset..offset + 4].copy_from_slice(&10.0f32.to_le_bytes());
        }
        font.font_data_38[0..4].copy_from_slice(&16.0f32.to_le_bytes());
        let layout = font_layout_text(&font, &collection, b"aa bb cc", 55, 0, b'\n', 1).unwrap();
        assert_eq!(layout.text, b"aa bb\ncc");
        assert_eq!(layout.lines, 2);
        assert_eq!(layout.height, 16);
        assert_eq!(layout.width, 50);
    }
}
