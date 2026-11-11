//! Phase 12b: TESRegion, as in the C++'s forms_region: its point-list areas, the TESRegionDataManager dispatch, and the five region data classes 004A4600 creates. A region data is an entity, since 00B3541C can outlive the region that made it: a chunk arriving before any RDAT loads into the data of the region before.
//!
//! Evidence: objdump of each address named here; P12-RUN-001's SAMPLE and DATA rows, which reach only the head of each list.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::form::{FormIdentity, OriginalVtable};
use super::form_arms::{FormClass, FormLoadCx, default_editor_id, reset_14_components};
use super::form_components::{FormRef, load_texture};
use super::forms_environment::{
    WeatherEntry, load_string, load_weather_list, with_weather_list_warnings,
};
use super::forms_misc::set_initialized;
use crate::bs::string::BsStringT;
use crate::tes::data_handler::{DataHandler, FormsById, TES_TEXTURE_VTABLE, TesTexture};
use crate::tes::data_load::initialize_form_from_current_record;
use crate::tes::form::construct_tes_form;
use crate::tes::tes_file::map_file_form_id;

/// IDA: the form type of TESRegion.
pub const FORM_TYPE_REGION: u8 = 0x2F;
/// IDA: the vtables 004A2C30, 004A4600's constructors and 004A59E0 store.
pub const TES_REGION_VTABLE: u32 = 0x00A3_FEE4;
pub const TES_REGION_DATA_WEATHER_VTABLE: u32 = 0x00A4_0140;
pub const TES_REGION_DATA_MAP_VTABLE: u32 = 0x00A4_00DC;
pub const TES_REGION_DATA_LANDSCAPE_VTABLE: u32 = 0x00A4_0054;
pub const TES_REGION_DATA_GRASS_VTABLE: u32 = 0x00A3_FFF0;
pub const TES_REGION_DATA_SOUND_VTABLE: u32 = 0x00A4_0110;
pub const TES_REGION_GRASS_OBJECT_VTABLE: u32 = 0x00A4_01B8;

// IDA: the form types 004A59E0 and 004473F0 test or cast to.
const FORM_TYPE_GRASS: u8 = 0x1D;
const FORM_TYPE_LAND_TEXTURE: u8 = 0x0E;
const FORM_TYPE_SOUND: u8 = 0x0A;

// IDA: the chunks 004A31C0 and 004A4890 dispatch on.
const EDID: u32 = 0x4449_4445;
const ICON: u32 = 0x4E4F_4349;
const WNAM: u32 = 0x4D41_4E57;
const RPLI: u32 = 0x494C_5052;
const RPLD: u32 = 0x444C_5052;
const RDAT: u32 = 0x5441_4452;
const RDOT: u32 = 0x544F_4452;
const RDWT: u32 = 0x5457_4452;
const RDMP: u32 = 0x504D_4452;
const RDGS: u32 = 0x5347_4452;
const RDLN: u32 = 0x4E4C_4452;
const RDMD: u32 = 0x444D_4452;
const RDSD: u32 = 0x4453_4452;
const RDOB: u32 = 0x424F_4452;
const RDOJ: u32 = 0x4A4F_4452;

/// IDA: the state of the load's region data: 00B3541C the current data, and DataHandler +0xCD8's +0x04 region.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionLoadGlobals {
    pub current_b3541c: Option<Entity>,
    pub manager_region_04: Option<Entity>,
}

/// IDA: a region point {float x, float y}. 004A6930 copies the two dwords raw.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TesRegionPoint {
    pub x: f32,
    pub y: f32,
}

/// IDA: 004A6DE0 TESRegionPointList(owns) (0x28): +0x00/+0x04 the list of points, prepended; +0x08 a block 004A76F0 frees (0 here); +0x0C owns; +0x10/+0x14 min x and y; +0x18/+0x1C max x and y; +0x20 the edge falloff (RPLI); +0x24 the point count.
#[derive(Clone, Debug, PartialEq)]
pub struct TesRegionPointList {
    pub points_00: Vec<TesRegionPoint>,
    pub field_08: u32,
    pub owns_0c: u8,
    pub min_x_10: f32,
    pub min_y_14: f32,
    pub max_x_18: f32,
    pub max_y_1c: f32,
    pub edge_falloff_20: u32,
    pub count_24: u32,
}

// IDA: 00A32048 and 00A3B888, read from the exe: 0x7F7FFFFF and 0xFF7FFFFF.
const BOUND_MAX: f32 = f32::MAX;
const BOUND_MIN: f32 = -f32::MAX;

