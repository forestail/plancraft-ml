//! Files embedded in notes ("Notes ▸ Insert Object ▸ Create from file" in Microsoft Project).
//!
//! The file content is stored inside the project file (base64 in JSON), so the project can be
//! moved or e-mailed with its documentation.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Largest file that can be embedded (bytes).
pub const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;
/// Most files embedded in one note.
pub const MAX_ATTACHMENTS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Attachment {
    /// File name shown in the note (no directories).
    pub name: String,
    #[serde(with = "b64")]
    pub data: Vec<u8>,
}

impl Attachment {
    /// Reads a file to embed it.
    pub fn from_file(path: &Path) -> Result<Attachment, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("Could not read {}: {}", path.display(), e))?;
        if meta.len() > MAX_ATTACHMENT_BYTES as u64 {
            return Err(format!("{} is larger than 25 MB", path.display()));
        }
        let data = std::fs::read(path).map_err(|e| format!("Could not read {}: {}", path.display(), e))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        Ok(Attachment { name: clean_name(&name), data })
    }

    /// Human-readable size ("12 KB").
    pub fn size_label(&self) -> String {
        let n = self.data.len() as f64;
        if n < 1024.0 {
            format!("{} {}", self.data.len(), crate::i18n::tr("bytes"))
        } else if n < 1024.0 * 1024.0 {
            format!("{:.0} {}", n / 1024.0, crate::i18n::tr("KB"))
        } else {
            format!("{:.1} {}", n / 1024.0 / 1024.0, crate::i18n::tr("MB"))
        }
    }

    /// Writes the file into `dir` (created if needed) and returns its path.
    pub fn extract_to(&self, dir: &Path) -> std::io::Result<std::path::PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(clean_name(&self.name));
        std::fs::write(&path, &self.data)?;
        Ok(path)
    }
}

/// Keeps only the file name and drops characters that are not allowed in file names.
pub fn clean_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let s: String = base.chars().filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') && !c.is_control()).collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "file".into() } else { s.chars().take(200).collect() }
}

/// Standard base64 (RFC 4648) without external crates.
pub mod b64 {
    use serde::{Deserialize, Deserializer, Serializer};

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b0 = chunk.first().copied().unwrap_or(0) as u32;
            let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
            let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;
            let ch = |shift: u32| ALPHABET.get(((n >> shift) & 63) as usize).copied().unwrap_or(b'A') as char;
            out.push(ch(18));
            out.push(ch(12));
            out.push(if chunk.len() > 1 { ch(6) } else { '=' });
            out.push(if chunk.len() > 2 { ch(0) } else { '=' });
        }
        out
    }

    pub fn decode(s: &str) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(s.len() / 4 * 3);
        let mut acc: u32 = 0;
        let mut bits = 0;
        for c in s.bytes() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' | b'-' => 62,
                b'/' | b'_' => 63,
                b'=' | b'\r' | b'\n' | b' ' => continue,
                _ => return Err("invalid base64 data".into()),
            };
            acc = (acc << 6) | u32::from(v);
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push(((acc >> bits) & 0xFF) as u8);
            }
        }
        Ok(out)
    }

    pub fn serialize<S: Serializer>(data: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode(data))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        decode(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0u8, 255, 128, 7, 9]] {
            let e = b64::encode(data);
            assert_eq!(b64::decode(&e).unwrap(), data);
        }
        assert_eq!(b64::encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(b64::encode(b"fo"), "Zm8=");
    }

    #[test]
    fn attachment_serializes_as_text() {
        let a = Attachment { name: "отчёт.docx".into(), data: vec![1, 2, 3, 250] };
        let j = serde_json::to_string(&a).unwrap();
        assert!(j.contains("AQID+g=="));
        let b: Attachment = serde_json::from_str(&j).unwrap();
        assert_eq!(a, b);
        assert_eq!(clean_name("../../etc/passwd"), "passwd");
        assert_eq!(clean_name("C:\\docs\\a?.txt"), "a.txt");
    }
}
