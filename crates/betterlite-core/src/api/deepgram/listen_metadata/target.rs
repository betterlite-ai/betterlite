use url::Url;

use super::error::ListenMetadataError;
use super::query::{DEFAULT_MODEL, ListenQuery, decode_listen_query};

pub const DEFAULT_API_BASE: &str = "https://api.deepgram.com/v1";

// the listen URL a passthrough dials, parsed back with parse_qs' blank-value rule
#[derive(Clone, Debug)]
pub struct ListenTarget {
    pub url: Url,
    pub query: ListenQuery,
}

pub fn parse_listen_target(upstream_url: &str) -> Result<ListenTarget, ListenMetadataError> {
    let url = Url::parse(upstream_url).map_err(|source| ListenMetadataError::Target { source })?;
    let query = decode_listen_query(url.query().unwrap_or_default().as_bytes());
    Ok(ListenTarget {
        url,
        query: query.without_blank_values(),
    })
}

pub fn websocket_target(
    api_base: Option<&str>,
    query: &ListenQuery,
) -> Result<String, ListenMetadataError> {
    let base = api_base
        .filter(|base| !base.is_empty())
        .unwrap_or(DEFAULT_API_BASE);
    let listen = format!("{}/listen", base.trim_end_matches('/'));
    let mut url = Url::parse(&listen).map_err(|source| ListenMetadataError::Base { source })?;
    let scheme = websocket_scheme(url.scheme()).to_owned();
    if scheme != url.scheme() {
        url.set_scheme(&scheme)
            .map_err(|()| ListenMetadataError::Scheme {
                scheme: scheme.clone(),
            })?;
    }
    let mut params = query.first_occurrences();
    if params.first("model").is_none_or(str::is_empty) {
        params.pairs.retain(|(pair, _)| pair != "model");
        params
            .pairs
            .push(("model".to_owned(), DEFAULT_MODEL.to_owned()));
    }
    let encoded = params.encode();
    Ok(format!("{url}?{encoded}"))
}

fn websocket_scheme(scheme: &str) -> &str {
    match scheme {
        "https" => "wss",
        "http" => "ws",
        other => other,
    }
}