impl TesRegionPointList {
    // IDA: 004A7130: +0x10 below FLT_MAX is it; otherwise the list's least x, by `fcompp` of the running value: only above takes the point. 004A71D0 the same on y.
    fn least(&self, cached: f32, axis: fn(&TesRegionPoint) -> f32) -> f32 {
        if cached < BOUND_MAX {
            return cached;
        }
        let mut best = BOUND_MAX;
        for point in &self.points_00 {
            if best > axis(point) {
                best = axis(point);
            }
        }
        best
    }

    // IDA: 004A7180: +0x18 above -FLT_MAX is it; otherwise the list's greatest x. 004A7220 the same on y.
    fn greatest(&self, cached: f32, axis: fn(&TesRegionPoint) -> f32) -> f32 {
        if cached > BOUND_MIN {
            return cached;
        }
        let mut best = BOUND_MIN;
        for point in &self.points_00 {
            if best < axis(point) {
                best = axis(point);
            }
        }
        best
    }

    /// IDA: 004A7330 TESRegionPointList_ContainsPoint(&point): under 3 points is false; outside the four bounds false; each edge, A to the next node's B with the last wrapping to the head: both x above the point, or one above with the point below 004A6AA0's crossing, and A's y above with B's at or below, or A's at or below with B's above, counts. Odd is inside. An unordered compare is never "above". Completion: 100%.
    #[must_use]
    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        if self.count_24 < 3
            || self.least(self.min_x_10, |point| point.x) > x
            || self.greatest(self.max_x_18, |point| point.x) < x
            || self.least(self.min_y_14, |point| point.y) > y
            || self.greatest(self.max_y_1c, |point| point.y) < y
        {
            return false;
        }
        let mut crossings = 0_u32;
        for (index, a) in self.points_00.iter().enumerate() {
            let b = &self.points_00[(index + 1) % self.points_00.len()];
            let (a_right, b_right) = (a.x > x, b.x > x);
            let eligible = (a_right && b_right)
                || (a_right != b_right && {
                    // 004A6AA0, at 24 bits: b.x - ((a.x - b.x) / (a.y - b.y)) * (b.y - y).
                    let slope = (a.x - b.x) / (a.y - b.y);
                    x < b.x - slope * (b.y - y)
                });
            if eligible && ((a.y > y && b.y <= y) || (a.y <= y && b.y > y)) {
                crossings = crossings.wrapping_add(1);
            }
        }
        !crossings.is_multiple_of(2)
    }

    /// IDA: 004A6DE0. Completion: 100%.
    #[must_use]
    pub const fn new(owns: u8) -> Self {
        Self {
            points_00: Vec::new(),
            field_08: 0,
            owns_0c: owns,
            min_x_10: BOUND_MAX,
            min_y_14: BOUND_MAX,
            max_x_18: BOUND_MIN,
            max_y_1c: BOUND_MIN,
            edge_falloff_20: 0x400,
            count_24: 0,
        }
    }

    /// IDA: 004A78A0 TESRegionPointList_IsValid(closed): closed with under 3 points is invalid; otherwise 004A7270(1) must be 0. Completion: 100%.
    #[must_use]
    pub fn is_valid(&self, closed: bool) -> bool {
        if closed && self.count_24 < 3 {
            return false;
        }
        !region_area_self_intersects(self, true)
    }
}

/// IDA: 004A6AF0 the segment test of a1-a2 against a3-a4. The x87 runs at 24 bits here, so each step is an `f32` in the exe's grouping. An unordered compare goes where the status bits send it: `test ah,5` with `jp` treats a NaN as not below 0, and `test ah,0x41` with `jne` treats it as not above. The C++'s `dx >= 0` and `denominator >= first` compares differ from the exe on a NaN. Completion: 100%.
#[must_use]
pub fn region_segments_intersect(
    a1: TesRegionPoint,
    a2: TesRegionPoint,
    a3: TesRegionPoint,
    a4: TesRegionPoint,
) -> bool {
    let dx = a2.x - a1.x;
    let ex = a3.x - a4.x;
    let (high, low) = if dx < 0.0 { (a1.x, a2.x) } else { (a2.x, a1.x) };
    if ex > 0.0 {
        if a4.x > high || low > a3.x {
            return false;
        }
    } else if a3.x > high || low > a4.x {
        return false;
    }
    let dy = a2.y - a1.y;
    let ey = a3.y - a4.y;
    let (high, low) = if dy < 0.0 { (a1.y, a2.y) } else { (a2.y, a1.y) };
    if ey > 0.0 {
        if a4.y > high || low > a3.y {
            return false;
        }
    } else if a3.y > high || low > a4.y {
        return false;
    }
    let fx = a1.x - a3.x;
    let fy = a1.y - a3.y;
    let denominator = dy * ex - ey * dx;
    let first = dx * fy - dy * fx;
    let second = fx * ey - ex * fy;
    if denominator == 0.0 {
        return true;
    }
    if denominator > 0.0 {
        if second < 0.0 || second > denominator || first < 0.0 {
            return false;
        }
        return denominator >= first || denominator.is_nan() || first.is_nan();
    }
    if second > 0.0 || second < denominator || first > 0.0 {
        return false;
    }
    denominator <= first || denominator.is_nan() || first.is_nan()
}

