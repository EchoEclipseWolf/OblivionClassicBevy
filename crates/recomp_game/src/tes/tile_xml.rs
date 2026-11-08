//! Phase 07: the original TileXML pipeline (D2): buffer load and normalization, include expansion, the token scanner
//! and its template folding,
//! tile construction, trait and action application and Tile_ReadXML. Evidence: evidence/phase07-window-2026-10-06. The
//! C++ DOM path
//! (`src/ui/xml/tile_xml.cpp`) is for the menus Phase 21 owns and is not here.

use recomp_core::crt::{scan_f32, stricmp};
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::tile::{InterfaceCx, MenuId, TileId, TileKind, lift_text};
use super::tile_values::{
    TRAIT_CLASS, TRAIT_FILENAME, TRAIT_STRING, tile_get_or_create_value,
    tile_get_or_register_token, tile_initialize, tile_lookup_token, tile_resolve_source,
    tile_update_scene_graph, tile_value_append_constant, tile_value_append_marker,
    tile_value_append_reference, tile_value_evaluate, tile_value_set_number, tile_value_set_string,
};

/// IDA: TileXMLTokenKind (TileTemplateItem +0x00). Completion: 100%.
pub mod token {
    pub const INVALID: i32 = -1;
    pub const OPEN_TAG: i32 = 0x0A;
    pub const CLOSE_TAG: i32 = 0x0F;
    pub const ACTION_BEGIN: i32 = 0x14;
    pub const ACTION_END: i32 = 0x19;
    pub const TRAIT_BEGIN: i32 = 0x1E;
    pub const TRAIT_END: i32 = 0x23;
    pub const TILE_BEGIN: i32 = 0x28;
    pub const TILE_END: i32 = 0x2D;
    pub const TRAIT_VALUE: i32 = 0x32;
    pub const CONSTANT_ACTION: i32 = 0x37;
    pub const REFERENCE_ACTION: i32 = 0x3C;
    pub const TEXT: i32 = 0xBB9;
    pub const NAME: i32 = 0xBBA;
    pub const SOURCE: i32 = 0xBBB;
    pub const SOURCE_TRAIT: i32 = 0xBBC;
}

/// IDA: TileTemplateItem (0x18): +0x00 kind, +0x04 float number, +0x08 text, +0x10 payload (an ID, or the created Tile*
/// after 00590330), +0x14
/// line. Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct TileTemplateItem {
    pub kind: i32,
    pub number: f32,
    pub text: Vec<u8>,
    pub id: u32,
    pub tile: Option<TileId>,
    pub line: u32,
}

/// IDA: TileTemplate (0x1C, no vtable): +0x00 name, +0x0C NiTList<TileTemplateItem*>, here a list in order. +0x08 the
/// owning context is the
/// host's `TileParseContext` argument. Completion: 100%.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileTemplate {
    pub name: Vec<u8>,
    pub items: Vec<TileTemplateItem>,
}

/// IDA: TileXMLParseContext (0x14): +0x00 root template ("main"), +0x04 singly linked named templates, +0x0C active
/// template, +0x10 owns byte.
/// Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct TileParseContext {
    pub root: TileTemplate,
    pub templates: Vec<TileTemplate>,
    /// +0x0C, as an index into `templates`.
    pub active: Option<usize>,
    pub owns_templates: bool,
}

/// IDA: TileXMLBuffer (8): +0x00 length, +0x04 data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TileXmlBuffer {
    pub data: Vec<u8>,
}

/// IDA: the cache key: the text after the last '\' (strstr loop on "\\"). Completion: 100%.
fn cache_key(path: &[u8]) -> Vec<u8> {
    path.rsplit(|&byte| byte == b'\\')
        .next()
        .unwrap_or(path)
        .to_vec()
}

/// IDA: __ftol2_sse truncation for the float-to-ID conversions. Completion: 100%.
fn to_id(value: f32) -> u32 {
    x87::ftol(value) as u32
}

/// IDA: the XML ERROR text: warning and 00B3B0A0. Completion: 100%.
fn xml_error(cx: &mut InterfaceCx, what: &str, path: &str, line: u32) {
    cx.host.warning(&format!(
        "XML ERROR: {what} -- in file '{path}' on line {line}."
    ));
    cx.engine.parse_failed = true;
}

