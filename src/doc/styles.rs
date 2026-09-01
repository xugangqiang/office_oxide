//! Style sheet (STSH) parsing for legacy binary `.doc` (MS-DOC §2.7.1).
//!
//! The style sheet maps a paragraph's style index (`istd`) to a built-in
//! style id (`sti`) and a name. Built-in heading styles carry `sti` 1–9
//! (Heading 1–9); user-defined heading styles are named `Heading N`. Both let
//! us derive a paragraph's real heading level instead of the line heuristic in
//! `convert_doc.rs`.
//!
//! Every parse step is bounds-checked: a malformed or truncated style sheet
//! yields an empty `Vec`, so callers degrade to "no style" (and the heuristic)
//! rather than panicking (AGENTS.md rule 6).

use super::fib::Fib;

/// One style definition, indexed by `istd`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StyleDef {
    /// Built-in style id (`StdfBase.sti`). `0x0FFE` means user-defined.
    pub sti: u16,
    /// Style name (from the style-name STTB).
    pub name: String,
}

/// Parse the document style sheet (STSH) from the Table stream.
///
/// Returns an empty vector when the style sheet is absent (`fcStshf == 0`),
/// out of bounds, or malformed.
pub fn parse_style_sheet(table_stream: &[u8], fib: &Fib) -> Vec<StyleDef> {
    if fib.fc_stshf == 0 || fib.lcb_stshf == 0 {
        return Vec::new();
    }
    let start = fib.fc_stshf as usize;
    let end = (start + fib.lcb_stshf as usize).min(table_stream.len());
    if start >= table_stream.len() || end <= start {
        return Vec::new();
    }
    parse_stsh(&table_stream[start..end])
}

/// Resolve a paragraph's `istd` (optionally overridden by `sprmPStyle`) to a
/// heading level (1–9), or `None` when the style is not a heading.
pub fn heading_level_for_istd(styles: &[StyleDef], istd: u16) -> Option<u8> {
    let s = styles.get(istd as usize)?;
    // Built-in heading styles: `sti` 1..9 == Heading 1..9.
    if (1..=9).contains(&s.sti) {
        return Some(s.sti as u8);
    }
    // User-defined heading styles are named "Heading N".
    if let Some(level) = heading_level_from_name(&s.name) {
        return Some(level);
    }
    None
}

fn heading_level_from_name(name: &str) -> Option<u8> {
    // Real Word style names are "Heading N" (capital H); match case-insensitively
    // so "heading 2" / "HEADING 2" also resolve.
    let lowered: String = name.trim().to_ascii_lowercase();
    let rest = lowered.strip_prefix("heading ")?;
    let level: u8 = rest.trim().parse().ok()?;
    if (1..=9).contains(&level) {
        Some(level)
    } else {
        None
    }
}

/// `data` is the `stshf` slice (the STSH). Layout (MS-DOC §2.7.1):
/// `cbStshi(u16)` + `Stshif(cbStshi)` + style-name `STTB` + `cstd` `LPStd`
/// (each `cbStd(u16)` + `Stdf`).
fn parse_stsh(data: &[u8]) -> Vec<StyleDef> {
    // `cbStshi` then `Stshif`.
    if data.len() < 2 {
        return Vec::new();
    }
    let cb_stshi = u16::from_le_bytes([data[0], data[1]]) as usize;
    let mut pos = 2;
    if cb_stshi < 18 || pos + cb_stshi > data.len() {
        return Vec::new();
    }
    // Stshif: `cstd` at offset 0 (u16).
    let cstd = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
    pos += cb_stshi;

    // Style-name STTB.
    let (names, next) = match parse_sttb(data, pos) {
        Some(v) => v,
        None => return Vec::new(),
    };
    pos = next;

    // Style-definition array: `cstd` `LPStd` entries, each `cbStd(u16)` + `Stdf`.
    let cap = cstd.min(4096);
    let mut styles = Vec::with_capacity(cap);
    for _ in 0..cap {
        if pos + 2 > data.len() {
            break;
        }
        let cb_std = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        if cb_std == 0 {
            // Empty style (fixed-index slots MAY be empty).
            styles.push(StyleDef::default());
            continue;
        }
        if pos + cb_std > data.len() {
            break;
        }
        let std = &data[pos..pos + cb_std];
        pos += cb_std;
        // `StdfBase.sti` is the low 12 bits of the first u16.
        let sti = if std.len() >= 2 {
            u16::from_le_bytes([std[0], std[1]]) & 0x0FFF
        } else {
            0
        };
        styles.push(StyleDef {
            sti,
            name: String::new(),
        });
    }

    // Names and definitions are both indexed by `istd`; attach by index.
    for (i, s) in styles.iter_mut().enumerate() {
        if let Some(n) = names.get(i) {
            s.name = n.clone();
        }
    }
    styles
}