/// IDA: 004A7270 TESRegionPointList_SelfIntersects(closed): under 4 points, 0. Each segment (first, second) from the head, against each later segment (left, right), where a closed walk not on the head's own segment ends with the wrap to the head. Completion: 100%.
#[must_use]
pub fn region_area_self_intersects(area: &TesRegionPointList, closed: bool) -> bool {
    if area.count_24 < 4 {
        return false;
    }
    let points = &area.points_00;
    let mut first = 0_usize;
    let mut second = 1_usize;
    let mut third = 2_usize;
    while third < points.len() {
        let mut left = Some(third);
        let mut right = (third + 1 < points.len()).then_some(third + 1);
        loop {
            let mut last = false;
            if right.is_none() {
                if closed && first != 0 {
                    right = Some(0);
                }
                last = true;
            }
            if let (Some(l), Some(r)) = (left, right)
                && region_segments_intersect(points[first], points[second], points[l], points[r])
            {
                return true;
            }
            left = right;
            right = right.and_then(|node| (node + 1 < points.len()).then_some(node + 1));
            if last {
                break;
            }
        }
        first = second;
        second += 1;
        third = second + 1;
    }
    false
}

// IDA: 004A6F40 TESRegionPointList_LoadRPLD(file), whose byte result the caller ignores: any other chunk or an empty one reads nothing; a size that isn't a multiple of 8 warns. The buffer goes in whole; +0x24 = 0; each point is prepended, and widens the bounds, `fld`/`fstp`ing the coordinate on a strict compare that a NaN fails. Completion: 100%.
fn load_points(
    area: &mut TesRegionPointList,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    if cx.file.chunk_type()? != RPLD {
        return Ok(());
    }
    let size = cx.file.chunk_size()?;
    if size & 7 != 0 {
        let name = cx.file.file()?.name_1c.clone();
        cx.file.services.warn(format!(
            "Invalid Region Point List data in file \"{name}\"."
        ));
        return Ok(());
    }
    if size == 0 {
        return Ok(());
    }
    let mut buffer = vec![0_u8; size as usize];
    cx.file.read_chunk(&mut buffer, size)?;
    area.count_24 = 0;
    for entry in buffer.as_chunks::<8>().0 {
        let x_bits = u32::from_le_bytes(entry[..4].try_into().expect("four bytes"));
        let y_bits = u32::from_le_bytes(entry[4..].try_into().expect("four bytes"));
        let point = TesRegionPoint {
            x: f32::from_bits(x_bits),
            y: f32::from_bits(y_bits),
        };
        if area.min_x_10 > point.x {
            area.min_x_10 = x87::fld_fstp_copy(x_bits);
        }
        if area.min_y_14 > point.y {
            area.min_y_14 = x87::fld_fstp_copy(y_bits);
        }
        if area.max_x_18 < point.x {
            area.max_x_18 = x87::fld_fstp_copy(x_bits);
        }
        if area.max_y_1c < point.y {
            area.max_y_1c = x87::fld_fstp_copy(y_bits);
        }
        area.points_00.insert(0, point);
        area.count_24 = area.count_24.wrapping_add(1);
    }
    Ok(())
}

/// IDA: a grass object of a region: 004A59E0 TESRegionGrassObject (0x0C): the vtable, +0x04 the TESGrass, +0x08 the parent TESLandTexture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TesRegionGrassObject {
    pub original_vtable: u32,
    pub grass_04: Entity,
    pub land_texture_08: Option<Entity>,
}

/// IDA: an RDSD entry (0x0C): the sound, then flags and chance as read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TesRegionSound {
    pub sound_00: Entity,
    pub flags_04: u32,
    pub chance_08: u32,
}

/// IDA: what the five classes hold past the base. vt+0x0C of each is its type number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionDataKind {
    /// IDA: 004A5610 (0x10, type 3): +0x08 the weather list.
    Weather { weathers_08: Vec<WeatherEntry> },
    /// IDA: 004A4A80 (0x10, type 4): +0x08 the name.
    Map { name_08: BsStringT },
    /// IDA: 004A3CD0 (0x0C, type 5): +0x08 a FormHeap TESTexture.
    Landscape { texture_08: TesTexture },
    /// IDA: 004A35E0 (0x0C, type 6): +0x08 a FormHeap 004A5FD0 list object (0x14): the vtable, +0x04/+0x08 the list, +0x0C owns (1), +0x10 the count.
    Grass {
        grasses_04: Vec<TesRegionGrassObject>,
        owns_0c: u8,
        count_10: u32,
    },
    /// IDA: 004A55C0 (0x14, type 7): +0x08 the music type, +0x0C/+0x10 the RDSD entries.
    Sound {
        music_08: u32,
        sounds_0c: Vec<TesRegionSound>,
    },
}

