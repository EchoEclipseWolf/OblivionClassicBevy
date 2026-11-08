//! M6: the saves a menu lists: 0045D450's walk of `Saves\*.ess`, newest first, with what 0045DBC0 and 0045D5F0 read of each file's header, for the Load menu's rows and its picture. Nothing here writes a save. A test names its own folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use super::interface_scene::SavePreview;

/// IDA: SaveGameFile (0x160, BSFile +0x3C the path) and the fields 0045D5F0 reads into 00464060's and 004610F0's buffers: the save number, the player's name, level, location, days, play time ms, the SYSTEMTIME and the screenshot. `readable` is 0045DBC0's non-zero answer. Completion: 100% of the fields the menus read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveFile {
    pub path: PathBuf,
    pub readable: bool,
    pub save_number: u32,
    pub player: Vec<u8>,
    pub level: u16,
    /// None for a count of 0, 0045D72C's arm: 00B38728 takes its place.
    pub location: Option<Vec<u8>>,
    pub game_days: i32,
    pub play_ticks: u32,
    pub time: [u16; 8],
    pub preview: Option<Arc<SavePreview>>,
}

impl SaveFile {
    /// IDA: 005AE6D0 with 00464060: a name of the file's own gives "name\nPlay Time: t"; "Save n - ..." gives "Save n - Level x\nPlay Time: t"; 00459570's parse of the old "Save n - ... Playing Time" names isn't kept, a recorded limit. Completion: 90%.
    /// TODO: [ ] 00459570's name parse for files with no readable header.
    #[must_use]
    pub fn caption(
        &self,
        save_word: &[u8],
        quick_word: &[u8],
        auto_word: &[u8],
        time_word: &[u8],
        level_word: &[u8],
    ) -> Vec<u8> {
        let stem = self.stem();
        // 004640BD: an unreadable file's level, count and time buffers stay empty, and 0.
        let (level, number, time) = if self.readable {
            (
                [level_word, b" ", self.level.to_string().as_bytes()].concat(),
                self.save_number,
                self.play_time(),
            )
        } else {
            (Vec::new(), 0, String::new())
        };
        // 004641C1, 0046423A: `strstr` over the whole path, so case counts. 004642C9's "Save " is `_strnicmp` on the file's name.
        let path: Vec<u8> = self.path.to_string_lossy().bytes().collect();
        let contains = |needle: &[u8]| path.windows(needle.len()).any(|window| window == needle);
        // 00464060: "quicksave" and "autosave" name the row with their words and the level.
        let name: Vec<u8> = if contains(b"quicksave") {
            [quick_word, b" - ", &level].concat()
        } else if contains(b"autosave") {
            [auto_word, b" - ", &level].concat()
        } else if stem
            .get(..5)
            .is_some_and(|head| head.eq_ignore_ascii_case(b"save "))
        {
            [
                save_word,
                b" ",
                number.to_string().as_bytes(),
                b" - ",
                &level,
            ]
            .concat()
        } else {
            // 00464060: the file's name, four bytes off the end, cut at 18.
            stem.iter().take(18).copied().collect()
        };
        [&name[..], b"\n", time_word, b": ", time.as_bytes()].concat()
    }

    /// IDA: 00464060's caption buffer non-empty (the `strlen` 005AE6D0 and 005D3650 test): every arm of `caption` but
    /// "Save n - level", which is the empty buffer's "%s %i - %s" format. Completion: as `caption`.
    #[must_use]
    pub fn caption_is_named(&self) -> bool {
        let path: Vec<u8> = self.path.to_string_lossy().bytes().collect();
        let contains = |needle: &[u8]| path.windows(needle.len()).any(|window| window == needle);
        contains(b"quicksave")
            || contains(b"autosave")
            || !self
                .stem()
                .get(..5)
                .is_some_and(|head| head.eq_ignore_ascii_case(b"save "))
    }

    /// IDA: 005AE240's sprintf("%s\n%s\n%s\n%s\n%s", name, "Level n", location, "Days n", date) with 004610F0's "%s %i" and "%d/%d/%02d %02d:%02d". Completion: 100%.
    #[must_use]
    pub fn detail_text(
        &self,
        level_word: &[u8],
        days_word: &[u8],
        unknown_location: &[u8],
    ) -> Vec<u8> {
        let [year, month, _, day, hour, minute, _, _] = self.time;
        [
            &self.player[..],
            b"\n",
            level_word,
            b" ",
            self.level.to_string().as_bytes(),
            b"\n",
            self.location.as_deref().unwrap_or(unknown_location),
            b"\n",
            days_word,
            b" ",
            self.game_days.to_string().as_bytes(),
            b"\n",
            format!("{month}/{day}/{year:02} {hour:02}:{minute:02}").as_bytes(),
        ]
        .concat()
    }

