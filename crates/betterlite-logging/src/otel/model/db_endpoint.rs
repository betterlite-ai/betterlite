use std::net::Ipv6Addr;

use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::KeyValue;
use percent_encoding::percent_decode_str;

use crate::otel::vocabulary::constants::attribute_keys::{db, server};
use crate::otel::vocabulary::resolvers::{POSTGRESQL, db_system};

const DATABASE_URL_ENV: &str = "DATABASE_URL";
const READ_REPLICA_ENV: &str = "DATABASE_URL_READ_REPLICA";
const DEFAULT_PORT: i64 = 5432;
const DEFAULT_SCHEMA: &str = "public";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseEndpoint {
    pub address: Option<String>,
    pub port: Option<i64>,
    pub namespace: Option<String>,
}

struct UrlParts {
    netloc: String,
    path: String,
    query: String,
}

// Mirrors Python's urlsplit for the two PostgreSQL schemes; None where urlsplit would raise.
fn split_url(url: &str) -> Option<UrlParts> {
    let cleaned: String = url
        .trim_start_matches(|char: char| char <= ' ')
        .chars()
        .filter(|char| !matches!(char, '\t' | '\r' | '\n'))
        .collect();
    let (scheme, rest) = cleaned.split_once(':')?;
    if !(scheme.eq_ignore_ascii_case("postgres") || scheme.eq_ignore_ascii_case("postgresql")) {
        return None;
    }
    let (netloc, remainder) = match rest.strip_prefix("//") {
        Some(authority_onward) => {
            let end = authority_onward
                .find(['/', '?', '#'])
                .unwrap_or(authority_onward.len());
            authority_onward.split_at_checked(end)?
        }
        None => ("", rest),
    };
    if netloc.contains('[') != netloc.contains(']') {
        return None;
    }
    if let Some((_, after_open)) = netloc.split_once('[') {
        let host = after_open.split_once(']')?.0;
        host.split('%').next()?.parse::<Ipv6Addr>().ok()?;
    }
    let without_fragment = remainder
        .split_once('#')
        .map_or(remainder, |(before, _)| before);
    let (path, query) = without_fragment
        .split_once('?')
        .unwrap_or((without_fragment, ""));
    Some(UrlParts {
        netloc: netloc.to_owned(),
        path: path.to_owned(),
        query: query.to_owned(),
    })
}

// Python's hostname property plus the raw port text, which only fails when it is read.
fn host_and_port_text(netloc: &str) -> (Option<String>, &str) {
    let host_info = netloc.rsplit_once('@').map_or(netloc, |(_, after)| after);
    let (raw_host, port) = match host_info.split_once('[') {
        Some((_, bracketed)) => {
            let (inside, after) = bracketed.split_once(']').unwrap_or((bracketed, ""));
            (
                inside,
                after.split_once(':').map_or("", |(_, port_text)| port_text),
            )
        }
        None => host_info.split_once(':').unwrap_or((host_info, "")),
    };
    let (name, zone_text) = raw_host.split_once('%').unwrap_or((raw_host, ""));
    let zone = if raw_host.contains('%') {
        format!("%{zone_text}")
    } else {
        String::new()
    };
    let host = (!raw_host.is_empty()).then(|| format!("{}{zone}", name.to_lowercase()));
    (host, port)
}

// Python's port property raises for a non-numeric or out-of-range port.
fn parse_port(text: &str) -> Option<i64> {
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<i64>()
        .ok()
        .filter(|number| *number <= i64::from(u16::MAX))
}

// parse_qs: blank values are dropped, '+' is a space, and the first occurrence of a key wins.
fn first_query_value(query: &str, key: &str) -> String {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(name, value)| {
            !value.is_empty()
                && percent_decode_str(&name.replace('+', " ")).decode_utf8_lossy() == key
        })
        .map(|(_, value)| {
            percent_decode_str(&value.replace('+', " "))
                .decode_utf8_lossy()
                .into_owned()
        })
        .next()
        .unwrap_or_default()
}

