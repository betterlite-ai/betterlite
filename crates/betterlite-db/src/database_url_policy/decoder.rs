use super::url_codec::{UrlSplitError, parse_qsl, urlsplit};
use secrecy::SecretString;
use std::fmt::{self, Debug, Formatter};
use std::num::ParseIntError;
use std::str::{self, Utf8Error};

// 1 MiB cap on a database URL before decoding is attempted.
const MAX_DATABASE_URL_BYTES: usize = 0x0010_0000;

type Netloc = (Option<String>, Option<SecretString>, String, Option<u16>);

const SECRET_QUERY_KEYS: [&str; 2] = ["password", "sslpassword"];

#[derive(Clone)]
pub struct DbEndpoint {
    pub scheme: String,
    pub user: Option<String>,
    pub password: Option<SecretString>,
    pub host: String,
    pub port: Option<u16>,
    pub database: String,
    pub query: Vec<(String, String)>,
}

impl Debug for DbEndpoint {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let redacted_query: Vec<(&str, &str)> = self
            .query
            .iter()
            .map(|(key, value)| {
                if SECRET_QUERY_KEYS.contains(&key.as_str()) {
                    (key.as_str(), "REDACTED")
                } else {
                    (key.as_str(), value.as_str())
                }
            })
            .collect();
        f.debug_struct("DbEndpoint")
            .field("scheme", &self.scheme)
            .field("user", &self.user.as_ref().map(|_| "REDACTED"))
            .field("password", &self.password.as_ref().map(|_| "REDACTED"))
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("query", &redacted_query)
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
    #[error("database URL host has an unterminated or misplaced '[' bracket")]
    MalformedBracket,
    #[error(transparent)]
    InvalidIpv6Url(#[from] UrlSplitError),
    #[error("database URL port {text:?} is not a valid port number")]
    MalformedPort {
        text: String,
        #[source]
        source: ParseIntError,
    },
    #[error("database URL has an incomplete or invalid percent-escape")]
    MalformedPercentEscape,
}

// The native URL byte boundary; does not reuse the Python-parity helpers above.
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
    let database = parsed
        .path
        .strip_prefix('/')
        .unwrap_or(&parsed.path)
        .to_owned();
    let query = parse_qsl(&parsed.query, true);
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
            Some(text) => Some(parse_port(text)?),
            None if tail.is_empty() => None,
            None => return Err(DbError::MalformedBracket),
        };
        return Ok((percent_decode(host)?, port));
    }
    match hostport.rsplit_once(':') {
        Some((host, text)) => Ok((percent_decode(host)?, Some(parse_port(text)?))),
        None => Ok((percent_decode(hostport)?, None)),
    }
}

fn parse_port(text: &str) -> Result<u16, DbError> {
    let parsed = text.parse::<u16>();
    parsed.map_err(|source| DbError::MalformedPort {
        text: text.to_owned(),
        source,
    })
}

fn percent_decode(text: &str) -> Result<String, DbError> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' {
            let (digits, after) = tail
                .split_at_checked(2)
                .ok_or(DbError::MalformedPercentEscape)?;
            let digits_text =
                str::from_utf8(digits).map_err(|_source| DbError::MalformedPercentEscape)?;
            let decoded = u8::from_str_radix(digits_text, 16)
                .map_err(|_source| DbError::MalformedPercentEscape)?;
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
