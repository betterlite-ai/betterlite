use super::url_codec::{
    SplitUrl, UrlSplitError, parse_qs, parse_qsl, urlencode_doseq, urlencode_pairs, urlsplit,
    urlunsplit,
};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(untagged)]
pub enum QueryValue {
    Integer(i64),
    Float(f64),
    Text(String),
}

impl QueryValue {
    fn to_param_string(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Integer(number) => number.to_string(),
            // Matches Python's str(float): a whole number still carries ".0".
            Self::Float(number) => {
                let fixed = number.to_string();
                if fixed.contains('.') {
                    fixed
                } else {
                    format!("{fixed}.0")
                }
            }
        }
    }
}

const IDLE_LIFETIME_DEFAULT: i64 = 60;

pub(super) const CONNECTION_PARAM_KEYS: [&str; 6] = [
    "connection_limit",
    "pool_timeout",
    "connect_timeout",
    "socket_timeout",
    "max_idle_connection_lifetime",
    "pgbouncer",
];

const PRISMA_TLS_PARAM_KEYS: [&str; 3] = ["sslmode", "sslcert", "sslaccept"];

// A configured value passes through unchanged; only a missing one gets the default.
#[must_use]
pub fn idle_lifetime_params(configured: Option<QueryValue>) -> Vec<(String, QueryValue)> {
    let value = configured.unwrap_or(QueryValue::Integer(IDLE_LIFETIME_DEFAULT));
    vec![("max_idle_connection_lifetime".to_owned(), value)]
}

// Returns url unchanged when nothing is added.
pub fn add_missing_query_params(
    url: &str,
    params: &[(String, QueryValue)],
) -> Result<String, UrlSplitError> {
    let parsed = urlsplit(url)?;
    let existing = parse_qsl(&parsed.query, true);
    let pinned: HashSet<&str> = existing.iter().map(|(key, _)| key.as_str()).collect();
    let additions: Vec<(String, String)> = params
        .iter()
        .filter(|(key, _)| !pinned.contains(key.as_str()))
        .map(|(key, value)| (key.clone(), value.to_param_string()))
        .collect();
    if additions.is_empty() {
        return Ok(url.to_owned());
    }
    let mut all = existing;
    all.extend(additions);
    let query = urlencode_pairs(&all);
    Ok(urlunsplit(&SplitUrl { query, ..parsed }))
}

// The pool/timeout allowlist the reader may inherit.
#[must_use]
pub fn reader_shareable_params(params: &[(String, QueryValue)]) -> Vec<(String, QueryValue)> {
    let kept: Vec<(String, QueryValue)> = params
        .iter()
        .filter(|(key, _)| CONNECTION_PARAM_KEYS.contains(&key.as_str()))
        .cloned()
        .collect();
    kept
}

fn last_duplicate_wins(pairs: Vec<(String, String)>) -> Vec<(String, QueryValue)> {
    let mut deduped: Vec<(String, QueryValue)> = Vec::new();
    for (key, value) in pairs {
        let entry = QueryValue::Text(value);
        match deduped.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, slot)) => *slot = entry,
            None => deduped.push((key, entry)),
        }
    }
    deduped
}

// The reader-shareable subset already on url.
pub fn connection_params_from_url(url: &str) -> Result<Vec<(String, QueryValue)>, UrlSplitError> {
    let parsed = urlsplit(url)?;
    let deduped = last_duplicate_wins(parse_qsl(&parsed.query, false));
    Ok(reader_shareable_params(&deduped))
}

// The pool/timeout and Prisma TLS params a refreshed token URL carries over; schema and options are never included.
pub fn token_refresh_params_from_url(
    url: &str,
) -> Result<Vec<(String, QueryValue)>, UrlSplitError> {
    let parsed = urlsplit(url)?;
    let kept: Vec<(String, String)> = parse_qsl(&parsed.query, false)
        .into_iter()
        .filter(|(key, _)| {
            CONNECTION_PARAM_KEYS.contains(&key.as_str())
                || PRISMA_TLS_PARAM_KEYS.contains(&key.as_str())
        })
        .collect();
    Ok(last_duplicate_wins(kept))
}

