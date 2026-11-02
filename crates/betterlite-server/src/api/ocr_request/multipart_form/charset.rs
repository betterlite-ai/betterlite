use encoding_rs::{Encoding, GBK};

// codecs.lookup resolves these aliases to "ascii" or "latin_1" and both decode as pure byte-identity in Python, unlike encoding_rs's windows-1252 mapping for the same WHATWG labels.
const ASCII_OR_LATIN1_LABELS: &[&str] = &[
    "ascii",
    "646",
    "ansi_x3.4_1968",
    "ansi_x3_4_1968",
    "ansi_x3.4_1986",
    "cp367",
    "csascii",
    "ibm367",
    "iso646_us",
    "iso_646.irv_1991",
    "iso_ir_6",
    "us",
    "us_ascii",
    "latin_1",
    "8859",
    "cp819",
    "csisolatin1",
    "ibm819",
    "iso8859",
    "iso8859_1",
    "iso_8859_1",
    "iso_8859_1_1987",
    "iso_ir_100",
    "l1",
    "latin",
    "latin1",
];

// codecs.lookup resolves these to Python's iso8859_9, whose 0x00-0x9F range is byte-identical and whose 0xA0-0xFF range differs from latin-1 at exactly six Turkish letters, unlike encoding_rs's windows-1254 mapping for the same WHATWG labels.
const ISO8859_9_LABELS: &[&str] = &[
    "iso8859_9",
    "csisolatin5",
    "iso_8859_9",
    "iso_8859_9_1989",
    "iso_ir_148",
    "l5",
    "latin5",
];

// codecs.lookup resolves these to Python's iso8859_11, which keeps 0x00-0x9F byte-identical and maps 0xA0 to U+00A0, unlike tis_620 below.
const ISO8859_11_LABELS: &[&str] = &["iso8859_11", "iso_8859_11", "iso_8859_11_2001", "thai"];

// codecs.lookup resolves these to Python's tis_620, which agrees with iso8859_11 everywhere except that 0xA0 itself is undefined rather than U+00A0.
const TIS_620_LABELS: &[&str] = &[
    "tis_620",
    "iso_ir_166",
    "tis620",
    "tis_620_0",
    "tis_620_2529_0",
    "tis_620_2529_1",
];

// codecs.lookup resolves these to Python's strict gb2312, which rejects the GBK-only lead/trail bytes that encoding_rs's gb2312 label (an alias for GBK) accepts.
const GB2312_LABELS: &[&str] = &[
    "gb2312",
    "chinese",
    "csiso58gb231280",
    "euc_cn",
    "euccn",
    "eucgb2312_cn",
    "gb2312_1980",
    "gb2312_80",
    "iso_ir_58",
    "x_mac_simp_chinese",
];

// Python's encodings.normalize_encoding: keep ASCII letters, digits and '.', lowercased, collapsing any run of other characters into a single '_' between kept characters.
fn normalize_charset_label(charset: &str) -> String {
    let mut out = String::with_capacity(charset.len());
    let mut pending_separator = false;
    for ch in charset.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' {
            if pending_separator && !out.is_empty() {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            pending_separator = false;
        } else {
            pending_separator = true;
        }
    }
    out
}

// Python's iso8859_9 differs from latin-1 at exactly these six Turkish letters and never fails to decode a byte, unlike encoding_rs's windows-1254 mapping, which leaves several bytes in 0x80-0x9F undefined.
fn decode_iso8859_9(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&byte| match byte {
            0xD0 => '\u{11E}',
            0xDD => '\u{130}',
            0xDE => '\u{15E}',
            0xF0 => '\u{11F}',
            0xFD => '\u{131}',
            0xFE => '\u{15F}',
            other => char::from(other),
        })
        .collect()
}

// Python's iso8859_11 and tis_620 keep every byte below 0xA0 byte-identical and map 0xA1-0xDA and 0xDF-0xFB onto the Thai block starting at U+0E00; 0xDB-0xDE and 0xFC-0xFF are undefined in both, and 0xA0 itself is U+00A0 in iso8859_11 only.
fn decode_thai(bytes: &[u8], non_breaking_space_at_a0: bool) -> Option<String> {
    let mut decoded = String::with_capacity(bytes.len());
    for &byte in bytes {
        let ch = match byte {
            0x00..=0x9F => char::from(byte),
            0xA0 if non_breaking_space_at_a0 => '\u{A0}',
            0xA1..=0xDA | 0xDF..=0xFB => {
                char::from_u32(0x0E00_u32.saturating_add(u32::from(byte.saturating_sub(0xA0))))?
            }
            _ => return None,
        };
        decoded.push(ch);
    }
    Some(decoded)
}

// Strict GB2312 only assigns the 94x94 grid of lead bytes 0xA1-0xF7 paired with trail bytes 0xA1-0xFE; GBK (what encoding_rs's gb2312 label actually decodes) also assigns lead bytes 0x81-0xA0 and 0xF8-0xFE, which strict GB2312 rejects as undefined.
fn is_strict_gb2312(bytes: &[u8]) -> bool {
    let mut index = 0_usize;
    while let Some(&lead) = bytes.get(index) {
        if lead < 0x80 {
            index = index.saturating_add(1);
        } else if (0xA1..=0xF7).contains(&lead) {
            match bytes.get(index.saturating_add(1)) {
                Some(&trail) if (0xA1..=0xFE).contains(&trail) => {
                    index = index.saturating_add(2);
                }
                _ => return false,
            }
        } else {
            return false;
        }
    }
    true
}

fn decode_gb2312(bytes: &[u8]) -> String {
    if !is_strict_gb2312(bytes) {
        return bytes.iter().copied().map(char::from).collect();
    }
    let (decoded, had_errors) = GBK.decode_without_bom_handling(bytes);
    if had_errors {
        bytes.iter().copied().map(char::from).collect()
    } else {
        decoded.into_owned()
    }
}

// _user_safe_decode: decode with the charset, falling back to latin-1 (which never fails) on an unrecognized charset or a decode error.
pub(super) fn decode_part(bytes: &[u8], charset: &str) -> String {
    let label = normalize_charset_label(charset);
    if ASCII_OR_LATIN1_LABELS.contains(&label.as_str()) {
        return bytes.iter().copied().map(char::from).collect();
    }
    if ISO8859_9_LABELS.contains(&label.as_str()) {
        return decode_iso8859_9(bytes);
    }
    if ISO8859_11_LABELS.contains(&label.as_str()) {
        return decode_thai(bytes, true)
            .unwrap_or_else(|| bytes.iter().copied().map(char::from).collect());
    }
    if TIS_620_LABELS.contains(&label.as_str()) {
        return decode_thai(bytes, false)
            .unwrap_or_else(|| bytes.iter().copied().map(char::from).collect());
    }
    if GB2312_LABELS.contains(&label.as_str()) {
        return decode_gb2312(bytes);
    }
    Encoding::for_label(charset.as_bytes()).map_or_else(
        || bytes.iter().copied().map(char::from).collect(),
        |encoding| {
            let (decoded, had_errors) = encoding.decode_without_bom_handling(bytes);
            if had_errors {
                bytes.iter().copied().map(char::from).collect()
            } else {
                decoded.into_owned()
            }
        },
    )
}
