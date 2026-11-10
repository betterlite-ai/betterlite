use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

// CPython's quote keeps letters, digits and _.-~ literal and escapes everything else, spaces as %20.
const QUOTE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

pub(super) struct SplitUrl {
    pub(super) scheme: String,
    pub(super) netloc: String,
    pub(super) path: String,
    pub(super) query: String,
    pub(super) fragment: String,
}

// Mirrors urllib.parse.urlsplit: control characters are stripped, then scheme, netloc, path, query and fragment.
pub(super) fn split_url(url: &str) -> SplitUrl {
    let cleaned: String = url
        .trim_start_matches(|character: char| u32::from(character) <= 0x20)
        .chars()
        .filter(|character| !matches!(character, '\t' | '\r' | '\n'))
        .collect();
    let (before_fragment, fragment) = cleaned.split_once('#').unwrap_or((&cleaned, ""));
    let (before_query, query) = before_fragment
        .split_once('?')
        .unwrap_or((before_fragment, ""));
    let (scheme, rest) = split_scheme(before_query);
    let (netloc, path) = split_netloc(rest);
    SplitUrl {
        scheme,
        netloc,
        path,
        query: query.to_owned(),
        fragment: fragment.to_owned(),
    }
}

fn split_scheme(url: &str) -> (String, &str) {
    let Some((candidate, rest)) = url.split_once(':') else {
        return (String::new(), url);
    };
    let mut characters = candidate.chars();
    let starts_alpha = characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic());
    let valid = starts_alpha
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        });
    if valid {
        (candidate.to_ascii_lowercase(), rest)
    } else {
        (String::new(), url)
    }
}

fn split_netloc(rest: &str) -> (String, String) {
    let Some(after) = rest.strip_prefix("//") else {
        return (String::new(), rest.to_owned());
    };
    match after.split_once('/') {
        Some((netloc, tail)) => (netloc.to_owned(), format!("/{tail}")),
        None => (after.to_owned(), String::new()),
    }
}

// Mirrors urllib.parse.urlunsplit; postgres and postgresql are not in CPython's uses_netloc list.
pub(super) fn unsplit(parts: &SplitUrl, query: &str) -> String {
    let mut url = parts.path.clone();
    if !parts.netloc.is_empty() {
        if !url.is_empty() && !url.starts_with('/') {
            url.insert(0, '/');
        }
        url = format!("//{}{url}", parts.netloc);
    }
    if parts.netloc.is_empty() && url.starts_with("//") {
        url = format!("//{url}");
    }
    if !parts.scheme.is_empty() {
        url = format!("{}:{url}", parts.scheme);
    }
    if !query.is_empty() {
        url = format!("{url}?{query}");
    }
    if !parts.fragment.is_empty() {
        url = format!("{url}#{}", parts.fragment);
    }
    url
}

// Mirrors urllib.parse.parse_qsl: '&'-separated fields, empty chunks skipped, blanks kept only on request.
pub(super) fn query_pairs(query: &str, keep_blank_values: bool) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|field| !field.is_empty())
        .filter_map(|field| {
            let (name, value) = field.split_once('=').unwrap_or((field, ""));
            (keep_blank_values || !value.is_empty()).then(|| (decode(name), decode(value)))
        })
        .collect()
}

// Mirrors urllib.parse.unquote_plus: '+' is a space, a bad escape stays literal and invalid UTF-8 becomes U+FFFD.
fn decode(text: &str) -> String {
    let spaced = text.replace('+', " ");
    let decoded = percent_decode_str(&spaced);
    decoded.decode_utf8_lossy().into_owned()
}

// Mirrors urllib.parse.urlencode: quote_plus when the caller used the default, quote otherwise.
pub(super) fn encode_pairs(pairs: &[(String, String)], space_as_plus: bool) -> String {
    let encoded: Vec<String> = pairs
        .iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                encode(name, space_as_plus),
                encode(value, space_as_plus)
            )
        })
        .collect();
    encoded.join("&")
}

fn encode(text: &str, space_as_plus: bool) -> String {
    let quoted = utf8_percent_encode(text, QUOTE_SET).to_string();
    if space_as_plus {
        quoted.replace("%20", "+")
    } else {
        quoted
    }
}
