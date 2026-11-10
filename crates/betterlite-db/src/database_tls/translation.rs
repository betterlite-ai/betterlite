use crate::database_url_policy::url_codec::{
    SplitUrl, UrlSplitError, parse_qsl, urlencode_pairs, urlencode_pairs_quote, urlsplit,
    urlunsplit,
};

pub const DEFAULT_POSTGRES_PORT: u16 = 5432;
pub const LIBPQ_VERIFY_SSLMODES: [&str; 2] = ["verify-ca", "verify-full"];
pub const LIBPQ_URL_PARAMS: [&str; 13] = [
    "sslmode",
    "sslcert",
    "sslkey",
    "sslrootcert",
    "sslpassword",
    "application_name",
    "connect_timeout",
    "client_encoding",
    "options",
    "service",
    "gssencmode",
    "krbsrvname",
    "target_session_attrs",
];

#[derive(Debug, thiserror::Error)]
pub enum TranslateError {
    #[error("invalid URL: {source}")]
    Url {
        #[source]
        source: UrlSplitError,
    },
    #[error("port {value:?} could not be cast to an integer value")]
    MalformedPort { value: String },
    #[error("port out of range 0-65535: {value:?}")]
    PortOutOfRange { value: String },
}

// Mirrors db_url_settings.py::translate_libpq_ssl_params with the resolver supplied by the caller.
pub fn translate_libpq_ssl_params<F>(
    url: &str,
    mut resolve_root_cert: F,
) -> Result<String, TranslateError>
where
    F: FnMut(&str, &str, u16) -> String,
{
    let mut parts = urlsplit(url).map_err(|source| TranslateError::Url { source })?;
    let pairs = parse_qsl(&parts.query, true);
    let wants_verify = pairs
        .iter()
        .any(|(key, value)| key == "sslmode" && LIBPQ_VERIFY_SSLMODES.contains(&value.as_str()));
    let has_root = pairs.iter().any(|(key, _)| key == "sslrootcert");
    if !wants_verify && !has_root {
        return Ok(url.to_owned());
    }
    let has_cert = pairs.iter().any(|(key, _)| key == "sslcert");
    let mut translated: Vec<(String, String)> = Vec::new();
    for (key, value) in &pairs {
        if key == "sslrootcert" {
            continue;
        }
        if key == "sslmode" && LIBPQ_VERIFY_SSLMODES.contains(&value.as_str()) {
            translated.push(("sslmode".to_owned(), "require".to_owned()));
        } else {
            translated.push((key.clone(), value.clone()));
        }
    }
    if !has_cert {
        let roots: Vec<&String> = pairs
            .iter()
            .filter(|(key, _)| key == "sslrootcert")
            .map(|(_, value)| value)
            .collect();
        if !roots.is_empty() {
            let (host, port) = host_and_port(&parts)?;
            for root in roots {
                let resolved = resolve_root_cert(root, &host, port);
                translated.push(("sslcert".to_owned(), resolved));
            }
        }
    }
    if !pairs.iter().any(|(key, _)| key == "sslaccept") {
        translated.push(("sslaccept".to_owned(), "strict".to_owned()));
    }
    parts.query = urlencode_pairs(&translated);
    Ok(urlunsplit(&parts))
}

fn host_and_port(parts: &SplitUrl) -> Result<(String, u16), TranslateError> {
    let hostinfo = parts
        .netloc
        .rsplit_once('@')
        .map_or(parts.netloc.as_str(), |(_, host)| host);
    let (hostname, port_text) = hostinfo.strip_prefix('[').map_or_else(
        || hostinfo.split_once(':').unwrap_or((hostinfo, "")),
        |rest| {
            let (inside, after) = rest.split_once(']').unwrap_or((rest, ""));
            (inside, after.strip_prefix(':').unwrap_or(""))
        },
    );
    let resolved = port_value(port_text)?
        .filter(|value| *value != 0)
        .unwrap_or(DEFAULT_POSTGRES_PORT);
    Ok((hostname.to_lowercase(), resolved))
}

// CPython 3.13's urlsplit only casts a port made of ASCII digits; a sign, space or underscore is a ValueError.
fn port_value(text: &str) -> Result<Option<u16>, TranslateError> {
    if text.is_empty() {
        return Ok(None);
    }
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(TranslateError::MalformedPort {
            value: text.to_owned(),
        });
    }
    let out_of_range = || TranslateError::PortOutOfRange {
        value: text.to_owned(),
    };
    let value: u64 = text.parse().map_err(|_source| out_of_range())?;
    u16::try_from(value)
        .map(Some)
        .map_err(|_source| out_of_range())
}

// Mirrors utils.py::_strip_prisma_query_params; an unqueried URL keeps every byte the operator wrote.
pub fn strip_prisma_query_params(url: &str) -> Result<String, TranslateError> {
    let mut parts = urlsplit(url).map_err(|source| TranslateError::Url { source })?;
    if parts.query.is_empty() {
        return Ok(url.to_owned());
    }
    let pairs = parse_qsl(&parts.query, false);
    let kept: Vec<(String, String)> = pairs
        .iter()
        .filter(|(key, _)| LIBPQ_URL_PARAMS.contains(&key.as_str()))
        .cloned()
        .collect();
    let sslaccept = pairs
        .iter()
        .find(|(key, _)| key == "sslaccept")
        .map(|(_, value)| value.as_str());
    let libpq = libpq_tls_params(&kept, sslaccept);
    parts.query = urlencode_pairs_quote(&libpq);
    Ok(urlunsplit(&parts))
}

// Mirrors utils.py::_libpq_tls_params: Prisma's sslcert is the CA unless an sslkey makes it a client certificate.
#[must_use]
pub fn libpq_tls_params(
    pairs: &[(String, String)],
    sslaccept: Option<&str>,
) -> Vec<(String, String)> {
    let has_cert = pairs.iter().any(|(key, _)| key == "sslcert");
    let has_key = pairs.iter().any(|(key, _)| key == "sslkey");
    if !has_cert || has_key {
        return pairs.to_vec();
    }
    let sslmode = pairs
        .iter()
        .find(|(key, _)| key == "sslmode")
        .map(|(_, value)| value.clone());
    let mut rest: Vec<(String, String)> = pairs
        .iter()
        .filter(|(key, _)| key != "sslcert" && key != "sslmode")
        .cloned()
        .collect();
    let unverified = sslaccept.is_none() || sslaccept == Some("accept_invalid_certs");
    if unverified || sslmode.as_deref() == Some("disable") {
        if let Some(mode) = sslmode {
            rest.push(("sslmode".to_owned(), mode));
        }
        return rest;
    }
    let has_root = pairs.iter().any(|(key, _)| key == "sslrootcert");
    if !has_root {
        for (key, value) in pairs {
            if key == "sslcert" {
                rest.push(("sslrootcert".to_owned(), value.clone()));
            }
        }
    }
    rest.push(("sslmode".to_owned(), "verify-full".to_owned()));
    rest
}