impl RegionDataKind {
    /// IDA: vt+0x0C: 0073FD50, 0073DAA0, 00924E30, 007AFE90, 004059C0.
    #[must_use]
    pub const fn type_number(&self) -> u32 {
        match self {
            Self::Weather { .. } => 3,
            Self::Map { .. } => 4,
            Self::Landscape { .. } => 5,
            Self::Grass { .. } => 6,
            Self::Sound { .. } => 7,
        }
    }
}

/// IDA: 004A34C0 TESRegionData: the vtable (A3FFCC until the class's replaces it), +0x04 flags, +0x05, +0x06 priority (0x32), as an entity with the class's state.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct TesRegionData {
    pub original_vtable: u32,
    pub flags_04: u8,
    pub field_05: u8,
    pub priority_06: u8,
    pub kind: RegionDataKind,
}

// IDA: 004A4600 TESRegionDataManager vt+4: a region data of a type 3..=7 (`type - 3 <= 4` unsigned, the 004A4754 table), else null. Completion: 100%.
fn create_region_data(world: &mut World, kind: u32) -> Option<Entity> {
    let (original_vtable, kind) = match kind {
        3 => (
            TES_REGION_DATA_WEATHER_VTABLE,
            RegionDataKind::Weather {
                weathers_08: Vec::new(),
            },
        ),
        4 => {
            let mut name_08 = BsStringT::default();
            name_08.set(Some(b"Default Region Name"), 0);
            (TES_REGION_DATA_MAP_VTABLE, RegionDataKind::Map { name_08 })
        }
        5 => {
            let mut texture_08 = TesTexture {
                original_vtable: TES_TEXTURE_VTABLE,
                path_04: BsStringT::default(),
            };
            texture_08.path_04.set(Some(b"Trees\\CanopyShadow.dds"), 0);
            (
                TES_REGION_DATA_LANDSCAPE_VTABLE,
                RegionDataKind::Landscape { texture_08 },
            )
        }
        6 => (
            TES_REGION_DATA_GRASS_VTABLE,
            RegionDataKind::Grass {
                grasses_04: Vec::new(),
                owns_0c: 1,
                count_10: 0,
            },
        ),
        7 => (
            TES_REGION_DATA_SOUND_VTABLE,
            RegionDataKind::Sound {
                music_08: 0,
                sounds_0c: Vec::new(),
            },
        ),
        _ => return None,
    };
    Some(
        world
            .spawn(TesRegionData {
                original_vtable,
                flags_04: 0,
                field_05: 0,
                priority_06: 0x32,
                kind,
            })
            .id(),
    )
}

// IDA: 004A5960 vt+0x20 of the weather data: 004EEDD0 on +0x08 with g_disableWarningMessages set, and if 00B34D88 grew, a warning. Returns 1. Completion: 100%.
fn load_region_weather(
    world: &mut World,
    data: Entity,
    region: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let id = world
        .get::<FormIdentity>(region)
        .expect("a region")
        .form_id_0c;
    let mut list = match &mut world
        .get_mut::<TesRegionData>(data)
        .expect("a region data")
        .kind
    {
        RegionDataKind::Weather { weathers_08 } => std::mem::take(weathers_08),
        _ => {
            return Err(GameError::unclassified(
                0x004A4938,
                "004A4938: not a weather data",
            ));
        }
    };
    let result = with_weather_list_warnings(world, cx, |world, cx| {
        load_weather_list(world, &mut list, id, cx)
    });
    if let RegionDataKind::Weather { weathers_08 } = &mut world
        .get_mut::<TesRegionData>(data)
        .expect("a region data")
        .kind
    {
        *weathers_08 = list;
    }
    if result? {
        cx.file.services.warn(
            "Warnings were encountered while loading weather list chunk from region ".to_owned(),
        );
    }
    Ok(())
}

// IDA: 0046B250 with the ID, null for an absent one.
fn lookup(world: &World, id: u32) -> Option<Entity> {
    world.resource::<FormsById>().lookup(id)
}

// IDA: N/A the form type byte of a form.
fn form_type_of(world: &World, form: Entity) -> u8 {
    world
        .get::<FormIdentity>(form)
        .expect("a form")
        .form_type_04
}

