//! Failure information shared by the engine layers. The message is the original port diagnostic.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameErrorKind {
    /// Port execution stopped at an unimplemented arm.
    Stop,
    /// The port took a demonstrated failure path of the original executable.
    Original,
    /// A parser rejected its input.
    Stream,
    /// The evidence has yet to establish which kind of failure this is.
    Unclassified,
}

#[derive(Clone, Copy)]
pub struct GameError {
    pub kind: GameErrorKind,
    /// Original site when known. Zero means unknown.
    pub site: u32,
    pub message: &'static str,
}

impl GameError {
    /// IDA: N/A records an evidenced host stop.
    #[must_use]
    pub const fn stop(site: u32, message: &'static str) -> Self {
        Self {
            kind: GameErrorKind::Stop,
            site,
            message,
        }
    }

    /// IDA: N/A records a recovered original failure path.
    #[must_use]
    pub const fn original(site: u32, message: &'static str) -> Self {
        Self {
            kind: GameErrorKind::Original,
            site,
            message,
        }
    }

    /// IDA: N/A the parser owns this failure. Some reader callers also supply an original site.
    #[must_use]
    pub const fn stream(message: &'static str) -> Self {
        Self {
            kind: GameErrorKind::Stream,
            site: message_site(message),
            message,
        }
    }

    /// IDA: N/A an address identifies a site even when its failure semantics remain unresolved.
    #[must_use]
    pub const fn unclassified(site: u32, message: &'static str) -> Self {
        Self {
            kind: GameErrorKind::Unclassified,
            site,
            message,
        }
    }

    /// IDA: N/A recover an address prefix from a variable-provided legacy diagnostic.
    #[must_use]
    pub const fn from_legacy_message(message: &'static str) -> Self {
        Self::unclassified(message_site(message), message)
    }
}

// IDA: N/A the port's eight-hex-digit address prefix, followed by a colon, space or end of text.
const fn message_site(message: &str) -> u32 {
    let bytes = message.as_bytes();
    if bytes.len() < 8 || (bytes.len() > 8 && !matches!(bytes[8], b':' | b' ')) {
        return 0;
    }
    let mut site = 0;
    let mut i = 0;
    while i < 8 {
        let digit = match bytes[i] {
            byte @ b'0'..=b'9' => byte - b'0',
            byte @ b'A'..=b'F' => byte - b'A' + 10,
            byte @ b'a'..=b'f' => byte - b'a' + 10,
            _ => return 0,
        };
        site = (site << 4) | digit as u32;
        i += 1;
    }
    site
}

impl PartialEq for GameError {
    // IDA: N/A existing handler and replay comparisons use the diagnostic text alone.
    fn eq(&self, other: &Self) -> bool {
        self.message == other.message
    }
}

impl Eq for GameError {}

impl fmt::Display for GameError {
    // IDA: N/A preserve the text written to rows and logs.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl fmt::Debug for GameError {
    // IDA: N/A callers also write the former tuple's Debug form to diagnostic output.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("StreamError")
            .field(&self.message)
            .finish()
    }
}

impl std::error::Error for GameError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::ByteReader;

    #[test]
    fn classification_preserves_legacy_diagnostics_and_comparisons() {
        let text = "0044DB24: a \"quoted\" diagnostic";
        let pending = GameError::from_legacy_message(text);
        let original = GameError::original(0x0044_DB24, text);
        let stop = GameError::stop(0x0044_DB24, text);
        assert_eq!(pending.kind, GameErrorKind::Unclassified);
        assert_eq!(pending.site, 0x0044_DB24);
        assert_eq!(original.kind, GameErrorKind::Original);
        assert_eq!(stop.kind, GameErrorKind::Stop);
        assert_eq!(stop, original);
        assert_eq!(stop, pending);
        assert_eq!(format!("{stop}"), text);
        assert_eq!(format!("{stop:?}"), format!("StreamError({text:?})"));
        assert_ne!(stop, GameError::unclassified(stop.site, "different text"));
        assert_eq!(GameError::from_legacy_message("details 0044DB24").site, 0);
        assert_eq!(GameError::from_legacy_message("0044db24x").site, 0);
        assert_eq!(GameError::from_legacy_message("0044db24").site, stop.site);
    }

    #[test]
    fn a_short_read_keeps_its_site_message_and_cursor() {
        let mut reader = ByteReader::new(&[1]);
        let text = "006F1234 truncated word";
        let error = reader.u16(text).unwrap_err();
        assert_eq!(
            (error.kind, error.site, error.message),
            (GameErrorKind::Stream, 0x006F_1234, text)
        );
        assert_eq!(reader.position(), 0);
        assert_eq!(GameError::stream("truncated word").site, 0);
    }
}