/// IDA: 00584F10 TileXML_LoadNormalizedBuffer(path, sourceText). The host output grows instead of overrunning on the
/// 100000-character newline
/// insertion. Completion: 100%.
pub fn tile_xml_load_normalized(
    cx: &mut InterfaceCx,
    path: &str,
    source_text: Option<&[u8]>,
) -> Option<TileXmlBuffer> {
    let owned;
    let source: &[u8] = if let Some(source) = source_text {
        source
    } else {
        let Some(bytes) = cx.host.read_tile_file(path) else {
            cx.host
                .output_debug_string(&format!("XML : error opening XML file : {path}"));
            return None;
        };
        owned = bytes;
        &owned
    };
    let mut data: Vec<u8> = Vec::with_capacity(source.len() + 1);
    let mut last = 0u8;
    let mut second = 0u8;
    let mut third = 0u8;
    let mut in_comment = false;
    let mut seen_close = false;
    let mut break_line = false;
    let mut run = 0u32;
    for &c in source {
        if c == 0 {
            break;
        }
        if c == 9 || c == 10 || c == 13 {
            continue;
        }
        if in_comment {
            if second == b'-' && last == b'-' && c == b'>' {
                in_comment = false;
            }
            third = second;
            second = last;
            last = c;
            continue;
        }
        if c == b' ' {
            if last == b' ' || last == b'>' {
                continue;
            }
        } else if last == b' ' && (c == b'<' || c == b'>') {
            data.pop();
        } else if second == b' ' && last == b'/' && c == b'>' {
            data.pop();
            if let Some(back) = data.last_mut() {
                *back = b'/';
            }
        } else if third == b'<' && second == b'!' && last == b'-' && c == b'-' {
            last = 0;
            second = 0;
            third = 0;
            data.truncate(data.len().saturating_sub(3));
            in_comment = true;
            continue;
        }
        if run > 100_000 {
            if seen_close && c == b'>' {
                break_line = true;
            } else if last == b'<' && c == b'/' {
                seen_close = true;
            }
        }
        data.push(c);
        run += 1;
        third = second;
        second = last;
        last = c;
        if break_line {
            data.push(b'\n');
            seen_close = false;
            break_line = false;
            run = 0;
        }
    }
    Some(TileXmlBuffer { data })
}

/// IDA: 00585220 TileXML_GetOrLoadBuffer(path, cacheResult). Host: returns a copy. Completion: 100% of the lookup.
fn get_or_load_buffer(
    cx: &mut InterfaceCx,
    path: &str,
    cache_result: bool,
) -> Option<TileXmlBuffer> {
    let key = cache_key(path.as_bytes());
    if let Some((_, buffer)) = cx
        .engine
        .buffer_cache
        .iter()
        .find(|(name, _)| stricmp(name, &key) == 0)
    {
        return Some(buffer.clone());
    }
    let buffer = tile_xml_load_normalized(cx, path, None)?;
    if cache_result {
        cx.engine.buffer_cache.push((key, buffer.clone()));
    }
    Some(buffer)
}

