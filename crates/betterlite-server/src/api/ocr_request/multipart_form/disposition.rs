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

// A ';' only ends a parameter segment when the quotes seen since the previous segment start, minus escaped quotes, are balanced: _parseparam (python-multipart's vendored copy of email.message._parseparam) counts quotes the same way to skip a ';' inside a quoted value such as filename="report;v2.pdf".
fn quote_parity_is_odd(bytes: &[u8], start: usize, end: usize) -> bool {
    let window = bytes.get(start..end).unwrap_or(&[]);
    let quotes = window.iter().fold(0_usize, |count, &byte| {
        if byte == b'"' {
            count.saturating_add(1)
        } else {
            count
        }
    });
    let escaped_quotes = window.windows(2).fold(0_usize, |count, pair| {
        if pair == b"\\\"" {
            count.saturating_add(1)
        } else {
            count
        }
    });
    (quotes & 1) != (escaped_quotes & 1)
}

fn find_semicolon_from(bytes: &[u8], start: usize) -> Option<usize> {
    bytes
        .get(start..)?
        .iter()
        .position(|&byte| byte == b';')
        .map(|offset| offset.saturating_add(start))
}

// Splits a Content-Disposition value the way _parseparam does: on ';', except inside a quoted segment, with each key lowercased and both the key and value trimmed.
fn parse_params(disposition: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut params = Vec::new();
    let mut start = 0_usize;
    while start < disposition.len() {
        let mut end = find_semicolon_from(disposition, start);
        while let Some(semicolon) = end {
            if semicolon <= start || !quote_parity_is_odd(disposition, start, semicolon) {
                break;
            }
            end = find_semicolon_from(disposition, semicolon.saturating_add(1));
        }
        let segment_end = end.unwrap_or(disposition.len());
        let segment = disposition.get(start..segment_end).unwrap_or(&[]);
        let entry = segment.iter().position(|&byte| byte == b'=').map_or_else(
            || (segment.trim_ascii().to_ascii_lowercase(), Vec::new()),
            |equals| {
                let (key, rest) = segment.split_at(equals);
                let value = rest.get(1..).unwrap_or(&[]);
                (
                    key.trim_ascii().to_ascii_lowercase(),
                    value.trim_ascii().to_vec(),
                )
            },
        );
        params.push(entry);
        start = segment_end.saturating_add(1);
    }
    params
}

pub(super) fn disposition_param_bytes(disposition: &[u8], key: &str) -> Option<Vec<u8>> {
    let key_bytes = key.as_bytes();
    let mut result = None;
    for (segment_key, segment_value) in parse_params(disposition) {
        if segment_key == key_bytes {
            let value = dequote(&segment_value);
            result = Some(if key == "filename" {
                strip_ie_path(value)
            } else {
                value
            });
        }
    }
    result
}