/// Parse an `STTB` (§2.4.1) starting at `pos`, returning the strings and the
/// offset just past the table (including its trailing null entry).
fn parse_sttb(data: &[u8], mut pos: usize) -> Option<(Vec<String>, usize)> {
    if pos + 3 > data.len() {
        return None;
    }
    let f2 = data[pos]; // 1 => 2-byte counts, 0 => 1-byte counts
    pos += 1;
    let c_data = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2;
    let count_len = if f2 != 0 { 2 } else { 1 };
    let (cb_data, mut pos) = if f2 != 0 {
        (u16::from_le_bytes([data[pos], data[pos + 1]]) as usize, pos + 2)
    } else {
        (data[pos] as usize, pos + 1)
    };

    let cap = c_data.min(4096);
    let mut names = Vec::with_capacity(cap);
    for _ in 0..cap {
        if pos + count_len > data.len() {
            return None;
        }
        let cch = if f2 != 0 {
            let v = u16::from_le_bytes([data[pos], data[pos + 1]]);
            pos += 2;
            v as usize
        } else {
            let v = data[pos];
            pos += 1;
            v as usize
        };
        let str_bytes = cch.saturating_mul(2);
        if pos + str_bytes > data.len() {
            return None;
        }
        let units: Vec<u16> = (0..cch)
            .map(|i| u16::from_le_bytes([data[pos + 2 * i], data[pos + 2 * i + 1]]))
            .collect();
        pos += str_bytes;
        if pos + cb_data > data.len() {
            return None;
        }
        pos += cb_data; // skip per-entry extra data
        names.push(String::from_utf16_lossy(&units));
    }

    // Trailing null string entry (cch = 0).
    if pos + count_len <= data.len() {
        pos += count_len;
    }
    Some((names, pos))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal but well-formed STSH: 3 styles — `Normal` (sti 0),
    /// `Body Text` (sti 2), and a user-defined `Heading 2` (sti 0x0FFE).
    fn synthetic_stsh() -> Vec<u8> {
        let mut d = Vec::new();
        // cbStshi = 18
        d.extend_from_slice(&18u16.to_le_bytes());
        // Stshif: cstd = 3, cbSTDBaseInFile = 10, rest 0.
        d.extend_from_slice(&3u16.to_le_bytes());
        d.extend_from_slice(&10u16.to_le_bytes());
        d.extend_from_slice(&[0u8; 14]); // remainder of the 18-byte Stshif

        // Style-name STTB (f2 = 1, cData = 3, cbData = 0).
        d.push(1); // f2
        d.extend_from_slice(&3u16.to_le_bytes()); // cData
        d.extend_from_slice(&0u16.to_le_bytes()); // cbData
        // entry 0: "Normal"
        let name0 = "Normal";
        d.extend_from_slice(&(name0.len() as u16).to_le_bytes());
        for c in name0.encode_utf16() {
            d.extend_from_slice(&c.to_le_bytes());
        }
        // entry 1: "Heading 1"
        let name1 = "Heading 1";
        d.extend_from_slice(&(name1.len() as u16).to_le_bytes());
        for c in name1.encode_utf16() {
            d.extend_from_slice(&c.to_le_bytes());
        }
        // entry 2: "Heading 2"
        let name2 = "Heading 2";
        d.extend_from_slice(&(name2.len() as u16).to_le_bytes());
        for c in name2.encode_utf16() {
            d.extend_from_slice(&c.to_le_bytes());
        }
        // trailing null string (cch = 0, 2 bytes)
        d.extend_from_slice(&0u16.to_le_bytes());

        // `cstd` = 3 LPStd entries, each cbStd = 10 + StdfBase.
        // entry 0: sti = 0 (Normal)
        d.extend_from_slice(&10u16.to_le_bytes());
        d.extend_from_slice(&[0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
        // entry 1: sti = 1 (built-in Heading 1)
        d.extend_from_slice(&10u16.to_le_bytes());
        d.extend_from_slice(&[0x01, 0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
        // entry 2: sti = 0x0FFE (user-defined heading, name carries the level)
        d.extend_from_slice(&10u16.to_le_bytes());
        d.extend_from_slice(&[0xFE, 0x0F, 0, 0, 0, 0, 0, 0, 0, 0]);

        d
    }

    #[test]
    fn parse_stsh_reads_sti_and_names() {
        let styles = parse_stsh(&synthetic_stsh());
        assert_eq!(styles.len(), 3);
        assert_eq!(styles[0].sti, 0);
        assert_eq!(styles[0].name, "Normal");
        assert_eq!(styles[1].sti, 1);
        assert_eq!(styles[1].name, "Heading 1");
        // User-defined heading: sti 0x0FFE, name "Heading 2".
        assert_eq!(styles[2].sti, 0x0FFE);
        assert_eq!(styles[2].name, "Heading 2");
    }

    #[test]
    fn heading_level_resolves_builtin_and_user() {
        let styles = parse_stsh(&synthetic_stsh());
        // sti 1..9 resolve directly.
        assert_eq!(heading_level_for_istd(&styles, 1), Some(1));
        // sti 0x0FFE named "Heading 2" resolves via name.
        assert_eq!(heading_level_for_istd(&styles, 2), Some(2));
        // sti 0 (Normal) is not a heading.
        assert_eq!(heading_level_for_istd(&styles, 0), None);
        // out-of-range istd.
        assert_eq!(heading_level_for_istd(&styles, 99), None);
    }

    #[test]
    fn truncated_style_sheet_is_empty() {
        assert!(parse_stsh(&[0u8; 4]).is_empty());
        assert!(parse_stsh(&[18, 0, 0, 0]).is_empty());
    }
}