/// IDA: 00589110 TileXML_ExpandIncludes(buffer, freeIncluded): an 8-byte upper-cased window finds "<INCLUDE"; the
/// quoted name (up to 259
/// characters) names Data\Menus\Prefabs\<name>, which replaces the tag; the scan resumes two past the old '>' offset
/// inside the new buffer (the
/// original's arithmetic). `None` when nothing was included. A malformed tag ends the scan with the current data. The
/// original dereferences a
/// missing prefab buffer; the host splices nothing (recorded). Completion: 100%.
pub fn tile_xml_expand_includes(
    cx: &mut InterfaceCx,
    source: &TileXmlBuffer,
) -> Option<TileXmlBuffer> {
    let mut data = source.data.clone();
    let mut length = data.len();
    if length == 0 {
        return None;
    }
    let mut included = false;
    let mut window = [0u8; 8];
    let mut index = 0;
    'scan: loop {
        window.copy_within(1..8, 0);
        let mut c = data[index];
        if (c as i8) > 96 {
            c = c.wrapping_sub(32);
        }
        window[7] = c;
        let shown = window.split(|&byte| byte == 0).next().unwrap_or(&window);
        if stricmp(shown, b"<INCLUDE") == 0 {
            included = true;
            let start = index - 7;
            if index >= length {
                break 'scan;
            }
            while data[index] != b'"' {
                index += 1;
                if index >= length {
                    break 'scan;
                }
            }
            let mut cursor = index + 1;
            if cursor >= length {
                break 'scan;
            }
            let mut name = Vec::new();
            while name.len() < 259 {
                let n = data[cursor];
                if (n as i8) <= 32 || n == b'"' {
                    break;
                }
                name.push(n);
                cursor += 1;
                if cursor >= length {
                    break 'scan;
                }
            }
            if cursor >= length || name.len() >= 260 {
                break 'scan;
            }
            while cursor < length {
                if data[cursor] == b'>' {
                    break;
                }
                cursor += 1;
            }
            index = cursor + 1;
            let path = format!("Data\\Menus\\Prefabs\\{}", lift_text(&name));
            let prefab = get_or_load_buffer(cx, &path, false);
            let mut merged = data[..start].to_vec();
            if let Some(prefab) = prefab {
                merged.extend_from_slice(&prefab.data);
            }
            if index < length {
                merged.extend_from_slice(&data[index..length]);
            }
            data = merged;
            length = data.len();
        }
        index += 1;
        if index >= length {
            if !included {
                return None;
            }
            break;
        }
    }
    data.truncate(length);
    Some(TileXmlBuffer { data })
}

/// IDA: 0058BC80 TileXMLParseContext_FindTemplate (_stricmp). Completion: 100%.
fn find_template(context: &TileParseContext, name: &[u8]) -> Option<usize> {
    context
        .templates
        .iter()
        .position(|candidate| stricmp(&candidate.name, name) == 0)
}

/// IDA: 0058D3BA..0058D462: texts made only of digits, '-' and '.' (or empty) are numbers. Completion: 100%.
fn numeric_text(text: &[u8]) -> bool {
    text.iter()
        .all(|&c| c.is_ascii_digit() || c == b'-' || c == b'.')
}

/// IDA: the sscanf("%f") of 0058D3BA: an unconverted text leaves the number. Completion: 100%.
fn scan_number(text: &[u8], number: &mut f32) {
    if let Some((scanned, _)) = scan_f32(text) {
        *number = scanned;
    }
}

/// IDA: N/A host helper. Completion: 100%.
fn current_template(context: &mut TileParseContext) -> &mut TileTemplate {
    match context.active {
        Some(index) => &mut context.templates[index],
        None => &mut context.root,
    }
}

