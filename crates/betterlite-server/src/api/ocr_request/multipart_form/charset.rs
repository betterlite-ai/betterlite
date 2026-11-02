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

// _user_safe_decode: decode with the charset, falling back to latin-1 (which never fails) on an unrecognized charset or a decode error.
pub(super) fn decode_part(bytes: &[u8], charset: &str) -> String {
    if ASCII_OR_LATIN1_LABELS.contains(&normalize_charset_label(charset).as_str()) {
        return bytes.iter().copied().map(char::from).collect();
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
