//! IDA: N/A original paths on a Linux file system: backslash separators, case-insensitive names,
//! and the sorted order NTFS gives `FindFirstFile`. A recorded host substitution for the Win32
//! file APIs. Completion: 100% for names that fit one byte per character.
//!
//! A game string is original bytes lifted one to one into a `&str`: each `char` is one byte of the original's `-A`
//! string, so 0xE9 is "é" and finds a UTF-8 "é" on disk. A disk name that is not UTF-8 lifts the same way, so it round
//! trips through `find_files` and `resolve` and two such names never collide.
//!
//! Recorded host substitutions: the lift is Latin-1, where a Windows code page is 1252 or other, so bytes 0x80..0x9F
//! name C1 controls here and not 1252's punctuation. Case folds through Rust's one-to-one uppercase map, standing in
//! for NTFS's $UpCase table.
//! TODO: [ ] Compare the uppercase map with an NTFS $UpCase dump, and the lift with the evidence machine's code page.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// IDA: N/A a disk name as a game string: itself when UTF-8, otherwise its bytes lifted one to one.
fn lifted_name(name: &OsStr) -> Cow<'_, str> {
    match name.to_str() {
        Some(text) => Cow::Borrowed(text),
        None => Cow::Owned(
            name.as_encoded_bytes()
                .iter()
                .map(|&byte| char::from(byte))
                .collect(),
        ),
    }
}

/// IDA: N/A the original's bytes of a lifted string: a character up to U+00FF is its byte. A character above it can only
/// come from a UTF-8 disk name, has no byte without a code page, and keeps its UTF-8 bytes.
/// Completion: 100% below U+0100.
#[must_use]
pub fn original_bytes(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    for character in text.chars() {
        match u8::try_from(u32::from(character)) {
            Ok(byte) => bytes.push(byte),
            Err(_) => {
                bytes.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
            }
        }
    }
    bytes
}

/// IDA: N/A a lifted string's length in the original's bytes, for its fixed `char` buffers.
#[must_use]
pub fn original_length(text: &str) -> usize {
    text.chars().count()
}

// IDA: N/A a helper, with no address of its own.
fn upcase(character: char) -> char {
    let mut upper = character.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => single,
        _ => character,
    }
}

// IDA: N/A a helper, with no address of its own.
fn names_match(left: &str, right: &str) -> bool {
    left.chars().map(upcase).eq(right.chars().map(upcase))
}

// IDA: N/A a helper, with no address of its own.
fn find_entry(directory: &Path, name: &OsStr) -> Option<PathBuf> {
    let direct = directory.join(name);
    if direct.exists() {
        return Some(direct);
    }
    let wanted = lifted_name(name);
    // Linux can hold names differing only by case, which NTFS cannot, and `read_dir` order is arbitrary: the smallest
    // raw name pins which one is taken.
    std::fs::read_dir(if directory.as_os_str().is_empty() {
        Path::new(".")
    } else {
        directory
    })
    .ok()?
    .flatten()
    .map(|entry| entry.file_name())
    .filter(|candidate| names_match(&lifted_name(candidate), &wanted))
    .min()
    .map(|found| directory.join(found))
}

/// IDA: N/A a helper, with no address of its own.
/// The host path of an original relative path: under `working` unless already absolute, each
/// component matched case-insensitively. Components that do not exist stay as written, so a write
/// creates them under that name.
#[must_use]
pub fn resolve(working: &Path, original: &str) -> PathBuf {
    let translated = original.replace('\\', "/");
    let relative = Path::new(&translated);
    let joined = if relative.is_absolute() {
        relative.to_path_buf()
    } else {
        working.join(relative)
    };
    if joined.exists() {
        return joined;
    }
    let mut current = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::Normal(name) => match find_entry(&current, name) {
                Some(found) => current = found,
                None => current.push(name),
            },
            other => current.push(other.as_os_str()),
        }
    }
    current
}

/// IDA: N/A a helper, with no address of its own.
/// `_access(path, 0) != -1`: files and directories. An empty path fails.
#[must_use]
pub fn access(working: &Path, original: &str) -> bool {
    !original.is_empty() && resolve(working, original).exists()
}

// IDA: N/A a helper, with no address of its own.
fn wildcard_matches(pattern: &[char], name: &[char]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some(('*', rest)) => (0..=name.len()).any(|skip| wildcard_matches(rest, &name[skip..])),
        Some(('?', rest)) => name
            .split_first()
            .is_some_and(|(_, tail)| wildcard_matches(rest, tail)),
        Some((first, rest)) => name.split_first().is_some_and(|(character, tail)| {
            upcase(*character) == upcase(*first) && wildcard_matches(rest, tail)
        }),
    }
}

