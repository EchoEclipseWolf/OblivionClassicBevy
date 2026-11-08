//! IDA: N/A host diagnostics: level filtering, `OblivionRecompSettings.ini`, console output.
//! Diagnostics never change game behaviour. Completion: 100% of the C++ host's behaviour.

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::profile;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    Verbose,
    Debug,
    Info,
    Warning,
    Error,
}

impl Level {
    const ALL: [Self; 5] = [
        Self::Verbose,
        Self::Debug,
        Self::Info,
        Self::Warning,
        Self::Error,
    ];

    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Verbose => "VERBOSE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warning => "WARNING",
            Self::Error => "ERROR",
        }
    }

    // IDA: N/A a helper, with no address of its own.
    fn parse(text: &[u8]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|level| level.name().as_bytes().eq_ignore_ascii_case(text))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub level: Level,
    pub created: bool,
    pub invalid_level: bool,
}

const DEFAULT_CONTENTS: &str = "; OblivionRecomp console logging level: Verbose, Debug, Info, Warning, Error\r\n\
                                [Logging]\r\nLevel=Info\r\n";

static THRESHOLD: AtomicU8 = AtomicU8::new(Level::Info as u8);

/// IDA: N/A a helper, with no address of its own.
/// Creates the file once and never overwrites an existing one.
pub fn load_or_create_settings(file: &Path) -> std::io::Result<Settings> {
    let created = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)
    {
        Ok(mut new_file) => {
            new_file.write_all(DEFAULT_CONTENTS.as_bytes())?;
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => return Err(error),
    };
    let parsed = Level::parse(&profile::read_string(
        file,
        b"Logging",
        b"Level",
        Some(b"Info"),
        64,
    ));
    Ok(Settings {
        level: parsed.unwrap_or(Level::Info),
        created,
        invalid_level: parsed.is_none(),
    })
}

/// IDA: N/A a helper, with no address of its own.
pub fn set_level(level: Level) {
    THRESHOLD.store(level as u8, Ordering::Relaxed);
}

/// IDA: N/A a helper, with no address of its own.
#[must_use]
pub fn level() -> Level {
    Level::ALL[usize::from(THRESHOLD.load(Ordering::Relaxed))]
}

/// IDA: N/A a helper, with no address of its own.
#[must_use]
pub fn enabled(severity: Level) -> bool {
    severity >= level()
}

/// IDA: N/A a helper, with no address of its own.
/// Writes one line to stderr, colouring warnings and errors on a terminal. Failures are ignored.
pub fn emit(severity: Level, message: &str) {
    if !enabled(severity) {
        return;
    }
    let mut stderr = std::io::stderr().lock();
    let colour = match severity {
        Level::Warning if stderr.is_terminal() => "\x1b[93m",
        Level::Error if stderr.is_terminal() => "\x1b[91m",
        _ => "",
    };
    let reset = if colour.is_empty() { "" } else { "\x1b[0m" };
    let _ = writeln!(stderr, "{colour}[{}] {message}{reset}", severity.name());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from `logging_settings`.
    #[test]
    fn settings_are_created_once_and_parsed() {
        let directory = std::env::temp_dir().join(format!("recomp_logging_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("OblivionRecompSettings.ini");
        let _ = std::fs::remove_file(&file);

        let first = load_or_create_settings(&file).unwrap();
        assert!(first.created && !first.invalid_level);
        assert_eq!(first.level, Level::Info);
        for (text, expected) in [
            ("Verbose", Level::Verbose),
            ("Debug", Level::Debug),
            ("Info", Level::Info),
            ("Warning", Level::Warning),
            ("Error", Level::Error),
            ("vErBoSe", Level::Verbose),
        ] {
            std::fs::write(&file, format!("[Logging]\nLevel={text}\n")).unwrap();
            let setting = load_or_create_settings(&file).unwrap();
            assert!(!setting.created && !setting.invalid_level);
            assert_eq!(setting.level, expected);
        }
        std::fs::write(&file, "[Logging]\nLevel=broken\n").unwrap();
        let invalid = load_or_create_settings(&file).unwrap();
        assert!(invalid.invalid_level);
        assert_eq!(invalid.level, Level::Info);

        set_level(Level::Warning);
        assert!(!enabled(Level::Debug) && enabled(Level::Warning) && enabled(Level::Error));
        set_level(Level::Info);
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