// IDA: 004A59E0 TESRegionGrassObject(ids): with a region at the manager's +0x04, both IDs map through 0046BB20 with its last file (0046B680(-1)); without one they don't. A grass of type 0x1D is kept. Otherwise, one warning of four by whether there's an object and a region; the region's text is its vt+0xD4 (via 00404E50's length, whose guard byte doesn't change the answer here), then its ID as %d. With a grass and a parent: a type 0x0E parent is kept, otherwise a warning. Completion: 100%.
fn new_grass_object(
    world: &World,
    grass_id: u32,
    parent_id: u32,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(Option<Entity>, Option<Entity>), GameError> {
    let region = world.resource::<RegionLoadGlobals>().manager_region_04;
    let (grass, parent) = if let Some(region) = region {
        let file = crate::tes::form::mod_file_at_index(world, region, -1);
        let grass = lookup(world, map_file_form_id(world, cx, grass_id, file)?);
        let parent = lookup(world, map_file_form_id(world, cx, parent_id, file)?);
        (grass, parent)
    } else {
        (lookup(world, grass_id), lookup(world, parent_id))
    };
    let region_id = region.map(|region| {
        world
            .get::<FormIdentity>(region)
            .expect("a region")
            .form_id_0c as i32
    });
    let mut kept = None;
    match grass {
        Some(form) if form_type_of(world, form) == FORM_TYPE_GRASS => kept = Some(form),
        Some(_) => cx.file.services.warn(match region_id {
            Some(id) => format!(
                "Instantiating a Grass region object for region  ({id}) with a non-Grass object."
            ),
            None => "Instantiating a Grass region object with a non-Grass object.".to_owned(),
        }),
        None => cx.file.services.warn(match region_id {
            Some(id) => {
                format!("Instantiating a Grass region object for region  ({id}) without an object.")
            }
            None => "Instantiating a Grass region object without an object.".to_owned(),
        }),
    }
    let mut land = None;
    if kept.is_some()
        && let Some(parent) = parent
    {
        if form_type_of(world, parent) == FORM_TYPE_LAND_TEXTURE {
            land = Some(parent);
        } else {
            cx.file.services.warn(
                "Instantiating a Grass region object with a parent that isn't a land texture."
                    .to_owned(),
            );
        }
    }
    Ok((kept, land))
}

// IDA: 004A38A0 vt+0x20 of the grass data: another chunk returns 0; a size that isn't a multiple of 8 warns and returns 0; no entries returns 1. Each {grass, parent} builds an object (004A59E0): with a grass (vt+4), 004A5FF0 appends it to the +0x08 list and counts it; otherwise vt+8(1) deletes it. The result isn't read by 004A4890. Completion: 100%.
fn load_region_grass(
    world: &mut World,
    data: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    if cx.file.chunk_type()? != RDGS {
        return Ok(());
    }
    let size = cx.file.chunk_size()?;
    if size & 7 != 0 {
        let name = cx.file.file()?.name_1c.clone();
        cx.file.services.warn(format!(
            "Invalid Region Grass Object Data in file \"{name}\"."
        ));
        return Ok(());
    }
    if size == 0 {
        return Ok(());
    }
    let mut buffer = vec![0_u8; size as usize];
    cx.file.read_chunk(&mut buffer, size)?;
    for entry in buffer.as_chunks::<8>().0 {
        let grass_id = u32::from_le_bytes(entry[..4].try_into().expect("four bytes"));
        let parent_id = u32::from_le_bytes(entry[4..].try_into().expect("four bytes"));
        let (grass, land) = new_grass_object(world, grass_id, parent_id, cx)?;
        let Some(grass_04) = grass else {
            continue;
        };
        if let RegionDataKind::Grass {
            grasses_04,
            count_10,
            ..
        } = &mut world
            .get_mut::<TesRegionData>(data)
            .expect("a region data")
            .kind
        {
            grasses_04.push(TesRegionGrassObject {
                original_vtable: TES_REGION_GRASS_OBJECT_VTABLE,
                grass_04,
                land_texture_08: land,
            });
            *count_10 = count_10.wrapping_add(1);
        }
    }
    Ok(())
}

// IDA: 004A51F0 vt+0x20 of the sound data: RDMD reads 4 bytes into +0x08 (004510E0). RDSD: count = size / 12 (the `mul`, `shr` 3); a size of exactly 4 returns 0 (the old music form). A block of count entries (each {0, 0xF, 0}, all overwritten) reads with size 12 * count: count 0 is the "no limit" read into a FormHeap(0) block, which overruns the heap in the original, and a remainder truncates, with the warning. Each nonzero ID that 004473F0 finds and casts to TESSound, unmapped, is appended with the flags and chance as read. Completion: 100%.
fn load_region_sound(
    world: &mut World,
    data: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let chunk = cx.file.chunk_type()?;
    if chunk == RDMD {
        let mut music = 0;
        if let RegionDataKind::Sound { music_08, .. } = &world
            .get::<TesRegionData>(data)
            .expect("a region data")
            .kind
        {
            music = *music_08;
        }
        let (bits, _) = cx.file.read_dword(music)?;
        if let RegionDataKind::Sound { music_08, .. } = &mut world
            .get_mut::<TesRegionData>(data)
            .expect("a region data")
            .kind
        {
            *music_08 = bits;
        }
        return Ok(());
    }
    if chunk != RDSD {
        return Ok(());
    }
    let size = cx.file.chunk_size()?;
    let count = size / 12;
    if size == 4 {
        return Ok(());
    }
    let wanted = count * 12;
    let mut buffer = vec![0_u8; (wanted as usize).max(size as usize)];
    cx.file.read_chunk(&mut buffer, wanted)?;
    for entry in buffer[..wanted as usize].as_chunks::<12>().0 {
        let id = u32::from_le_bytes(entry[..4].try_into().expect("four bytes"));
        let flags_04 = u32::from_le_bytes(entry[4..8].try_into().expect("four bytes"));
        let chance_08 = u32::from_le_bytes(entry[8..].try_into().expect("four bytes"));
        if id == 0 {
            continue;
        }
        let Some(sound_00) =
            lookup(world, id).filter(|&form| form_type_of(world, form) == FORM_TYPE_SOUND)
        else {
            continue;
        };
        if let RegionDataKind::Sound { sounds_0c, .. } = &mut world
            .get_mut::<TesRegionData>(data)
            .expect("a region data")
            .kind
        {
            sounds_0c.push(TesRegionSound {
                sound_00,
                flags_04,
                chance_08,
            });
        }
    }
    Ok(())
}

// IDA: 004A49F0 vt+0x20 of the map data: an RDMP's text into +0x08. Completion: 100%.
fn load_region_map(
    world: &mut World,
    data: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    if cx.file.chunk_type()? != RDMP {
        return Ok(());
    }
    if let RegionDataKind::Map { name_08 } = &mut world
        .get_mut::<TesRegionData>(data)
        .expect("a region data")
        .kind
    {
        load_string(name_08, cx)?;
    }
    Ok(())
}

// IDA: 004A3BB0 vt+0x20 of the landscape data: an ICON goes through 004700E0 on +0x08; RDLN reaches here and does nothing. Completion: 100%.
fn load_region_landscape(
    world: &mut World,
    data: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    if cx.file.chunk_type()? != ICON {
        return Ok(());
    }
    if let RegionDataKind::Landscape { texture_08 } = &mut world
        .get_mut::<TesRegionData>(data)
        .expect("a region data")
        .kind
    {
        load_texture(texture_08, cx)?;
    }
    Ok(())
}

// IDA: 00B3541C when the 004A47A0..004A4860 slot's type test passes: each slot answers its argument if vt+0x0C is its number. A dangling pointer here (a chunk after the region that owned it was reset) has no host value, so that's a stop.
fn current_of_type(world: &World, kind: u32) -> Result<Option<Entity>, GameError> {
    let Some(data) = world.resource::<RegionLoadGlobals>().current_b3541c else {
        return Ok(None);
    };
    let found = world
        .get::<TesRegionData>(data)
        .ok_or(GameError::unclassified(
            0x00B3541C,
            "00B3541C: a region data that was freed",
        ))?;
    Ok((found.kind.type_number() == kind).then_some(data))
}

// IDA: 004A4890 TESRegionDataManager vt+0x20(file, region): +0x04 = region. RDAT: 8 bytes over zeros; 00B3541C = vt+4 of the first dword, even when null; with one, vt+8 004A35B0 takes +0x04 from byte 4, and from byte 5 +0x06 when it's at most 100 unsigned; a prepend (00446CB0) on the region's +0x18 list. RDWT, RDMP, RDLN and ICON, RDGS, RDMD and RDSD load the current data if it has the slot's type. RDOT, RDOB and RDOJ do nothing, nor does anything else. Completion: 100%.
fn load_region_data_chunk(
    world: &mut World,
    region: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    world
        .get_resource_or_init::<RegionLoadGlobals>()
        .manager_region_04 = Some(region);
    match cx.file.chunk_type()? {
        RDAT => {
            let mut bytes = [0_u8; 8];
            cx.file.read_chunk(&mut bytes, 8)?;
            let kind = u32::from_le_bytes(bytes[..4].try_into().expect("four bytes"));
            let data = create_region_data(world, kind);
            world.resource_mut::<RegionLoadGlobals>().current_b3541c = data;
            if let Some(data) = data {
                {
                    let mut found = world.get_mut::<TesRegionData>(data).expect("just spawned");
                    found.flags_04 = bytes[4];
                    if bytes[5] <= 100 {
                        found.priority_06 = bytes[5];
                    }
                }
                world
                    .get_mut::<TesRegion>(region)
                    .expect("a region")
                    .data_18
                    .items
                    .insert(0, data);
            }
        }
        RDWT => {
            if let Some(data) = current_of_type(world, 3)? {
                load_region_weather(world, data, region, cx)?;
            }
        }
        RDMP => {
            if let Some(data) = current_of_type(world, 4)? {
                load_region_map(world, data, cx)?;
            }
        }
        RDLN | ICON => {
            if let Some(data) = current_of_type(world, 5)? {
                load_region_landscape(world, data, cx)?;
            }
        }
        RDGS => {
            if let Some(data) = current_of_type(world, 6)? {
                load_region_grass(world, data, cx)?;
            }
        }
        RDMD | RDSD => {
            if let Some(data) = current_of_type(world, 7)? {
                load_region_sound(world, data, cx)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// IDA: 004A43E0 TESRegionDataList (0x0C): the list, +0x08 owns (1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TesRegionDataList {
    pub items: Vec<Entity>,
    pub owns_08: u8,
}

/// IDA: 004A2C30 TESRegion (0x2C): +0x18 the TESRegionDataList, +0x1C the area list (an 8-byte FormHeap head), +0x20 the worldspace (WNAM, raw until InitItem), +0x24, +0x28 float.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct TesRegion {
    pub data_18: TesRegionDataList,
    pub areas_1c: Vec<TesRegionPointList>,
    pub worldspace_20: FormRef,
    pub field_24: u32,
    pub field_28: f32,
}

// IDA: 004A2C30 TESRegion_Constructor: 0046C460, the vtable, 004A43E0(1), the area head, 00A30634 (-1.0) at +0x28, +0x04, +0x20 = +0x24 = 0. No component pass. Completion: 100% of the store order.
fn construct_region(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        OriginalVtable(TES_REGION_VTABLE),
        TesRegion {
            data_18: TesRegionDataList {
                items: Vec::new(),
                owns_08: 1,
            },
            areas_1c: Vec::new(),
            worldspace_20: FormRef::Raw(0),
            field_24: 0,
            field_28: -1.0,
        },
    ));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_REGION;
    Ok(form)
}

// IDA: 004A2E20 TESRegion vt+0x18, with no 0046C2F0: 004A4400 pops the data list, vt+0(1) on each when +0x08 owns, then each area is popped, 004A76F0'd and freed. A freed data the 00B3541C global still names is the dangling pointer `current_of_type` stops on. Completion: 100%.
fn destroy_region(world: &mut World, form: Entity) -> Result<(), GameError> {
    let (items, owns) = {
        let mut region = world.get_mut::<TesRegion>(form).expect("a region");
        region.areas_1c.clear();
        (
            std::mem::take(&mut region.data_18.items),
            region.data_18.owns_08,
        )
    };
    if owns != 0 {
        for item in items {
            world.despawn(item);
        }
    }
    Ok(())
}

// IDA: 004A31C0 TESRegion vt+0x1C LoadForm. The record must be REGN (the low byte); 00451210; 0046AB80(0). The chunks: WNAM 004510E0 over a zeroed local into +0x20, and a nonzero ID on a form with bit 6 outside a load (+0xCD7 clear) calls 004EF170 on that raw ID, a host stop; EDID; RPLI 004510E0, a new area (004A6DE0(1), 004A6D70 the value), remembered and appended (0067B1E0); RPLD 004A6F40 on the remembered area, if any; the region data chunks through +0xCD8 vt+0x20. Then the walk: while the node's item is set, an invalid area is removed (0065C620), destroyed, and the walk restarts from the head. Returns 1. Completion: 100%, with that stop.
// TODO: [ ] 004A3356 004EF170, when a WNAM outside a load is reachable.
fn load_region_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_REGION {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    let mut area: Option<usize> = None;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            WNAM => {
                let (id, _) = cx.file.read_dword(0)?;
                world
                    .get_mut::<TesRegion>(form)
                    .expect("a region")
                    .worldspace_20 = FormRef::Raw(id);
                let flags = world.get::<FormIdentity>(form).expect("a form").flags_08;
                if id != 0 && flags & 0x40 != 0 && world.resource::<DataHandler>().loading_cd7 == 0
                {
                    return Err(GameError::unclassified(
                        0x004A3356,
                        "004A3356: 004EF170 on the raw WNAM form ID (not while loading)",
                    ));
                }
            }
            EDID => {
                cx.file.read_text(0x200)?;
            }
            RPLD => {
                if let Some(index) = area {
                    let mut region = world.get_mut::<TesRegion>(form).expect("a region");
                    load_points(&mut region.areas_1c[index], cx)?;
                }
            }
            RPLI => {
                let (value, _) = cx.file.read_dword(0)?;
                let mut created = TesRegionPointList::new(1);
                created.edge_falloff_20 = value;
                let mut region = world.get_mut::<TesRegion>(form).expect("a region");
                region.areas_1c.push(created);
                area = Some(region.areas_1c.len() - 1);
            }
            RDAT | RDOT | RDWT | RDMP | RDGS | RDLN | ICON | RDMD | RDSD | RDOB | RDOJ => {
                load_region_data_chunk(world, form, cx)?;
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    // 004A33C0..004A3401.
    let mut region = world.get_mut::<TesRegion>(form).expect("a region");
    let mut index = 0;
    while index < region.areas_1c.len() {
        if region.areas_1c[index].is_valid(true) {
            index += 1;
        } else {
            region.areas_1c.remove(index);
            index = 0;
        }
    }
    Ok(true)
}

/// IDA: TESRegion's class. 0046C2E0 finds no slots, and +0x20 stays through a reset.
/// TODO: [ ] 12d: the InitItem body 004A3430.
pub const CLASSES: &[FormClass] = &[FormClass {
    form_type: FORM_TYPE_REGION,
    vtable: TES_REGION_VTABLE,
    construct: construct_region,
    load_form: load_region_form,
    reset_14: reset_14_components,
    reset_18: destroy_region,
    editor_id: default_editor_id,
    init_item: super::forms_init::init_region,
    slots: &[],
    special_arm: None,
}];

// IDA: 004A4460(type) on a region's +0x18: the first item whose vt+0x0C is the type.
pub(crate) fn region_data_of_type(
    world: &World,
    region: Entity,
    kind: u32,
) -> Option<&RegionDataKind> {
    world
        .get::<TesRegion>(region)?
        .data_18
        .items
        .iter()
        .filter_map(|&data| world.get::<TesRegionData>(data))
        .map(|data| &data.kind)
        .find(|data| data.type_number() == kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn point(x: f32, y: f32) -> TesRegionPoint {
        TesRegionPoint { x, y }
    }

    fn area(points: &[(f32, f32)]) -> TesRegionPointList {
        let mut list = TesRegionPointList::new(1);
        list.points_00 = points.iter().map(|&(x, y)| point(x, y)).collect();
        list.count_24 = points.len() as u32;
        list
    }

    #[test]
    fn segments_crossing_parallel_and_apart() {
        assert!(region_segments_intersect(
            point(0.0, 0.0),
            point(2.0, 2.0),
            point(0.0, 2.0),
            point(2.0, 0.0),
        ));
        // Bounding boxes apart.
        assert!(!region_segments_intersect(
            point(0.0, 0.0),
            point(1.0, 1.0),
            point(5.0, 5.0),
            point(6.0, 7.0),
        ));
        // Overlapping boxes that don't cross.
        assert!(!region_segments_intersect(
            point(0.0, 0.0),
            point(4.0, 0.5),
            point(0.0, 2.0),
            point(4.0, 1.0),
        ));
        // Collinear overlap has a zero denominator: 1.
        assert!(region_segments_intersect(
            point(0.0, 0.0),
            point(2.0, 0.0),
            point(1.0, 0.0),
            point(3.0, 0.0),
        ));
    }

    #[test]
    fn nan_takes_the_exes_arms() {
        // `dx` NaN: the "not below zero" arm, with compares that all fail, so the walk reaches a NaN denominator: the 0x41 test sends it to the second arm, whose `fcompp` is unordered, and that's a 1.
        assert!(region_segments_intersect(
            point(f32::NAN, 0.0),
            point(1.0, 1.0),
            point(0.0, 1.0),
            point(1.0, 0.0),
        ));
    }

    #[test]
    fn self_intersection_walk_wraps_when_closed() {
        // A square, valid.
        let square = area(&[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]);
        assert!(!region_area_self_intersects(&square, true));
        assert!(square.is_valid(true));
        // A bow tie crosses.
        let bow = area(&[(0.0, 0.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)]);
        assert!(region_area_self_intersects(&bow, true));
        assert!(!bow.is_valid(true));
        // Under three points is invalid when closed; three can't cross.
        assert!(!area(&[(0.0, 0.0), (1.0, 1.0)]).is_valid(true));
        assert!(area(&[(0.0, 0.0), (1.0, 1.0), (0.0, 1.0)]).is_valid(true));
    }

    #[test]
    fn new_area_has_the_exes_bounds() {
        let list = TesRegionPointList::new(1);
        assert_eq!(list.min_x_10.to_bits(), 0x7F7F_FFFF);
        assert_eq!(list.max_y_1c.to_bits(), 0xFF7F_FFFF);
        assert_eq!(list.edge_falloff_20, 0x400);
    }
}