// Prisma quotes its schema name, so only the literal lowercase default stays implicit.
fn namespace(database: &str, schema: &str) -> Option<String> {
    let qualifier = if schema == DEFAULT_SCHEMA { "" } else { schema };
    let joined = [database, qualifier]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("|");
    (!joined.is_empty()).then_some(joined)
}

// A malformed port in the netloc invalidates the whole URL, a query port only counts when numeric.
fn endpoint_port(netloc_port_text: &str, query: &str) -> Option<i64> {
    let from_netloc = match netloc_port_text {
        "" => None,
        text => Some(parse_port(text)?),
    }
    .filter(|port| *port != 0);
    let query_port = first_query_value(query, "port");
    let from_query = if query_port.bytes().all(|byte| byte.is_ascii_digit()) {
        query_port.parse::<i64>().ok()
    } else {
        None
    };
    Some(from_query.or(from_netloc).unwrap_or(DEFAULT_PORT))
}

#[must_use]
pub fn parse_database_endpoint(url: Option<&str>) -> Option<DatabaseEndpoint> {
    let dsn = url.filter(|text| !text.is_empty())?;
    let parts = split_url(dsn)?;
    let database = parts.path.trim_start_matches('/');
    // A '/', '#' or '?' inside a password cuts the authority short and strands the '@'; refuse rather than leak.
    if database.contains('/') || (dsn.contains('@') && !parts.netloc.contains('@')) {
        return None;
    }
    let (netloc_host, netloc_port_text) = host_and_port_text(&parts.netloc);
    // host= names a Unix socket directory or sits behind the Cloud SQL connector's localhost netloc.
    let query_host = first_query_value(&parts.query, "host");
    let address = if query_host.is_empty() {
        netloc_host
    } else {
        Some(query_host)
    };
    let port = match &address {
        Some(_) => Some(endpoint_port(netloc_port_text, &parts.query)?),
        None => None,
    };
    let namespace = namespace(
        &percent_decode_str(database).decode_utf8_lossy(),
        &first_query_value(&parts.query, "schema"),
    );
    if address.is_none() && namespace.is_none() {
        return None;
    }
    Some(DatabaseEndpoint {
        address,
        port,
        namespace,
    })
}

// The environment is read on every call so RDS IAM rotation and URL overlays are followed.
#[must_use]
pub fn postgres_endpoint(environment: &dyn EnvironmentLookup) -> Option<DatabaseEndpoint> {
    if environment
        .lookup(READ_REPLICA_ENV)
        .is_some_and(|replica| !replica.is_empty())
    {
        return None;
    }
    parse_database_endpoint(environment.lookup(DATABASE_URL_ENV).as_deref())
}

#[must_use]
pub fn db_span_attributes(
    service_name: &str,
    call_type: Option<&str>,
    environment: &dyn EnvironmentLookup,
) -> Vec<KeyValue> {
    let Some(system) = db_system(service_name) else {
        return Vec::new();
    };
    let endpoint = (system == POSTGRESQL)
        .then(|| postgres_endpoint(environment))
        .flatten();
    let text = |key: &'static str, value: Option<&str>| {
        value.map(|text| KeyValue::new(key, text.to_owned()))
    };
    let port = endpoint
        .as_ref()
        .and_then(|found| found.port)
        .map(|port| KeyValue::new(server::PORT, port));
    [
        text(db::SYSTEM_NAME, Some(system)),
        text(db::SYSTEM_LEGACY, Some(system)),
        text(db::OPERATION_NAME, call_type),
        text(
            server::ADDRESS,
            endpoint.as_ref().and_then(|found| found.address.as_deref()),
        ),
        port,
        text(
            db::NAMESPACE,
            endpoint
                .as_ref()
                .and_then(|found| found.namespace.as_deref()),
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[path = "db_endpoint/tests.rs"]
mod tests;
