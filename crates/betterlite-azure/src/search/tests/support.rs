use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_config::sections::azure_search::Settings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::SearchQuery;
use betterlite_openai::search::records::{SearchResponse, SearchResult};
use secrecy::SecretString;

use crate::search::minter::{EntraTokenMinter, MintError};
use crate::search::reply::AzureSearchReply;
use crate::search::{AzureSearchConfig, AzureSearchError};

pub(super) const MODEL: &str = "gpt-4.1";

pub(super) fn json_value(source: &str) -> JsonValue {
    source.parse().unwrap()
}

pub(super) fn params(source: &str) -> Object {
    object(json_value(source))
}

pub(super) fn object(value: JsonValue) -> Object {
    match value {
        JsonValue::Object(map) => map,
        other => panic!("expected a JSON object, got {other}"),
    }
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn header_pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    headers(pairs).into_iter().collect()
}

pub(super) fn model_settings() -> Settings {
    Settings {
        project_endpoint: None,
        model: Some(String::from(MODEL)),
        connection_id: None,
        token: None,
    }
}

pub(super) fn connection_settings() -> Settings {
    Settings {
        connection_id: Some(String::from("connection-1")),
        ..model_settings()
    }
}

pub(super) fn token_settings(token: &str) -> Settings {
    Settings {
        project_endpoint: None,
        model: Some(String::from(MODEL)),
        connection_id: None,
        token: Some(SecretString::from(token)),
    }
}

pub(super) fn endpoint_settings(endpoint: &str) -> Settings {
    Settings {
        project_endpoint: Some(String::from(endpoint)),
        ..model_settings()
    }
}

pub(super) fn endpoint_token_settings(endpoint: &str, token: &str) -> Settings {
    Settings {
        project_endpoint: Some(String::from(endpoint)),
        ..token_settings(token)
    }
}

pub(super) struct FixtureMinter {
    result: Result<&'static str, &'static str>,
    pub(super) calls: AtomicUsize,
}

pub(super) const fn minter(result: Result<&'static str, &'static str>) -> FixtureMinter {
    let calls = AtomicUsize::new(0);
    FixtureMinter { result, calls }
}

pub(super) struct PanicMinter;

impl EntraTokenMinter for PanicMinter {
    fn mint(&self) -> Result<SecretString, MintError> {
        panic!("minted a token no trusted destination asked for");
    }
}

impl EntraTokenMinter for FixtureMinter {
    fn mint(&self) -> Result<SecretString, MintError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.result {
            Ok(token) => Ok(SecretString::from(token)),
            Err(detail) => Err(MintError::Detail {
                detail: detail.to_owned(),
            }),
        }
    }
}

pub(super) fn reply<'reply>(
    body: &'reply str,
    status: u16,
    headers: &'reply [(String, String)],
) -> AzureSearchReply<'reply> {
    AzureSearchReply {
        status_code: status,
        headers,
        body: body.as_bytes(),
    }
}

pub(super) fn transform<'reply>(
    config: &AzureSearchConfig<'_>,
    body: &'reply str,
    status: u16,
    headers: &'reply [(String, String)],
) -> Result<SearchResponse, AzureSearchError> {
    let reply = reply(body, status, headers);
    config.transform_search_response(&reply, None)
}

pub(super) fn results(body: &str) -> Vec<SearchResult> {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let response = transform(&config, body, 200, &[]).unwrap();
    response.results
}

pub(super) fn expect_result(title: &str, url: &str, snippet: &str) -> SearchResult {
    SearchResult {
        title: String::from(title),
        url: String::from(url),
        snippet: String::from(snippet),
        date: None,
        last_updated: None,
        extra: Object::new(),
    }
}

pub(super) fn request_body(config: &AzureSearchConfig<'_>, params: &Object) -> JsonValue {
    let request = config
        .transform_search_request(
            &SearchQuery::Single(String::from("azure grounding query")),
            params,
        )
        .unwrap();
    serde_json::to_value(&request).unwrap()
}

pub(super) fn envelope(output: &str) -> String {
    format!(r#"{{"output":{output}}}"#)
}

pub(super) fn indexed(text: &str, index: &JsonValue) -> String {
    let quoted = serde_json::to_string(text).unwrap();
    envelope(&format!(
        r#"[{{"type":"message","content":[{{"type":"output_text","text":{quoted},"annotations":[{{"type":"url_citation","url":"https://x.example","start_index":{index}}}]}}]}}]"#
    ))
}

pub(super) fn citation_body(urls: &[&str]) -> String {
    let annotations: Vec<String> = urls
        .iter()
        .map(|url| format!(r#"{{"type":"url_citation","url":"{url}"}}"#))
        .collect();
    envelope(&format!(
        r#"[{{"type":"message","content":[{{"type":"output_text","text":"claims","annotations":[{}]}}]}}]"#,
        annotations.join(",")
    ))
}

pub(super) fn response_with_params(
    config: &AzureSearchConfig<'_>,
    body: &str,
    params: Option<&Object>,
) -> Result<SearchResponse, AzureSearchError> {
    let reply = reply(body, 200, &[]);
    config.transform_search_response(&reply, params)
}
