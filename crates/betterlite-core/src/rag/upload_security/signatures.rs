use super::DisallowedKind;

const ARCHIVE: &[&[u8]] = &[
    b"PK\x03\x04",
    b"PK\x05\x06",
    b"PK\x07\x08",
    b"\x1f\x8b",
    b"\xfd7zXZ\0",
    b"7z\xbc\xaf'\x1c",
    b"Rar!\x1a\x07\0",
    b"Rar!\x1a\x07\x01\0",
    b"\x04\x22\x4d\x18",
    b"\x28\xb5\x2f\xfd",
];
const EXECUTABLE: &[&[u8]] = &[
    b"\x7fELF",
    b"\xca\xfe\xba\xbe",
    b"\xfe\xed\xfa\xce",
    b"\xfe\xed\xfa\xcf",
    b"\xce\xfa\xed\xfe",
    b"\xcf\xfa\xed\xfe",
    b"\0asm",
];

pub(super) fn shebang(content: &[u8]) -> bool {
    let body = content.strip_prefix(b"\xef\xbb\xbf").unwrap_or(content);
    // Python bytes.lstrip includes vertical tab, unlike Rust's ASCII whitespace predicate.
    let offset = body
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
        .unwrap_or(body.len());
    body.get(offset..)
        .is_some_and(|trimmed| trimmed.starts_with(b"#!"))
}

pub(super) fn binary_kind(content: &[u8], is_text: bool) -> Option<DisallowedKind> {
    if ARCHIVE.iter().any(|prefix| content.starts_with(prefix))
        || content.get(257..262) == Some(b"ustar")
        || (content.starts_with(b"BZh") && !is_text)
    {
        Some(DisallowedKind::Archive)
    } else if EXECUTABLE.iter().any(|prefix| content.starts_with(prefix))
        || ((content.starts_with(b"MZ") || content.starts_with(b"dex\n")) && !is_text)
    {
        Some(DisallowedKind::Executable)
    } else {
        None
    }
}