    // IDA: 00464060's "%02i:%02i:%02i" of the milliseconds.
    fn play_time(&self) -> String {
        let ticks = self.play_ticks;
        format!(
            "{:02}:{:02}:{:02}",
            ticks / 0x36_EE80,
            ticks % 0x36_EE80 / 0xEA60,
            ticks % 0x36_EE80 % 0xEA60 / 0x3E8
        )
    }

    // IDA: N/A a helper, with no address of its own.
    fn stem(&self) -> Vec<u8> {
        self.path
            .file_stem()
            .map_or_else(Vec::new, |stem| stem.to_string_lossy().bytes().collect())
    }
}

/// IDA: 0045D450 TESSaveLoadGame_RebuildSaveFileList: `*.ess`, a size of 0 skipped, inserted by 00416650 with SaveGameFile_CompareWriteTimeDescending. Host: file names sort a tie, as FindFirstFile's order isn't here. A missing folder is an empty list. Completion: 100% of the filter and the order.
#[must_use]
pub fn rebuild_save_file_list(directory: &Path) -> Vec<SaveFile> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let ess = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ess"));
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if ess && metadata.is_file() && metadata.len() != 0 {
            found.push((metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH), path));
        }
    }
    found.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    found
        .into_iter()
        .map(|(_, path)| read_save_file(path))
        .collect()
}

/// IDA: 0045DBC0 and 0045D5F0 on a save with form version 0x38 or more (004610F0): "TES4SAVEGAME", the version, the SYSTEMTIME; the record's size; then save number, name (u8 count), level, location (u8 count), days, ticks, the time again, the screenshot's size, width, height and RGB24. A file that doesn't read this way keeps `readable` false, which 004610F0 turns into the empty fields. Completion: 90%: the version gates below 0x38 aren't taken, since 1.2 writes 0x7D.
pub fn read_save_file(path: PathBuf) -> SaveFile {
    let mut save = SaveFile {
        path,
        readable: false,
        save_number: 0,
        player: Vec::new(),
        level: 0,
        location: None,
        game_days: 0,
        play_ticks: 0,
        time: [0; 8],
        preview: None,
    };
    let Ok(bytes) = std::fs::read(&save.path) else {
        return save;
    };
    let _ = parse_save_header(&bytes, &mut save);
    save
}

// IDA: the reads of `read_save_file`, each little-endian. `None` is a short file.
fn parse_save_header(bytes: &[u8], save: &mut SaveFile) -> Option<()> {
    let mut at = 0usize;
    let mut take = |count: usize| -> Option<&[u8]> {
        let slice = bytes.get(at..at.checked_add(count)?)?;
        at += count;
        Some(slice)
    };
    if take(12)? != b"TES4SAVEGAME" {
        return None;
    }
    // Major, minor, the file's SYSTEMTIME and the form version; 0045DE08 returns the record's size, and 004640B9 takes 0 as unreadable.
    take(2 + 16 + 4)?;
    if u32::from_le_bytes(take(4)?.try_into().ok()?) == 0 {
        return None;
    }
    save.save_number = u32::from_le_bytes(take(4)?.try_into().ok()?);
    let count = usize::from(take(1)?[0]);
    save.player = take(count)?
        .iter()
        .copied()
        .take_while(|&byte| byte != 0)
        .collect();
    save.level = u16::from_le_bytes(take(2)?.try_into().ok()?);
    let count = usize::from(take(1)?[0]);
    save.location = if count == 0 {
        None
    } else {
        Some(
            take(count)?
                .iter()
                .copied()
                .take_while(|&byte| byte != 0)
                .collect(),
        )
    };
    save.game_days = truncate_days(f32::from_le_bytes(take(4)?.try_into().ok()?));
    save.play_ticks = u32::from_le_bytes(take(4)?.try_into().ok()?);
    for field in &mut save.time {
        *field = u16::from_le_bytes(take(2)?.try_into().ok()?);
    }
    let size = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
    save.readable = true;
    // 0045D81D: any size but 0 reads the two words and the pixels.
    if size != 0 {
        let width = u32::from_le_bytes(take(4)?.try_into().ok()?);
        let height = u32::from_le_bytes(take(4)?.try_into().ok()?);
        let length = 3usize
            .checked_mul(width as usize)?
            .checked_mul(height as usize)?;
        let rgb = take(length)?.to_vec();
        // Host: a picture of no pixels is no picture. The exe would make an empty texture here.
        if length != 0 {
            save.preview = Some(Arc::new(SavePreview { width, height, rgb }));
        }
    }
    Some(())
}