// The first nonblank value for key, or "".
pub fn url_query_value(raw_url: Option<&str>, key: &str) -> Result<String, UrlSplitError> {
    let Some(url) = raw_url.filter(|value| !value.is_empty()) else {
        return Ok(String::new());
    };
    let parsed = urlsplit(url)?;
    Ok(parse_qs(&parsed.query)
        .into_iter()
        .find(|(existing, _)| existing == key)
        .and_then(|(_, values)| values.into_iter().next())
        .unwrap_or_default())
}

// Every old occurrence of key is removed, then one replacement is appended.
pub fn with_query_value(url: &str, key: &str, value: &str) -> Result<String, UrlSplitError> {
    let parsed = urlsplit(url)?;
    let mut pairs: Vec<(String, String)> = parse_qsl(&parsed.query, false)
        .into_iter()
        .filter(|(existing, _)| existing != key)
        .collect();
    pairs.push((key.to_owned(), value.to_owned()));
    let query = urlencode_pairs(&pairs);
    Ok(urlunsplit(&SplitUrl { query, ..parsed }))
}

fn update_query_groups(
    base: Vec<(String, Vec<String>)>,
    updates: &[(String, Vec<String>)],
) -> Vec<(String, Vec<String>)> {
    let mut result = base;
    for (key, values) in updates {
        match result.iter_mut().find(|(existing, _)| existing == key) {
            Some((_, slot)) => slot.clone_from(values),
            None => result.push((key.clone(), values.clone())),
        }
    }
    result
}

pub struct AppendOutcome {
    pub url: String,
    pub warning: Option<&'static str>,
}

const EMPTY_URL_WARNING: &str =
    "append_query_params received empty or non-string URL, returning empty string";

// Unlike add_missing_query_params, blank existing values are dropped and a supplied key's values overwrite any existing list.
pub fn append_query_params(
    raw_url: Option<&str>,
    params: &[(String, Vec<String>)],
) -> Result<AppendOutcome, UrlSplitError> {
    let Some(url) = raw_url.filter(|value| !value.is_empty()) else {
        return Ok(AppendOutcome {
            url: String::new(),
            warning: Some(EMPTY_URL_WARNING),
        });
    };
    let parsed = urlsplit(url)?;
    let groups = update_query_groups(parse_qs(&parsed.query), params);
    let query = urlencode_doseq(&groups);
    Ok(AppendOutcome {
        url: urlunsplit(&SplitUrl { query, ..parsed }),
        warning: None,
    })
}

const SECRET_QUERY_KEYS: [&str; 2] = ["password", "sslpassword"];

// The redacted form of what append_query_params would log; never carries the URL's credentials.
#[must_use]
pub fn append_query_params_diagnostic(
    raw_url: Option<&str>,
    params: &[(String, Vec<String>)],
) -> (String, String) {
    let redacted_url = raw_url.map_or_else(|| "None".to_owned(), redact_credentials);
    let mut redacted_params = String::from("{");
    let mut keys = params.iter().map(|(key, _)| key.as_str());
    if let Some(first) = keys.next() {
        redacted_params.push_str(first);
        redacted_params.push_str(": REDACTED");
    }
    for key in keys {
        redacted_params.push_str(", ");
        redacted_params.push_str(key);
        redacted_params.push_str(": REDACTED");
    }
    redacted_params.push('}');
    (redacted_url, redacted_params)
}

// A URL this helper cannot parse is reported fully redacted, never raw.
fn redact_credentials(url: &str) -> String {
    let Ok(parsed) = urlsplit(url) else {
        return "REDACTED".to_owned();
    };
    if parsed.netloc.is_empty() {
        return "REDACTED".to_owned();
    }
    let netloc = match parsed.netloc.rsplit_once('@') {
        Some((_, host)) => format!("REDACTED@{host}"),
        None => parsed.netloc.clone(),
    };
    let query = redact_secret_query_values(&parsed.query);
    urlunsplit(&SplitUrl {
        netloc,
        query,
        ..parsed
    })
}

fn redact_secret_query_values(query: &str) -> String {
    let redacted: Vec<(String, String)> = parse_qsl(query, true)
        .into_iter()
        .map(|(key, value)| {
            if SECRET_QUERY_KEYS.contains(&key.as_str()) {
                (key, "REDACTED".to_owned())
            } else {
                (key, value)
            }
        })
        .collect();
    urlencode_pairs(&redacted)
}
