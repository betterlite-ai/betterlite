use std::fmt::Write as _;
use std::str::from_utf8;

// Mirrors `urllib.parse.urlsplit`/`urlunsplit`/`parse_qsl`/`urlencode` for the
// postgres/postgresql schemes this task owns: those schemes never appear in
// urllib's `uses_netloc` or `uses_params` tables, so the parts of `urlsplit`
// that consult those tables (semicolon params, forced "//" on an empty
// netloc) never fire here and are left out.

pub(super) struct SplitUrl {
    pub(super) scheme: String,
    pub(super) netloc: String,
    pub(super) path: String,
    pub(super) query: String,
    pub(super) fragment: String,
}

pub(super) fn urlsplit(url: &str) -> SplitUrl {
    let trimmed = url.trim_start_matches(|character: char| u32::from(character) <= 0x20);
    let cleaned: String = trimmed
        .chars()
        .filter(|character| !matches!(character, '\t' | '\r' | '\n'))
        .collect();
    let mut rest = cleaned.as_str();
    let mut scheme = String::new();
    if let Some((prefix, remainder)) = rest.split_once(':') {
        let starts_alpha = prefix.starts_with(|character: char| character.is_ascii_alphabetic());
        if starts_alpha && prefix.bytes().all(is_scheme_byte) {
            scheme = prefix.to_ascii_lowercase();
            rest = remainder;
        }
    }
    let mut netloc = String::new();
    if let Some(after) = rest.strip_prefix("//") {
        if let Some(end) = after.find(['/', '?', '#']) {
            match after.split_at_checked(end) {
                Some((host, tail)) => {
                    host.clone_into(&mut netloc);
                    rest = tail;
                }
                None => rest = after,
            }
        } else {
            after.clone_into(&mut netloc);
            rest = "";
        }
    }
    let (before_fragment, fragment) = rest.split_once('#').unwrap_or((rest, ""));
    let (path, query) = before_fragment
        .split_once('?')
        .unwrap_or((before_fragment, ""));
    SplitUrl {
        scheme,
        netloc,
        path: path.to_owned(),
        query: query.to_owned(),
        fragment: fragment.to_owned(),
    }
}

pub(super) fn urlunsplit(parts: &SplitUrl) -> String {
    let mut url = parts.path.clone();
    // Python's `urlunsplit` also forces a `//` prefix when `scheme in
    // uses_netloc` and the path does not already start with `//`; postgres
    // and postgresql never appear in that table, so only the netloc branch
    // below applies for this task's schemes.
    if !parts.netloc.is_empty() {
        if !url.is_empty() && !url.starts_with('/') {
            url.insert(0, '/');
        }
        url = format!("//{}{url}", parts.netloc);
    }
    if !parts.scheme.is_empty() {
        url = format!("{}:{url}", parts.scheme);
    }
    if !parts.query.is_empty() && write!(url, "?{}", parts.query).is_err() {
        return url;
    }
    if !parts.fragment.is_empty() && write!(url, "#{}", parts.fragment).is_err() {
        return url;
    }
    url
}

const fn is_scheme_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')
}

/// `urllib.parse.parse_qsl(query, keep_blank_values=keep_blank_values)`.
pub(super) fn parse_qsl(query: &str, keep_blank_values: bool) -> Vec<(String, String)> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut pairs = Vec::new();
    for chunk in query.split('&') {
        if chunk.is_empty() {
            continue;
        }
        let (name, value) = chunk.split_once('=').unwrap_or((chunk, ""));
        if value.is_empty() && !keep_blank_values {
            continue;
        }
        pairs.push((unquote_plus(name), unquote_plus(value)));
    }
    pairs
}

/// `urllib.parse.parse_qs(query)`: `parse_qsl` grouped by key, each key's
/// values kept in the order they were seen, blanks dropped.
pub(super) fn parse_qs(query: &str) -> Vec<(String, Vec<String>)> {
    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    for (name, value) in parse_qsl(query, false) {
        match grouped.iter_mut().find(|(key, _)| *key == name) {
            Some((_, values)) => values.push(value),
            None => grouped.push((name, vec![value])),
        }
    }
    grouped
}

/// `urllib.parse.urlencode(pairs)`: plain `(name, value)` pairs, no `doseq`.
pub(super) fn urlencode_pairs(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", quote_plus(name), quote_plus(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// `urllib.parse.urlencode(groups, doseq=True)`: a key with several values
/// repeats the key once per value, in that value's order.
pub(super) fn urlencode_doseq(groups: &[(String, Vec<String>)]) -> String {
    let mut parts = Vec::new();
    for (name, values) in groups {
        let encoded_name = quote_plus(name);
        for value in values {
            parts.push(format!("{encoded_name}={}", quote_plus(value)));
        }
    }
    parts.join("&")
}

fn quote_plus(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            _ => {
                if write!(out, "%{byte:02X}").is_err() {
                    return out;
                }
            }
        }
    }
    out
}

fn unquote_plus(text: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'+' {
            bytes.push(b' ');
            rest = tail;
            continue;
        }
        if byte == b'%'
            && let Some(decoded) = tail
                .split_at_checked(2)
                .and_then(|(percent_digits, after)| {
                    from_utf8(percent_digits)
                        .ok()
                        .and_then(|digits| u8::from_str_radix(digits, 16).ok())
                        .map(|value| (value, after))
                })
        {
            bytes.push(decoded.0);
            rest = decoded.1;
            continue;
        }
        bytes.push(byte);
        rest = tail;
    }
    decode_percent_bytes(bytes)
}

/// Replicates Python's `bytes.decode("utf-8", errors="replace")` without the
/// workspace's banned `String::from_utf8_lossy`: each maximal invalid
/// subsequence becomes one U+FFFD, matching how `unquote` recovers from a
/// percent-escape that does not land on a UTF-8 boundary.
fn decode_percent_bytes(bytes: Vec<u8>) -> String {
    let Err(first_error) = from_utf8(&bytes) else {
        return String::from_utf8(bytes).unwrap_or_default();
    };
    let mut text = String::with_capacity(bytes.len());
    let mut rest = bytes.as_slice();
    let mut next_error = Some(first_error.valid_up_to());
    while let Some(valid_len) = next_error {
        let Some((valid, after_valid)) = rest.split_at_checked(valid_len) else {
            break;
        };
        text.push_str(from_utf8(valid).unwrap_or_default());
        text.push(char::REPLACEMENT_CHARACTER);
        let skip = match from_utf8(after_valid) {
            Ok(_) => after_valid.len(),
            Err(error) => error.error_len().unwrap_or(after_valid.len()).max(1),
        };
        rest = after_valid.get(skip..).unwrap_or(&[]);
        next_error = from_utf8(rest).err().map(|error| error.valid_up_to());
    }
    text.push_str(from_utf8(rest).unwrap_or_default());
    text
}
