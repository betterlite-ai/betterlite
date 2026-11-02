use encoding_rs::Encoding;

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

// _user_safe_decode: decode with the charset, falling back to latin-1 (which never fails) on an unrecognized charset or a decode error.
pub(super) fn decode_part(bytes: &[u8], charset: &str) -> String {
    let label = normalize_charset_label(charset);
    if ASCII_OR_LATIN1_LABELS.contains(&label.as_str()) {
        return bytes.iter().copied().map(char::from).collect();
    }
    if ISO8859_9_LABELS.contains(&label.as_str()) {
        return decode_iso8859_9(bytes);
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
