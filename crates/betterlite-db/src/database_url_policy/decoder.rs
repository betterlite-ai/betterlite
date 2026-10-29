use super::url_codec::{UrlSplitError, parse_qsl, urlsplit};
use secrecy::SecretString;
use std::fmt::{self, Debug, Formatter};
use std::num::ParseIntError;
use std::str::{self, Utf8Error};

const MAX_DATABASE_URL_BYTES: usize = 0x0010_0000;

type Netloc = (Option<String>, Option<SecretString>, String, Option<u16>);

const SECRET_QUERY_KEYS: [&str; 2] = ["password", "sslpassword"];

#[derive(Clone)]
pub enum QueryParamValue {
    Plain(String),
    Secret(SecretString),
}

impl Debug for QueryParamValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plain(value) => Debug::fmt(value, f),
            Self::Secret(_) => f.write_str("\"REDACTED\""),
        }
    }
}

#[derive(Clone)]
pub struct DbEndpoint {
    pub scheme: String,
    pub user: Option<String>,
    pub password: Option<SecretString>,
    pub host: String,
    pub port: Option<u16>,
    pub database: String,
    pub query: Vec<(String, QueryParamValue)>,
}

impl Debug for DbEndpoint {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbEndpoint")
            .field("scheme", &self.scheme)
            .field("user", &self.user.as_ref().map(|_| "REDACTED"))
            .field("password", &self.password.as_ref().map(|_| "REDACTED"))
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("query", &self.query)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database URL is {len} bytes, over the {MAX_DATABASE_URL_BYTES} byte limit")]
    TooLarge { len: usize },
    #[error("database URL is not valid UTF-8")]
    InvalidUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("database URL has no scheme")]
    MissingScheme,
    #[error(
        "database URL has credentials past the parsed host, carried by an unescaped '/', '?' or '#'"
    )]
    CredentialsPastNetloc,
    #[error("database URL host has an unterminated or misplaced '[' bracket")]
    MalformedBracket,
    #[error(transparent)]
    InvalidIpv6Url(#[from] UrlSplitError),
    #[error("database URL port ({len} bytes) is not a valid port number")]
    MalformedPort {
        len: usize,
        #[source]
        source: ParseIntError,
    },
    #[error("database URL has an incomplete or invalid percent-escape")]
    MalformedPercentEscape {
        #[source]
        source: PercentEscapeError,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum PercentEscapeError {
    #[error("percent-escape has fewer than two trailing characters")]
    Incomplete,
    #[error("percent-escape digits are not ASCII hex digits")]
    NotHexDigit,
    #[error("percent-escape digits are not valid UTF-8")]
    NotUtf8(#[source] Utf8Error),
    #[error("percent-escape digits are not a valid hex byte")]
    NotHex(#[source] ParseIntError),
}

pub fn decode_database_url(bytes: &[u8]) -> Result<DbEndpoint, DbError> {
    if bytes.len() > MAX_DATABASE_URL_BYTES {
        return Err(DbError::TooLarge { len: bytes.len() });
    }
    let text = str::from_utf8(bytes).map_err(|source| DbError::InvalidUtf8 { source })?;
    let parsed = urlsplit(text)?;
    if parsed.scheme.is_empty() {
        return Err(DbError::MissingScheme);
    }
    let (user, password, host, port) = parse_netloc(&parsed.netloc)?;
    // Mirrors query.rs's redact_credentials guard: once the netloc itself parses, an unencoded
    // '/', '?' or '#' in the password has still truncated it early, leaving the rest of the
    // credentials sitting unparsed (and unredacted) in the path, query or fragment.
    if !parsed.netloc.is_empty()
        && (parsed.path.contains('@')
            || parsed.query.contains('@')
            || parsed.fragment.contains('@'))
    {
        return Err(DbError::CredentialsPastNetloc);
    }
    let database = percent_decode(parsed.path.strip_prefix('/').unwrap_or(&parsed.path))?;
    let query = parse_qsl(&parsed.query, true)
        .into_iter()
        .map(|(key, value)| {
            if SECRET_QUERY_KEYS.contains(&key.as_str()) {
                (key, QueryParamValue::Secret(SecretString::from(value)))
            } else {
                (key, QueryParamValue::Plain(value))
            }
        })
        .collect();
    Ok(DbEndpoint {
        scheme: parsed.scheme,
        user,
        password,
        host,
        port,
        database,
        query,
    })
}

fn parse_netloc(netloc: &str) -> Result<Netloc, DbError> {
    let (userinfo, hostport) = match netloc.rsplit_once('@') {
        Some((info, rest)) => (Some(info), rest),
        None => (None, netloc),
    };
    let (user, password) = match userinfo {
        None => (None, None),
        Some(info) => match info.split_once(':') {
            Some((user, password)) => (
                Some(percent_decode(user)?),
                Some(SecretString::from(percent_decode(password)?)),
            ),
            None => (Some(percent_decode(info)?), None),
        },
    };
    let (host, port) = parse_hostport(hostport)?;
    Ok((user, password, host, port))
}

fn parse_hostport(hostport: &str) -> Result<(String, Option<u16>), DbError> {
    if let Some(after_bracket) = hostport.strip_prefix('[') {
        let (host, tail) = after_bracket
            .split_once(']')
            .ok_or(DbError::MalformedBracket)?;
        let port = match tail.strip_prefix(':') {
            Some(text) => Some(
                text.parse::<u16>()
                    .map_err(|source| DbError::MalformedPort {
                        len: text.len(),
                        source,
                    })?,
            ),
            None if tail.is_empty() => None,
            None => return Err(DbError::MalformedBracket),
        };
        return Ok((percent_decode(host)?, port));
    }
    match hostport.rsplit_once(':') {
        Some((host, text)) => Ok((
            percent_decode(host)?,
            Some(
                text.parse::<u16>()
                    .map_err(|source| DbError::MalformedPort {
                        len: text.len(),
                        source,
                    })?,
            ),
        )),
        None => Ok((percent_decode(hostport)?, None)),
    }
}

fn percent_decode(text: &str) -> Result<String, DbError> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' {
            let (digits, after) =
                tail.split_at_checked(2)
                    .ok_or(DbError::MalformedPercentEscape {
                        source: PercentEscapeError::Incomplete,
                    })?;
            // u8::from_str_radix also accepts a leading '+' sign, unlike CPython's _hextobyte.
            if !digits.iter().all(u8::is_ascii_hexdigit) {
                return Err(DbError::MalformedPercentEscape {
                    source: PercentEscapeError::NotHexDigit,
                });
            }
            let digits_text =
                str::from_utf8(digits).map_err(|source| DbError::MalformedPercentEscape {
                    source: PercentEscapeError::NotUtf8(source),
                })?;
            let decoded = u8::from_str_radix(digits_text, 16).map_err(|source| {
                DbError::MalformedPercentEscape {
                    source: PercentEscapeError::NotHex(source),
                }
            })?;
            bytes.push(decoded);
            rest = after;
            continue;
        }
        bytes.push(byte);
        rest = tail;
    }
    String::from_utf8(bytes).map_err(|error| DbError::InvalidUtf8 {
        source: error.utf8_error(),
    })
}
