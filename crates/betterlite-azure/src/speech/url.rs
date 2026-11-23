use betterlite_http::user_url::httpx_url::HttpxUrl;
use url::Url;

use super::error::AzureSpeechError;
use crate::transcription::encoding::quote_plus;

// The SDK composes its URL onto the client base and merges the caller's query over the client's api-version.
pub(super) fn compose_url(
    base: &str,
    deployment: Option<&str>,
    path: &str,
    version: Option<&str>,
    extra_query: &[(String, String)],
) -> Result<Url, AzureSpeechError> {
    let mut parsed = HttpxUrl::parse(base).map_err(|source| AzureSpeechError::Url { source })?;
    let base_query = parsed.query.take();
    parsed.fragment = None;
    let prefix = match deployment {
        Some(name) if !parsed.path.contains("/deployments") => format!("deployments/{name}/"),
        Some(_) | None => String::new(),
    };
    let composed_text = format!("{}{prefix}{path}", parsed.href());
    let mut composed =
        HttpxUrl::parse(&composed_text).map_err(|source| AzureSpeechError::RequestUrl {
            url: composed_text.clone(),
            source,
        })?;
    composed.query = merged_query(base_query.as_deref(), version, extra_query);
    Url::parse(&composed.href()).map_err(|source| AzureSpeechError::RequestUrl {
        url: composed.href(),
        source,
    })
}

fn merged_query(
    base_query: Option<&str>,
    version: Option<&str>,
    extra_query: &[(String, String)],
) -> Option<String> {
    let mut pairs: Vec<String> = base_query.map(split_pairs).unwrap_or_default();
    for (key, value) in extra_query {
        let encoded = format!("{}={}", quote_plus(key), quote_plus(value));
        upsert(&mut pairs, &encoded, key);
    }
    if let Some(version) = version {
        upsert(
            &mut pairs,
            &format!("api-version={}", quote_plus(version)),
            "api-version",
        );
    }
    (!pairs.is_empty()).then(|| pairs.join("&"))
}

fn split_pairs(query: &str) -> Vec<String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(str::to_owned)
        .collect()
}

fn upsert(pairs: &mut Vec<String>, encoded: &str, key: &str) {
    let existing = pairs
        .iter_mut()
        .find(|pair| pair.split('=').next() == Some(key));
    match existing {
        Some(pair) => {
            pair.clear();
            pair.push_str(encoded);
        }
        None => pairs.push(encoded.to_owned()),
    }
}