/// IDA: N/A a helper, with no address of its own.
/// `FindFirstFile` / `_findfirst` over a pattern whose last component may hold `*` and `?`. Names are
/// returned in NTFS's order, sorted by uppercased name, whatever order `readdir` gives; names NTFS could
/// not hold side by side, differing only by case, then sort by themselves. Each is a lifted game string.
/// Linux has no 8.3 short names, so patterns cannot match through them.
/// TODO: [ ] `FindFirstFile` also returns "." and ".." to a pattern matching them ("*", "*.*"); `read_dir` does not.
#[must_use]
pub fn find_files(working: &Path, original_pattern: &str) -> Vec<String> {
    let translated = original_pattern.replace('\\', "/");
    let (directory, pattern) = translated.rsplit_once('/').unwrap_or((".", &translated));
    let directory = resolve(working, directory);
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let pattern: Vec<char> = pattern.chars().collect();
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| lifted_name(&entry.file_name()).into_owned())
        .filter(|name| wildcard_matches(&pattern, &name.chars().collect::<Vec<char>>()))
        .collect();
    names.sort_by_cached_key(|name| (name.chars().map(upcase).collect::<String>(), name.clone()));
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_resolve_case_insensitively_with_backslashes() {
        let root = std::env::temp_dir().join(format!("recomp_host_path_{}", std::process::id()));
        std::fs::create_dir_all(root.join("Data/Meshes")).unwrap();
        std::fs::write(root.join("Data/Meshes/Sword.NIF"), "x").unwrap();
        assert_eq!(
            resolve(&root, "data\\MESHES\\sword.nif"),
            root.join("Data/Meshes/Sword.NIF")
        );
        assert!(access(&root, "DATA\\meshes\\SWORD.nif"));
        assert!(access(&root, "data"));
        assert!(!access(&root, "data\\none.nif"));
        assert!(!access(&root, ""));
        assert_eq!(
            resolve(&root, "data\\new.txt"),
            root.join("Data/new.txt"),
            "a new name stays as written"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_files_matches_and_sorts_as_ntfs_does() {
        let root = std::env::temp_dir().join(format!("recomp_host_find_{}", std::process::id()));
        std::fs::create_dir_all(root.join("Data")).unwrap();
        for name in [
            "zeta - b.bsa",
            "Foo - Voices.bsa",
            "foo - meshes.BSA",
            "Foo.txt",
            "Foo.bsa",
        ] {
            std::fs::write(root.join("Data").join(name), "").unwrap();
        }
        assert_eq!(
            find_files(&root, "data\\Foo*.bsa"),
            ["foo - meshes.BSA", "Foo - Voices.bsa", "Foo.bsa"]
        );
        assert_eq!(find_files(&root, "Data\\F?o.t*"), ["Foo.txt"]);
        assert!(find_files(&root, "Missing\\*.bsa").is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Case-only duplicates exist on Linux and not on NTFS: `read_dir` order must not pick between them.
    #[test]
    fn case_only_duplicates_have_a_pinned_order() {
        let root = std::env::temp_dir().join(format!("recomp_host_case_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for name in ["b.bsa", "B.bsa", "a.bsa", "A.BSA"] {
            std::fs::write(root.join(name), "").unwrap();
        }
        assert_eq!(
            find_files(&root, "*.bsa"),
            ["A.BSA", "a.bsa", "B.bsa", "b.bsa"]
        );
        assert_eq!(resolve(&root, "A.bsa"), root.join("A.BSA"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A UTF-8 name is one character per original byte, and non-ASCII case folds, as on NTFS.
    #[test]
    fn non_ascii_names_match_by_character() {
        let root = std::env::temp_dir().join(format!("recomp_host_utf8_{}", std::process::id()));
        std::fs::create_dir_all(root.join("Данные")).unwrap();
        std::fs::write(root.join("Данные").join("Étude.bsa"), "").unwrap();
        std::fs::write(root.join("Данные").join("zeta.bsa"), "").unwrap();
        assert_eq!(
            resolve(&root, "данные\\étude.BSA"),
            root.join("Данные").join("Étude.bsa")
        );
        assert_eq!(find_files(&root, "ДАННЫЕ\\?tude.bsa"), ["Étude.bsa"]);
        assert_eq!(find_files(&root, "Данные\\é*.bsa"), ["Étude.bsa"]);
        assert_eq!(
            find_files(&root, "Данные\\*.bsa"),
            ["zeta.bsa", "Étude.bsa"],
            "sorted by uppercased code point"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A disk name that is not UTF-8 lifts one byte per character, so it round trips, and two such names stay apart.
    #[cfg(unix)]
    #[test]
    fn non_utf8_names_round_trip_and_do_not_collide() {
        use std::os::unix::ffi::OsStrExt;
        let root = std::env::temp_dir().join(format!("recomp_host_raw_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let raw = root.join(OsStr::from_bytes(b"\xE9tude.bsa"));
        std::fs::write(&raw, "").unwrap();
        std::fs::write(root.join(OsStr::from_bytes(b"\xFFother.bsa")), "").unwrap();
        assert_eq!(find_files(&root, "*tude.bsa"), ["étude.bsa"]);
        assert_eq!(find_files(&root, "?tude.bsa"), ["étude.bsa"]);
        assert_eq!(resolve(&root, "étude.bsa"), raw);
        assert_eq!(resolve(&root, "ÉTUDE.BSA"), raw);
        assert!(access(&root, "étude.bsa"));

        // Both of these were U+FFFD through a lossy comparison, which matched the wrong directory.
        std::fs::create_dir(root.join(OsStr::from_bytes(b"\xFE"))).unwrap();
        let absent = root.join(OsStr::from_bytes(b"\xFD")).join("x");
        assert_eq!(resolve(&absent, ""), absent, "no directory matches \\xFD");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn original_bytes_lower_the_lift() {
        assert_eq!(original_bytes("aé\u{FF}"), b"a\xE9\xFF");
        assert_eq!(original_bytes("я"), "я".as_bytes(), "no code page here");
        assert_eq!(original_length("aé\u{FF}"), 3);
    }
}
