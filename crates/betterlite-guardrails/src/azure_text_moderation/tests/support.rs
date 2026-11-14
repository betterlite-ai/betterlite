use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::azure_content_safety::legacy_wire::{ModelResponse, TextCategory};
use crate::azure_text_moderation::config::OptionalParams;
use crate::azure_text_moderation::error::AzureTextModerationError;
use crate::azure_text_moderation::wire::{AnalyzeResponse, CategoryAnalysis};
use crate::azure_text_moderation::{AzureTextModerationGuardrail, AzureTextModerationParams};
use crate::guardrail_base::inputs::GuardrailApiInputs;

#[derive(Clone)]
pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) query: Option<String>,
    pub(crate) body: String,
}

pub(crate) struct Mock {
    pub(crate) base: String,
    pub(crate) seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinSet<io::Result<()>>,
}

pub(crate) async fn mock(status: u16, payload: String) -> Mock {
    mock_scripted(vec![(status, payload)]).await
}

fn record(entries: &Mutex<Vec<Seen>>, uri: &Uri, bytes: &Bytes) -> usize {
    let mut log = entries.lock().unwrap();
    log.push(Seen {
        path: uri.path().to_owned(),
        query: uri.query().map(str::to_owned),
        body: String::from_utf8(bytes.to_vec()).unwrap(),
    });
    log.len().saturating_sub(1)
}

fn answer_with(script: &[(u16, String)], index: usize) -> (StatusCode, HeaderMap, String) {
    let (status, payload) = script.get(index).or_else(|| script.last()).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    (
        StatusCode::from_u16(*status).unwrap(),
        headers,
        payload.clone(),
    )
}

// The mock answers the n-th request with the n-th reply and every later one with the last.
pub(crate) async fn mock_scripted(replies: Vec<(u16, String)>) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    let script = Arc::new(replies);
    let answer = move |uri: Uri, _headers: HeaderMap, bytes: Bytes| {
        let request_log = Arc::clone(&recorder);
        let answers = Arc::clone(&script);
        async move {
            let index = record(&request_log, &uri, &bytes);
            answer_with(&answers, index)
        }
    };
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, Router::new().fallback(answer)).into_future());
    Mock {
        base,
        seen,
        _server: server,
    }
}

pub(crate) fn guardrail(
    base: &str,
    optional_params: OptionalParams,
) -> AzureTextModerationGuardrail {
    guardrail_from(AzureTextModerationParams {
        api_key: SecretString::from("azure-key".to_owned()),
        api_base: base.to_owned(),
        api_version: None,
        timeout: None,
        optional_params,
    })
}

pub(crate) fn guardrail_from(params: AzureTextModerationParams) -> AzureTextModerationGuardrail {
    AzureTextModerationGuardrail::new(params, base_builder().build().unwrap())
}

pub(crate) fn params(
    severity_threshold: Option<i64>,
    severity_threshold_by_category: Option<BTreeMap<TextCategory, i64>>,
) -> OptionalParams {
    OptionalParams {
        severity_threshold,
        severity_threshold_by_category,
        categories: None,
        blocklist_names: None,
        halt_on_blocklist_hit: None,
        output_type: None,
    }
}

pub(crate) fn threshold_map(entries: &[(&str, i64)]) -> BTreeMap<TextCategory, i64> {
    entries
        .iter()
        .map(|(name, value)| (TextCategory::from_name(name).unwrap(), *value))
        .collect()
}

pub(crate) fn data(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn inputs(texts: Option<Vec<String>>, model: Option<&str>) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts,
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: model.map(str::to_owned),
        stream_holdback_chars: None,
    }
}

pub(crate) fn violation(error: AzureTextModerationError) -> (String, i64, i64) {
    match error {
        AzureTextModerationError::Violation {
            category,
            threshold,
            severity,
        } => (category, threshold, severity),
        other => panic!("expected a violation, got {other:?}"),
    }
}

pub(crate) fn analysis(entries: &[(&str, i64)]) -> AnalyzeResponse {
    AnalyzeResponse {
        categories_analysis: entries
            .iter()
            .map(|(category, severity)| CategoryAnalysis {
                category: (*category).to_owned(),
                severity: *severity,
            })
            .collect(),
    }
}

pub(crate) fn payload(entries: &[(&str, i64)]) -> String {
    let analyses: Vec<String> = entries
        .iter()
        .map(|(category, severity)| format!(r#"{{"category":"{category}","severity":{severity}}}"#))
        .collect();
    format!(r#"{{"categoriesAnalysis":[{}]}}"#, analyses.join(","))
}

#[derive(Deserialize)]
struct SentText {
    text: String,
}

pub(crate) fn sent_texts(seen: &[Seen]) -> Vec<String> {
    seen.iter()
        .map(|request| {
            serde_json::from_str::<SentText>(&request.body)
                .unwrap()
                .text
        })
        .collect()
}

pub(crate) fn model_response(json: &str) -> ModelResponse {
    serde_json::from_str(json).unwrap()
}
