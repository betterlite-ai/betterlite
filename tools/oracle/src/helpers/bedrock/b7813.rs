use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_provider::api::vector_store_contracts::config::Params;
use betterlite_rag::valkey_vector_store::params::ValkeyParams;
use betterlite_rag::valkey_vector_store::ports::{ValkeyDocument, ValkeyTimeout, socket_timeouts};
use betterlite_rag::valkey_vector_store::query::knn_query;
use betterlite_rag::valkey_vector_store::reply::search_response;
use betterlite_rag::valkey_vector_store::{build_valkey_url, pack_vector};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "call", content = "input", rename_all = "snake_case")]
enum Call {
    Url(UrlCall),
    Pack(PackCall),
    Knn(KnnCall),
    Results(ResultsCall),
    Timeouts(TimeoutsCall),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlCall {
    host: String,
    port: u16,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    ssl: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackCall {
    vector: Vec<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KnnCall {
    #[serde(default)]
    litellm_params: Params,
    #[serde(default)]
    optional_params: Params,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultsCall {
    #[serde(default)]
    litellm_params: Params,
    query_text: String,
    documents: Vec<DocumentCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentCall {
    id: String,
    #[serde(default)]
    fields: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TimeoutsCall {
    timeouts: Vec<TimeoutCall>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[derive(Clone, Copy)]
enum TimeoutCall {
    None,
    Seconds {
        seconds: f64,
    },
    Object {
        #[serde(default)]
        connect: Option<f64>,
        #[serde(default)]
        read: Option<f64>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(Value::Array(input.calls.iter().map(run_call).collect()))
    }
}

fn run_call(call: &Call) -> Value {
    match call {
        Call::Url(url) => json!({"ok": true, "value": url_value(url)}),
        Call::Pack(pack) => json!({"ok": true, "value": pack_vector(&pack.vector)}),
        Call::Knn(knn) => outcome(knn_value(knn)),
        Call::Results(results) => outcome(results_value(results)),
        Call::Timeouts(timeouts) => outcome(timeouts_value(timeouts)),
    }
}

fn outcome(result: Option<Value>) -> Value {
    result.map_or_else(
        || json!({"ok": false}),
        |value| json!({"ok": true, "value": value}),
    )
}

fn url_value(call: &UrlCall) -> String {
    build_valkey_url(
        &call.host,
        call.port,
        call.password.clone().map(SecretString::from).as_ref(),
        call.ssl,
    )
}

// the arguments redis-py's Query.get_args hands to FT.SEARCH, in the same order
fn knn_value(call: &KnnCall) -> Option<Value> {
    let params = ValkeyParams::read(&call.litellm_params).ok()?;
    let query = knn_query(
        &call.optional_params,
        &params.embedding_field,
        &params.text_field,
    )
    .ok()?;
    let mut arguments = vec![
        Value::String(query.expression),
        Value::String("RETURN".to_owned()),
        Value::from(query.return_fields.len()),
    ];
    arguments.extend(query.return_fields.into_iter().map(Value::String));
    arguments.push(Value::String("DIALECT".to_owned()));
    arguments.push(Value::from(query.dialect));
    arguments.push(Value::String("LIMIT".to_owned()));
    arguments.push(Value::from(query.offset));
    arguments.push(Value::from(query.count));
    Some(Value::Array(arguments))
}

fn results_value(call: &ResultsCall) -> Option<Value> {
    let params = ValkeyParams::read(&call.litellm_params).ok()?;
    let documents: Vec<ValkeyDocument> = call
        .documents
        .iter()
        .map(|document| ValkeyDocument {
            id: document.id.clone(),
            fields: document.fields.clone(),
        })
        .collect();
    let response = search_response(&documents, &call.query_text, &params.text_field).ok()?;
    serde_json::to_value(response).ok()
}

fn timeouts_value(call: &TimeoutsCall) -> Option<Value> {
    let mut pairs = Vec::with_capacity(call.timeouts.len());
    for timeout in &call.timeouts {
        let chosen = match *timeout {
            TimeoutCall::None => ValkeyTimeout::None,
            TimeoutCall::Seconds { seconds } => {
                ValkeyTimeout::Uniform(Duration::try_from_secs_f64(seconds).ok()?)
            }
            TimeoutCall::Object { connect, read } => ValkeyTimeout::ConnectRead {
                connect: connect.and_then(|seconds| Duration::try_from_secs_f64(seconds).ok()),
                read: read.and_then(|seconds| Duration::try_from_secs_f64(seconds).ok()),
            },
        };
        let timeouts = socket_timeouts(chosen);
        pairs.push(json!([
            timeouts.connect.as_secs_f64(),
            timeouts.read.as_secs_f64()
        ]));
    }
    Some(Value::Array(pairs))
}
