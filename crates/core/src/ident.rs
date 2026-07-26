//! Identifier decoding — a named security component.
//!
//! The identifier arrives as one URI path segment, percent-encoded by the
//! client (spec: `/ ? # [ ] @ %` and all non-US-ASCII must be encoded).
//! Rules enforced here, in order:
//!
//! 1. exactly **one** percent-decode pass — the output is never re-decoded;
//! 2. strict encoding: `%` must introduce exactly two hex digits;
//! 3. the decoded bytes must be valid UTF-8 with no control characters;
//! 4. canonical-path traversal rejection: decoded `/` separates
//!    subdirectory segments, and no segment may be empty, `.`, or `..`;
//!    absolute paths, backslashes, and NUL never survive.
//!
//! The result is a relative path safe to join under a source root.

use std::fmt;

/// A decoded, traversal-checked identifier. The inner string is a relative
/// path (`a/b/c.tif` style) guaranteed free of `.`/`..`/empty segments,
/// control characters, and backslashes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identifier(String);

impl Identifier {
    /// Decode a raw (still percent-encoded) identifier path segment.
    ///
    /// # Errors
    ///
    /// Returns an [`IdentifierError`] (HTTP 404) for malformed encodings,
    /// non-UTF-8 or control bytes, backslashes, and any traversal shape.
    pub fn decode(raw: &str) -> Result<Self, IdentifierError> {
        if raw.is_empty() {
            return Err(IdentifierError::Empty);
        }
        // A raw '/' cannot appear: the router hands us one path segment.
        // A raw '\' is rejected outright — it is never legitimate in an
        // identifier and only exists to confuse path handling downstream.
        let mut bytes = Vec::with_capacity(raw.len());
        let mut it = raw.bytes();
        while let Some(b) = it.next() {
            if b == b'%' {
                let hi = it
                    .next()
                    .and_then(hex_val)
                    .ok_or(IdentifierError::BadEscape)?;
                let lo = it
                    .next()
                    .and_then(hex_val)
                    .ok_or(IdentifierError::BadEscape)?;
                bytes.push(hi * 16 + lo);
            } else {
                bytes.push(b);
            }
        }
        let decoded = String::from_utf8(bytes).map_err(|_| IdentifierError::NotUtf8)?;
        if decoded.bytes().any(|b| b < 0x20 || b == 0x7F) {
            return Err(IdentifierError::ControlCharacter);
        }
        if decoded.contains('\\') {
            return Err(IdentifierError::Backslash);
        }
        if decoded.starts_with('/') {
            return Err(IdentifierError::Traversal);
        }
        for segment in decoded.split('/') {
            if segment.is_empty() || segment == "." || segment == ".." {
                return Err(IdentifierError::Traversal);
            }
        }
        Ok(Self(decoded))
    }

    /// The decoded identifier as a root-relative path.
    #[must_use]
    pub fn as_path(&self) -> &str {
        &self.0
    }

    /// Re-encode for use in URIs we emit (canonical Link headers, info.json
    /// `id`): percent-encode `%`, the spec's to-encode set, and everything
    /// outside printable US-ASCII.
    #[must_use]
    pub fn encoded(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut out = String::with_capacity(self.0.len());
        for b in self.0.bytes() {
            let escape = match b {
                b'/' | b'?' | b'#' | b'[' | b']' | b'@' | b'%' => true,
                0x21..=0x7E => false,
                _ => true,
            };
            if escape {
                out.push('%');
                out.push(char::from(HEX[usize::from(b >> 4)]));
                out.push(char::from(HEX[usize::from(b & 0xF)]));
            } else {
                out.push(char::from(b));
            }
        }
        out
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Why an identifier was rejected. All variants map to 404 at the HTTP
/// layer (the spec has no finer distinction for bad identifiers, and a 400
/// here would leak which malformed shapes we distinguish).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierError {
    Empty,
    BadEscape,
    NotUtf8,
    ControlCharacter,
    Backslash,
    Traversal,
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            Self::Empty => "empty identifier",
            Self::BadEscape => "invalid percent-encoding",
            Self::NotUtf8 => "identifier is not valid UTF-8",
            Self::ControlCharacter => "identifier contains control characters",
            Self::Backslash => "identifier contains a backslash",
            Self::Traversal => "identifier contains path traversal",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for IdentifierError {}
