fn dequote(value: &[u8]) -> Vec<u8> {
    // parse_options_header unescapes a quoted value with two sequential, non-overlapping global replaces, not a single left-to-right scan, which matters for a run of backslashes before a quote.
    match value {
        [b'"', middle @ .., b'"'] => {
            replace_bytes(&replace_bytes(middle, b"\\\\", b"\\"), b"\\\"", b"\"")
        }
        _ => value.to_vec(),
    }
}

fn replace_bytes(input: &[u8], pattern: &[u8], replacement: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut remaining = input;
    while !remaining.is_empty() {
        if remaining.starts_with(pattern) {
            out.extend_from_slice(replacement);
            remaining = remaining.get(pattern.len()..).unwrap_or(&[]);
        } else {
            let (byte, rest) = remaining.split_at(1);
            out.extend_from_slice(byte);
            remaining = rest;
        }
    }
    out
}

// python-multipart's IE6 workaround: a filename sent as a full Windows or UNC path keeps only the last path segment.
fn strip_ie_path(value: Vec<u8>) -> Vec<u8> {
    let is_windows_path = matches!(value.as_slice(), [_, b':', b'\\', ..]);
    let is_unc_path = matches!(value.as_slice(), [b'\\', b'\\', ..]);
    if is_windows_path || is_unc_path {
        value
            .rsplit(|&byte| byte == b'\\')
            .next()
            .map(<[u8]>::to_vec)
            .unwrap_or(value)
    } else {
        value
    }
}

pub(super) fn disposition_param_bytes(disposition: &[u8], key: &str) -> Option<Vec<u8>> {
    disposition
        .split(|&byte| byte == b';')
        .map(<[u8]>::trim_ascii)
        .find_map(|segment| {
            let equals = segment.iter().position(|&byte| byte == b'=')?;
            let (segment_key, rest) = segment.split_at(equals);
            let segment_value = rest.get(1..).unwrap_or(&[]);
            if segment_key != key.as_bytes() {
                return None;
            }
            let value = dequote(segment_value);
            Some(if key == "filename" {
                strip_ie_path(value)
            } else {
                value
            })
        })
}