/// IDA: 0058D2F0 TileTemplate_ProcessToken(kind, text, line, literal). Completion: 100%.
fn process_token(
    cx: &mut InterfaceCx,
    context: &mut TileParseContext,
    token_id: i32,
    text: &[u8],
    line: u32,
    literal: bool,
) {
    let mut number = 0.0f32;
    let looked_up = if literal {
        true
    } else {
        number = tile_lookup_token(cx, text) as i32 as f32;
        number == 0.0
    };
    if looked_up && text.first() == Some(&b'_') {
        number = tile_get_or_register_token(cx, text, -1) as i32 as f32;
    }
    let mut item = TileTemplateItem {
        kind: token_id,
        number,
        text: text.to_vec(),
        id: to_id(number),
        tile: None,
        line,
    };
    if numeric_text(text) {
        if number == -1.0 {
            scan_number(text, &mut number);
        }
        if number == 0.0 {
            scan_number(text, &mut number);
        }
        item.text.clear();
        item.number = number;
        item.id = to_id(number);
    }
    if token_id != 3002 {
        if token_id == token::CLOSE_TAG {
            let closing = number;
            if number == 999.0 && context.active.is_some() {
                context.active = None;
                return;
            }
            let current = current_template(context);
            let items = &mut current.items;
            let count = items.len();
            if count >= 1 {
                let previous_kind = items[count - 1].kind;
                if previous_kind == token::TEXT
                    && count >= 2
                    && (items[count - 2].kind == token::TRAIT_BEGIN
                        || items[count - 2].kind == token::ACTION_BEGIN)
                    && closing == items[count - 2].number
                {
                    let previous = items[count - 1].clone();
                    let second = &mut items[count - 2];
                    if (item.number < 4001.0 || item.number > 4097.0) && item.id < 10000 {
                        if item.number < 2001.0 || item.number > 2027.0 {
                            second.kind = token::INVALID;
                            cx.host.warning("Bad trait/action type in XML");
                        } else {
                            second.kind = token::CONSTANT_ACTION;
                        }
                    } else {
                        second.kind = token::TRAIT_VALUE;
                    }
                    second.id = to_id(second.number);
                    second.number = previous.number;
                    second.text = previous.text;
                    items.pop();
                    return;
                }
                if previous_kind == token::SOURCE_TRAIT
                    && count >= 3
                    && items[count - 2].kind == token::SOURCE
                    && items[count - 3].kind == token::ACTION_BEGIN
                    && items[count - 3].number == closing
                {
                    let previous_number = items[count - 1].number;
                    let second_text = items[count - 2].text.clone();
                    let third = &mut items[count - 3];
                    third.kind = token::REFERENCE_ACTION;
                    third.id = to_id(third.number);
                    third.number = previous_number;
                    third.text = second_text;
                    items.pop();
                    items.pop();
                    return;
                }
            }
        }
        let resolved = item.id as i32;
        if !(4001..=4097).contains(&resolved) && resolved < 10000 {
            if !(2001..=2027).contains(&resolved) {
                if item.kind == token::CLOSE_TAG && item.number >= 901.0 && item.number <= 907.0 {
                    item.kind = token::TILE_END;
                }
            } else if item.kind == token::OPEN_TAG {
                item.kind = token::ACTION_BEGIN;
            } else if item.kind == token::CLOSE_TAG {
                item.kind = token::ACTION_END;
            }
        } else if item.kind == token::OPEN_TAG {
            item.kind = token::TRAIT_BEGIN;
        } else if item.kind == token::CLOSE_TAG {
            item.kind = token::TRAIT_END;
        }
        current_template(context).items.push(item);
        return;
    }
    // The name attribute (3002).
    let current = current_template(context);
    let previous = current.items.last_mut();
    let Some(previous) = previous.filter(|previous| previous.kind == token::OPEN_TAG) else {
        item.kind = token::TRAIT_VALUE;
        item.id = 3002;
        current.items.push(item);
        return;
    };
    if previous.number != 999.0 {
        if previous.number >= 901.0 && previous.number <= 907.0 {
            previous.kind = token::TILE_BEGIN;
            previous.text = text.to_vec();
            return;
        }
        item.kind = token::TRAIT_VALUE;
        item.id = 3002;
        current.items.push(item);
        return;
    }
    // <template name="...">: drop the template's open-tag item from the current list and switch to (or create) the
    // named template.
    current.items.pop();
    if context.active.is_some() {
        cx.host
            .warning("Can't have nested template definitions in an XML file.");
    } else {
        context.active = find_template(context, text);
        if context.active.is_none() {
            // 0058CE30 TileXMLParseContext_AddTemplate (0058BCD0 + AppendNonNull).
            context.templates.push(TileTemplate {
                name: text.to_vec(),
                items: Vec::new(),
            });
            context.active = Some(context.templates.len() - 1);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scan {
    Outside,
    TagName,
    SelfClose,
    Text,
    Comment,
    AttributeName,
    AttributeValue,
}

/// IDA: 0058DBD0 TileXML_Parse(path, text, length). The original dereferences a null file buffer (missing file); the
/// host fails the parse instead
/// (recorded). Completion: 100% of the scanner states, folding calls and errors.
#[allow(clippy::too_many_lines)]
pub fn tile_xml_parse(
    cx: &mut InterfaceCx,
    path: &str,
    source_text: Option<&[u8]>,
) -> Option<TileParseContext> {
    let mut context = TileParseContext {
        root: TileTemplate {
            name: b"main".to_vec(),
            items: Vec::new(),
        },
        templates: Vec::new(),
        active: None,
        owns_templates: true,
    };
    let text = if let Some(source) = source_text {
        source.to_vec()
    } else {
        let Some(buffer) = get_or_load_buffer(cx, path, false) else {
            cx.host.warning(&format!(
                "XML ERROR: missing file '{path}' (the original dereferences a null buffer)"
            ));
            cx.engine.parse_failed = true;
            return None;
        };
        tile_xml_expand_includes(cx, &buffer).map_or(buffer.data, |expanded| expanded.data)
    };
    let mut scan = Scan::Outside;
    let mut complete = false;
    let mut quotes = false;
    let mut closing = false;
    let mut line = 1u32;
    let mut attribute = 0u32;
    let mut tag: Vec<u8> = Vec::new();
    let mut word: Vec<u8> = Vec::new();
    let length = text.len();
    let mut offset = 0usize;
    while offset < length {
        let c = text[offset];
        let self_closing = offset + 1 < length && c == b'/' && text[offset + 1] == b'>';
        if c == 10 {
            line += 1;
        }
        let opens_comment = |offset: usize| {
            offset + 3 < length
                && text[offset + 1] == b'!'
                && text[offset + 2] == b'-'
                && text[offset + 3] == b'-'
        };
        match scan {
            Scan::Outside => {
                if c == b'<' {
                    if opens_comment(offset) {
                        complete = false;
                        scan = Scan::Comment;
                    } else {
                        scan = Scan::TagName;
                        tag.clear();
                        closing = false;
                    }
                }
            }
            Scan::TagName => {
                if tag.is_empty() && c == b'/' {
                    closing = true;
                } else if c == b'>' || self_closing {
                    if !tag.is_empty() {
                        let kind = if closing {
                            token::CLOSE_TAG
                        } else {
                            token::OPEN_TAG
                        };
                        process_token(cx, &mut context, kind, &tag, line, false);
                    } else if !complete {
                        xml_error(cx, "Empty tag name", path, line);
                        return None;
                    }
                    scan = if c == b'/' {
                        Scan::SelfClose
                    } else {
                        Scan::Text
                    };
                    complete = false;
                } else if c > 0x20 {
                    if complete {
                        offset = offset.wrapping_sub(1);
                        scan = Scan::AttributeName;
                        complete = false;
                    } else {
                        tag.push(c);
                    }
                } else if !tag.is_empty() {
                    let kind = if closing {
                        token::CLOSE_TAG
                    } else {
                        token::OPEN_TAG
                    };
                    process_token(cx, &mut context, kind, &tag, line, false);
                    complete = true;
                }
            }
            Scan::Comment => {
                if offset + 2 < length
                    && c == b'-'
                    && text[offset + 1] == b'-'
                    && text[offset + 2] == b'>'
                {
                    offset += 2;
                    scan = Scan::Text;
                    complete = false;
                }
            }
            Scan::AttributeName => {
                if c == b'>' || self_closing {
                    xml_error(cx, "Attribute with no value", path, line);
                    return None;
                }
                if c == b'=' {
                    if word.is_empty() {
                        xml_error(cx, "Missing attribute name", path, line);
                        return None;
                    }
                    attribute = tile_lookup_token(cx, &word);
                    word.clear();
                    scan = Scan::AttributeValue;
                    complete = false;
                } else if c > 0x20 {
                    if complete {
                        xml_error(cx, "Unexpected word after attribute name", path, line);
                        return None;
                    }
                    append_word(cx, &mut word, c);
                } else if !word.is_empty() {
                    attribute = tile_lookup_token(cx, &word);
                    word.clear();
                    complete = true;
                }
            }
            Scan::AttributeValue => {
                if c == b'"' {
                    quotes = !quotes;
                } else if quotes {
                    append_word(cx, &mut word, c);
                } else if c == b'>' || self_closing {
                    if !word.is_empty() {
                        process_token(cx, &mut context, attribute as i32, &word, line, false);
                    } else if !complete {
                        xml_error(cx, "Missing attribute's value", path, line);
                        return None;
                    }
                    word.clear();
                    scan = if c == b'/' {
                        Scan::SelfClose
                    } else {
                        Scan::Text
                    };
                    complete = false;
                } else if c > 0x20 {
                    if complete {
                        offset = offset.wrapping_sub(1);
                        scan = Scan::AttributeName;
                        complete = false;
                    } else {
                        append_word(cx, &mut word, c);
                    }
                } else if !word.is_empty() {
                    process_token(cx, &mut context, attribute as i32, &word, line, false);
                    word.clear();
                    complete = true;
                }
            }
            Scan::SelfClose => {
                if c != b'>' {
                    xml_error(
                        cx,
                        "Close-tag marker '/' not followed by end-of-tag marker '>'",
                        path,
                        line,
                    );
                    return None;
                }
                process_token(cx, &mut context, token::CLOSE_TAG, &tag, line, false);
                scan = Scan::Text;
                tag.clear();
            }
            Scan::Text => {
                if c == b'<' {
                    if opens_comment(offset) {
                        complete = false;
                        scan = Scan::Comment;
                    } else {
                        if !word.is_empty() {
                            while word.last().is_some_and(|&back| back <= 0x20) {
                                word.pop();
                            }
                            if !word.is_empty() {
                                let literal = word[0] != b'&';
                                process_token(cx, &mut context, 0xBB9, &word, line, literal);
                            }
                        }
                        complete = false;
                        word.clear();
                        closing = false;
                        scan = Scan::TagName;
                        tag.clear();
                    }
                } else if self_closing {
                    xml_error(
                        cx,
                        "Unballanced close-tag marker pair '/>' found",
                        path,
                        line,
                    );
                    return None;
                } else if c == b'>' {
                    xml_error(cx, "Unballanced end-of-tag marker '>' found", path, line);
                    return None;
                } else if c > 0x20 || complete {
                    append_word(cx, &mut word, c);
                    complete = true;
                }
            }
        }
        offset = offset.wrapping_add(1);
    }
    Some(context)
}

/// IDA: the scanner's token append: 4096 bytes warn. Completion: 100%.
fn append_word(cx: &mut InterfaceCx, word: &mut Vec<u8>, c: u8) {
    word.push(c);
    if word.len() > 4096 {
        cx.host.warning("XML Read buffer too small!");
    }
}

/// IDA: 00584670 TileXML_ClearCachedFile(path, templateOwner). Completion: 100%.
pub fn tile_xml_clear_cached_file(
    cx: &mut InterfaceCx,
    path: &str,
    template_owner: Option<MenuId>,
) {
    let key = cache_key(path.as_bytes());
    cx.engine
        .buffer_cache
        .retain(|(name, _)| stricmp(name, &key) != 0);
    if let Some(index) = cx
        .engine
        .parse_cache
        .iter()
        .position(|(name, _)| stricmp(name, &key) == 0)
    {
        if let Some(owner) = template_owner
            && cx.engine.parse_cache[index].1.owns_templates
        {
            cx.engine.parse_cache[index].1.owns_templates = false;
            cx.engine.menu_mut(owner).owns_templates_1c = true;
        }
        cx.engine.parse_cache.remove(index);
    }
}

/// IDA: 005902A0 Tile_CreateByType: 0x385 TileRect (0x44), 0x386 TileImage (0x4C), 0x387 TileText (0x54), 0x388 Tile3D
/// (0x5C), 0x389 TileMenu (0x48),
/// 0x38B TileWindow. TileWindow is not bound (null, which aborts the build with the original warning;
/// carried). Completion: 100% for the
/// five kinds.
const fn kind_by_type(type_id: u32) -> Option<TileKind> {
    match type_id {
        0x385 => Some(TileKind::Rect),
        0x386 => Some(TileKind::Image),
        0x387 => Some(TileKind::Text),
        0x388 => Some(TileKind::Model),
        0x389 => Some(TileKind::Menu),
        _ => None,
    }
}

/// IDA: 00590330 Tile_BuildFromTemplate(parent, template). Completion: 100%.
fn build_from_template(
    cx: &mut InterfaceCx,
    mut parent: Option<TileId>,
    context: &mut TileParseContext,
) -> Result<Option<TileId>, StreamError> {
    let mut first = None;
    let mut items = std::mem::take(&mut current_template(context).items);
    let mut outcome = Ok(());
    for item in &mut items {
        if item.kind == token::TILE_BEGIN {
            let Some(kind) = kind_by_type(to_id(item.number)) else {
                cx.host
                    .warning("Unable to create tile. Aborting menu creation.");
                first = None;
                break;
            };
            let tile = match tile_initialize(cx, kind, parent, None, None) {
                Ok(tile) => tile,
                Err(error) => {
                    outcome = Err(error);
                    break;
                }
            };
            cx.engine.tile_mut(tile).name_08.clone_from(&item.text);
            if first.is_none() {
                first = Some(tile);
            }
            item.tile = Some(tile);
            parent = Some(tile);
        } else if item.kind == token::TILE_END {
            parent = parent.and_then(|tile| cx.engine.tile(tile).parent);
        }
    }
    current_template(context).items = items;
    outcome.map(|()| first)
}

/// IDA: 0058CF40 TileTemplate_ApplyTraitsAndActions. The unterminated byte gates evaluation between a tile's begin and
/// end, where every value except
/// the class (4002) is evaluated with force. Completion: 100% of the order.
fn apply_traits_and_actions(
    cx: &mut InterfaceCx,
    context: &mut TileParseContext,
) -> Result<(), StreamError> {
    let mut tile: Option<TileId> = None;
    let mut trait_id: u32 = 0;
    let items = std::mem::take(&mut current_template(context).items);
    for item in &items {
        match item.kind {
            token::TILE_BEGIN => {
                tile = item.tile;
                cx.engine.unterminated_tile = true;
            }
            token::TILE_END => {
                cx.engine.unterminated_tile = false;
                if let Some(id) = tile {
                    // 0058D0C1: the next value is read before the evaluation.
                    let mut next = cx.engine.tile(id).values_14.first().copied();
                    while let Some(current) = next {
                        let list = &cx.engine.tile(id).values_14;
                        let position = list.iter().position(|&value| value == current);
                        next = position.and_then(|index| list.get(index + 1).copied());
                        if cx.engine.value(current).trait_18 != TRAIT_CLASS {
                            tile_value_evaluate(cx, current, true)?;
                        }
                    }
                    tile = cx.engine.tile(id).parent;
                }
            }
            token::TRAIT_VALUE => {
                let Some(id) = tile else {
                    cx.host.warning("Trait defined outside of any tile.");
                    continue;
                };
                if (item.number == 0.0 && !item.text.is_empty())
                    || item.id == u32::from(TRAIT_STRING)
                    || item.id == u32::from(TRAIT_FILENAME)
                {
                    if item.id == 3002 {
                        cx.engine.tile_mut(id).name_08.clone_from(&item.text);
                    } else if item.id == u32::from(TRAIT_FILENAME) {
                        let value = tile_get_or_create_value(cx, id, TRAIT_FILENAME);
                        tile_value_set_string(cx, value, &item.text)?;
                        cx.engine.tile_mut(id).flags_2c |= 0x20;
                    } else {
                        let value = tile_get_or_create_value(cx, id, item.id as u16);
                        tile_value_set_string(cx, value, &item.text)?;
                    }
                } else {
                    let value = tile_get_or_create_value(cx, id, item.id as u16);
                    tile_value_set_number(cx, value, item.number)?;
                }
            }
            token::TRAIT_BEGIN => trait_id = to_id(item.number),
            token::TRAIT_END => trait_id = 0,
            token::CONSTANT_ACTION => {
                if trait_id == 0 {
                    cx.host.warning("Action defined outside of any trait.");
                    continue;
                }
                let id = tile.ok_or(StreamError("0058CF40: an action outside any tile"))?;
                let value = tile_get_or_create_value(cx, id, trait_id as u16);
                tile_value_append_constant(cx, value, item.number, item.id as u16)?;
            }
            token::REFERENCE_ACTION => {
                if trait_id == 0 {
                    cx.host.warning("Action link defined outside of any trait.");
                    continue;
                }
                if let Some(source) = tile_resolve_source(cx, tile, &item.text) {
                    let id = tile.ok_or(StreamError("0058CF40: an action outside any tile"))?;
                    let value = tile_get_or_create_value(cx, id, trait_id as u16);
                    tile_value_append_reference(
                        cx,
                        value,
                        source,
                        to_id(item.number),
                        item.id as u16,
                    )?;
                }
            }
            token::ACTION_BEGIN | token::ACTION_END => {
                if trait_id == 0 {
                    cx.host.warning(if item.kind == token::ACTION_BEGIN {
                        "Action begun outside of any trait."
                    } else {
                        "Action ended outside of any trait."
                    });
                    continue;
                }
                let id = tile.ok_or(StreamError("0058CF40: an action outside any tile"))?;
                let value = tile_get_or_create_value(cx, id, trait_id as u16);
                let marker = if item.kind == token::ACTION_BEGIN {
                    10
                } else {
                    15
                };
                tile_value_append_marker(cx, value, to_id(item.number), marker)?;
            }
            _ => {}
        }
    }
    current_template(context).items = items;
    Ok(())
}

/// IDA: 00585410 Menu_CreateChildTileFromTemplate(parent, name, null): the list walk stops on a match (_stricmp), but its `do..while` keeps the last template it looked at, and only a null one returns null. So with no match the list's last template is built. 005AE6D0 depends on it: it asks for "save_game_template", which is in the exe, while the shipped load_menu.xml names only "load_game_template", which isn't. Built and applied under a temporary context that doesn't own it. Completion: 100%.
pub fn tile_build_template(
    cx: &mut InterfaceCx,
    menu: super::tile::MenuId,
    parent: TileId,
    name: &[u8],
) -> Result<Option<TileId>, StreamError> {
    let templates = &cx.engine.menu(menu).templates_08;
    let Some(template) = templates
        .iter()
        .find(|template| stricmp(&template.name, name) == 0)
        .or_else(|| templates.last())
        .cloned()
    else {
        return Ok(None);
    };
    let mut context = TileParseContext {
        root: template,
        templates: Vec::new(),
        active: None,
        owns_templates: false,
    };
    let tile = build_from_template(cx, Some(parent), &mut context)?;
    apply_traits_and_actions(cx, &mut context)?;
    Ok(tile)
}

/// IDA: 00590420 Tile_ReadXML(parent, path). Returns the first created tile or null. The early failure exits keep the
/// original's skipped restore of
/// bDisableWarning:MESSAGES. Completion: 100% of the order.
pub fn tile_read_xml(
    cx: &mut InterfaceCx,
    parent: Option<TileId>,
    path: &str,
) -> Result<Option<TileId>, StreamError> {
    cx.engine.parse_failed = false;
    let saved_disable = cx.engine.disable_warning_messages;
    let saved_count = cx.engine.warning_count;
    cx.engine.disable_warning_messages = true;
    let parsed = tile_xml_parse(cx, path, None);
    let Some(mut context) = parsed.filter(|_| !cx.engine.parse_failed) else {
        return Ok(None);
    };
    let root = build_from_template(cx, parent, &mut context)?;
    if cx.engine.parse_failed {
        return Ok(None);
    }
    apply_traits_and_actions(cx, &mut context)?;
    if cx.engine.parse_failed {
        return Ok(None);
    }
    if cx.engine.unterminated_tile {
        cx.host
            .warning("A tile descriptor was not terminated... Check XML file.");
        cx.engine.unterminated_tile = false;
    }
    if let Some(menu) = root.and_then(|root| cx.engine.containing_menu(root)) {
        // 005904D7..00590503: Menu_AddTemplate prepends each named template.
        for named in std::mem::take(&mut context.templates) {
            let owner = cx.engine.menu_mut(menu);
            owner.templates_08.insert(0, named);
            owner.owns_templates_1c = true;
            context.owns_templates = false;
            if cx.engine.parse_failed {
                return Ok(None);
            }
        }
    }
    drop(context);
    if let Some(root) = root {
        tile_update_scene_graph(cx, root, false)?;
    }
    cx.engine.disable_warning_messages = saved_disable;
    if cx.engine.warning_count != saved_count {
        cx.host.warning(&format!(
            "Warnings were encountered in Tile::ReadFile for file '{path}'."
        ));
    }
    tile_xml_clear_cached_file(cx, path, None);
    cx.engine.parse_failed = false;
    Ok(root)
}