// IDA: 00461236..0046123E: `fistp qword` with truncation set, of which %i prints the low word. An out-of-range or NaN value stores the indefinite, whose low word is 0.
fn truncate_days(days: f32) -> i32 {
    if days.is_nan() || days.abs() >= 9_223_372_036_854_775_808.0 {
        0
    } else {
        (days as i64) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save(number: u32, picture: bool) -> Vec<u8> {
        let mut body = number.to_le_bytes().to_vec();
        body.extend(b"\x05Test\0");
        body.extend(7u16.to_le_bytes());
        body.extend(b"\x05Cell\0");
        body.extend(3.5f32.to_le_bytes());
        body.extend(3_661_000u32.to_le_bytes());
        for field in [2006u16, 12, 1, 25, 9, 5, 0, 0] {
            body.extend(field.to_le_bytes());
        }
        if picture {
            body.extend(20u32.to_le_bytes());
            body.extend(2u32.to_le_bytes());
            body.extend(2u32.to_le_bytes());
            body.extend([250u8; 12]);
        } else {
            body.extend(0u32.to_le_bytes());
        }
        let mut file = b"TES4SAVEGAME".to_vec();
        file.extend([0, 125]);
        file.extend([0; 16]);
        file.extend(0x7Du32.to_le_bytes());
        file.extend((body.len() as u32).to_le_bytes());
        file.extend(body);
        file
    }

    fn folder(tag: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("recomp_saves_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    // The C++ `save_index_test` and `load_menu_test`'s file half: a corrupt or short file stays listed, unreadable; an empty one and another extension don't list; a missing folder is empty; a save with no picture keeps its fields.
    #[test]
    fn the_list_keeps_what_it_can() {
        assert!(rebuild_save_file_list(Path::new("/nonexistent/recomp")).is_empty());
        let folder = folder("list");
        std::fs::write(folder.join("Good.ess"), save(4, true)).unwrap();
        std::fs::write(folder.join("Bare.ess"), save(5, false)).unwrap();
        std::fs::write(folder.join("Corrupt.ess"), b"TES4").unwrap();
        let mut short = save(6, true);
        short.truncate(short.len() - 4);
        std::fs::write(folder.join("Short.ess"), short).unwrap();
        std::fs::write(folder.join("Empty.ess"), b"").unwrap();
        std::fs::write(folder.join("Other.txt"), b"x").unwrap();
        let list = rebuild_save_file_list(&folder);
        let by_name = |name: &str| list.iter().find(|save| save.path.ends_with(name));
        assert_eq!(list.len(), 4);
        assert!(by_name("Empty.ess").is_none());
        let good = by_name("Good.ess").unwrap();
        assert!(good.readable);
        assert_eq!(good.preview.as_ref().unwrap().rgb.len(), 12);
        assert_eq!(
            good.detail_text(b"Level", b"Day", b"Unknown Location"),
            b"Test\nLevel 7\nCell\nDay 3\n12/25/2006 09:05"
        );
        assert!(by_name("Bare.ess").unwrap().preview.is_none());
        assert!(!by_name("Corrupt.ess").unwrap().readable);
        assert!(by_name("Short.ess").unwrap().preview.is_none());
        let _ = std::fs::remove_dir_all(&folder);
    }

    // 0045D450's order: newest write first, whatever the names say.
    #[test]
    fn the_newest_comes_first() {
        let folder = folder("order");
        for (name, seconds) in [("a.ess", 100u64), ("b.ess", 300), ("c.ess", 200)] {
            let path = folder.join(name);
            std::fs::write(&path, save(1, false)).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
                .unwrap();
        }
        let names: Vec<_> = rebuild_save_file_list(&folder)
            .iter()
            .map(|save| save.path.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["b.ess", "c.ess", "a.ess"]);
        let _ = std::fs::remove_dir_all(&folder);
    }

    // 00464060 and 005AE6D0's captions.
    #[test]
    fn captions_follow_the_file_name() {
        let base = SaveFile {
            path: PathBuf::new(),
            readable: true,
            save_number: 9,
            player: Vec::new(),
            level: 3,
            location: None,
            game_days: 0,
            play_ticks: 3_661_000,
            time: [0; 8],
            preview: None,
        };
        let caption = |name: &str| {
            let save = SaveFile {
                path: name.into(),
                ..base.clone()
            };
            save.caption(b"Save", b"Quicksave", b"Autosave", b"Play Time", b"Level")
        };
        assert_eq!(
            caption("quicksave.ess"),
            b"Quicksave - Level 3\nPlay Time: 01:01:01"
        );
        assert_eq!(
            caption("autosave.ess"),
            b"Autosave - Level 3\nPlay Time: 01:01:01"
        );
        assert_eq!(
            caption("Save 9.ess"),
            b"Save 9 - Level 3\nPlay Time: 01:01:01"
        );
        assert_eq!(
            caption("MyLongCharacterNameHere.ess"),
            b"MyLongCharacterNam\nPlay Time: 01:01:01"
        );
    }

    // 00464060 at 4640BD: an unreadable file leaves the level and time buffers empty, so 005AE6D0's formats print nothing there.
    #[test]
    fn an_unreadable_file_has_no_level_or_time() {
        let mut save = read_save_file(PathBuf::from("/nonexistent/Corrupt.ess"));
        assert!(!save.readable);
        let words: [&[u8]; 5] = [b"Save", b"Quicksave", b"Autosave", b"Play Time", b"Level"];
        let caption =
            |save: &SaveFile| save.caption(words[0], words[1], words[2], words[3], words[4]);
        assert_eq!(caption(&save), b"Corrupt\nPlay Time: ");
        save.path = "quicksave.ess".into();
        assert_eq!(caption(&save), b"Quicksave - \nPlay Time: ");
        save.path = "Save 4.ess".into();
        assert_eq!(caption(&save), b"Save 0 - \nPlay Time: ");
    }

    // 004641C1 and 0046423A are `strstr`: "QuickSave" isn't "quicksave". 004642C9's "Save " is `_strnicmp`.
    #[test]
    fn the_quick_and_auto_names_are_case_sensitive() {
        let save = SaveFile {
            path: "QuickSave.ess".into(),
            readable: true,
            save_number: 1,
            player: Vec::new(),
            level: 3,
            location: None,
            game_days: 0,
            play_ticks: 0,
            time: [0; 8],
            preview: None,
        };
        assert_eq!(
            save.caption(b"Save", b"Quicksave", b"Autosave", b"Play Time", b"Level"),
            b"QuickSave\nPlay Time: 00:00:00"
        );
    }

    // 0045D72C: a location of no bytes takes sMenuDisplayUnknownLocationString (00B38728).
    #[test]
    fn an_empty_location_is_unknown() {
        let save = SaveFile {
            path: "a.ess".into(),
            readable: true,
            save_number: 1,
            player: b"P".to_vec(),
            level: 3,
            location: None,
            game_days: 2,
            play_ticks: 0,
            time: [2006, 1, 0, 2, 3, 4, 0, 0],
            preview: None,
        };
        assert_eq!(
            save.detail_text(b"Level", b"Day", b"Unknown Location"),
            b"P\nLevel 3\nUnknown Location\nDay 2\n1/2/2006 03:04"
        );
    }

    // 0046123A: `fistp qword` under truncation, the low word printed by %i: a negative day count stays negative, and a NaN's indefinite gives 0.
    #[test]
    fn days_truncate_as_a_signed_number() {
        let folder = folder("days");
        let mut bytes = save(1, false);
        let days_at = bytes
            .windows(4)
            .position(|w| w == 3.5f32.to_le_bytes())
            .unwrap();
        bytes[days_at..days_at + 4].copy_from_slice(&(-5.5f32).to_le_bytes());
        let path = folder.join("Days.ess");
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(read_save_file(path.clone()).game_days, -5);
        bytes[days_at..days_at + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(read_save_file(path).game_days, 0);
        let _ = std::fs::remove_dir_all(&folder);
    }

    // A hostile header costs a line, never a panic: sizes past the file and every truncation.
    #[test]
    fn a_hostile_header_stays_unreadable_or_short() {
        let folder = folder("hostile");
        let path = folder.join("Bad.ess");
        let good = save(1, true);
        for cut in 0..good.len() {
            std::fs::write(&path, &good[..cut]).unwrap();
            let _ = read_save_file(path.clone());
        }
        let mut big = save(1, true);
        let at = big.len() - 12 - 8;
        big[at..at + 8].copy_from_slice(&[0xFF; 8]);
        std::fs::write(&path, &big).unwrap();
        assert!(read_save_file(path.clone()).preview.is_none());
        // A picture of no pixels, and a record size of 0 (004640B9).
        let mut empty = save(1, true);
        empty[at..at + 8].copy_from_slice(&[0; 8]);
        std::fs::write(&path, &empty).unwrap();
        let read = read_save_file(path.clone());
        assert!(read.readable && read.preview.is_none());
        let mut none = save(1, true);
        none[34..38].copy_from_slice(&[0; 4]);
        std::fs::write(&path, &none).unwrap();
        assert!(!read_save_file(path).readable);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
