use std::net::Ipv6Addr;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

const SOURCE_URI: &str = "x-amz-bedrock-kb-source-uri";
const CHUNK_ID: &str = "x-amz-bedrock-kb-chunk-id";
const DATA_SOURCE_ID: &str = "x-amz-bedrock-kb-data-source-id";
const PARAM_SCHEMES: [&str; 16] = [
    "", "ftp", "hdl", "prospero", "http", "imap", "https", "shttp", "rtsp", "rtsps", "rtspu",
    "sip", "sips", "mms", "sftp", "tel",
];

#[must_use]
pub fn file_id_from_metadata(metadata: &IndexMap<String, Json>) -> Json {
    if let Some(source) = metadata.get(SOURCE_URI).filter(|value| non_empty(value)) {
        return source.clone();
    }
    let chunk = metadata
        .get(CHUNK_ID)
        .map_or_else(|| "unknown".to_owned(), scalar_text);
    Json::Text(format!("bedrock-kb-{chunk}"))
}

#[must_use]
pub fn filename_from_metadata(metadata: &IndexMap<String, Json>) -> Json {
    let Some(source) = metadata.get(SOURCE_URI).filter(|value| non_empty(value)) else {
        let data_source = metadata
            .get(DATA_SOURCE_ID)
            .map_or_else(|| "unknown".to_owned(), scalar_text);
        return Json::Text(format!("bedrock-kb-document-{data_source}"));
    };
    let Json::Text(uri) = source else {
        return source.clone();
    };
    let Some(parts) = split_uri(uri) else {
        return Json::Text(uri.clone());
    };
    let candidate = parts
        .path
        .rsplit_once('/')
        .map_or(parts.path.as_str(), |(_, last)| last);
    let filename = if candidate.is_empty() || candidate == "/" {
        parts.netloc.as_str()
    } else {
        candidate
    };
    Json::Text(filename.to_owned())
}

struct UriParts {
    netloc: String,
    path: String,
}

// urlparse splits syntactically; it raises only on a bad bracketed host
fn split_uri(value: &str) -> Option<UriParts> {
    let stripped: String = value
        .trim_start_matches(|character| character <= ' ')
        .chars()
        .filter(|character| !matches!(character, '\t' | '\r' | '\n'))
        .collect();
    let without_fragment = stripped.split('#').next().unwrap_or_default();
    let without_query = without_fragment.split('?').next().unwrap_or_default();
    let (scheme, rest) = split_scheme(without_query);
    let (netloc, path) = match rest.strip_prefix("//") {
        None => ("", rest),
        Some(authority) => {
            let (host, tail) = authority
                .find('/')
                .map_or((authority, ""), |index| authority.split_at(index));
            let opens = host.contains('[');
            if opens != host.contains(']') || (opens && !bracketed_netloc_is_valid(host)) {
                return None;
            }
            (host, tail)
        }
    };
    let keeps_params = PARAM_SCHEMES
        .iter()
        .any(|known| known.eq_ignore_ascii_case(scheme));
    Some(UriParts {
        netloc: netloc.to_owned(),
        path: if keeps_params {
            without_params(path)
        } else {
            path
        }
        .to_owned(),
    })
}

fn without_params(path: &str) -> &str {
    let start = path.rfind('/').unwrap_or(0);
    path.match_indices(';')
        .find(|(index, _)| *index >= start)
        .map_or(path, |(index, _)| path.split_at(index).0)
}

fn bracketed_netloc_is_valid(netloc: &str) -> bool {
    let host_and_port = netloc.rsplit_once('@').map_or(netloc, |(_, rest)| rest);
    let hostname = match host_and_port.split_once('[') {
        Some((before, _)) if !before.is_empty() => return false,
        Some((_, bracketed)) => {
            let (hostname, port) = bracketed.split_once(']').unwrap_or((bracketed, ""));
            if !port.is_empty() && !port.starts_with(':') {
                return false;
            }
            hostname
        }
        None => host_and_port.split(':').next().unwrap_or_default(),
    };
    if let Some(future) = hostname.strip_prefix('v') {
        let after_digits =
            future.trim_start_matches(|character: char| character.is_ascii_hexdigit());
        return after_digits.len() < future.len()
            && after_digits
                .strip_prefix('.')
                .is_some_and(|tail| !tail.is_empty());
    }
    let (address, scope) = hostname
        .split_once('%')
        .map_or((hostname, None), |(address, scope)| (address, Some(scope)));
    scope.is_none_or(|id| !id.is_empty() && !id.contains('%'))
        && address.parse::<Ipv6Addr>().is_ok()
}

fn split_scheme(value: &str) -> (&str, &str) {
    let Some((candidate, rest)) = value.split_once(':') else {
        return ("", value);
    };
    let mut characters = candidate.chars();
    let valid = characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        });
    if valid {
        (candidate, rest)
    } else {
        ("", value)
    }
}

#[must_use]
pub fn non_empty(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number
            .as_f64()
            .is_some_and(|magnitude| magnitude != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

// Python's str() of a scalar; compatibility keeps standard JSON for containers instead of repr
fn scalar_text(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).unwrap_or_else(|_error| String::new())
        }
    }
}
