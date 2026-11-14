use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::azure_prompt_shield::billing::BillingUsage;
use crate::azure_prompt_shield::guardrail::{AzurePromptShieldGuardrail, PromptShieldParams};
use crate::azure_prompt_shield::process::RecordTiming;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::GuardrailSettings;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailSpanSink,
};
use crate::hook_initializers::secret_lookup::SecretLookup;

pub(crate) const CLEAN: &str =
    r#"{"userPromptAnalysis":{"attackDetected":false},"documentsAnalysis":[]}"#;
pub(crate) const NO_FLAG: &str = r#"{"userPromptAnalysis":{},"documentsAnalysis":[]}"#;
pub(crate) const ATTACK: &str =
    r#"{"userPromptAnalysis":{"attackDetected":true},"documentsAnalysis":[]}"#;
pub(crate) const RECORDS: &str = "standard_logging_guardrail_information";
pub(crate) const NOW: RecordTiming = RecordTiming {
    start_time: Some(1.0_f64),
    end_time: Some(1.5_f64),
    duration: Some(0.5_f64),
};

pub(crate) fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

pub(crate) fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn request(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

pub(crate) struct MapSecrets(BTreeMap<String, String>);

impl SecretLookup for MapSecrets {
    fn get_secret_str(&self, name: &str) -> Option<SecretString> {
        self.0
            .get(name)
            .map(|value| SecretString::from(value.as_str()))
    }
}

pub(crate) struct NoSecrets;

impl SecretLookup for NoSecrets {
    fn get_secret_str(&self, _name: &str) -> Option<SecretString> {
        None
    }
}

pub(crate) fn secrets(pairs: &[(&str, &str)]) -> MapSecrets {
    MapSecrets(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    )
}

pub(crate) fn litellm_params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

pub(crate) fn params(api_base: &str) -> PromptShieldParams {
    PromptShieldParams {
        api_key: SecretString::from("azure-key".to_owned()),
        api_base: api_base.to_owned(),
        api_version: None,
        timeout: None,
        cost_tier: None,
        price_per_1000_text_records: None,
        settings: GuardrailSettings::default(),
        strict_modes: true,
        async_handler: base_builder().build().unwrap(),
    }
}

pub(crate) fn priced(
    api_base: &str,
    tier: Option<GuardrailValue>,
    price: Option<GuardrailValue>,
) -> PromptShieldParams {
    let mut params = params(api_base);
    params.cost_tier = tier;
    params.price_per_1000_text_records = price;
    params
}

pub(crate) fn build(
    params: PromptShieldParams,
    lookup: &dyn SecretLookup,
) -> AzurePromptShieldGuardrail {
    let mut guardrail = AzurePromptShieldGuardrail::new(params, lookup).unwrap();
    guardrail.base.settings.guardrail_name = Some("azure-prompt-shield".to_owned());
    guardrail
}

pub(crate) fn guardrail(api_base: &str) -> AzurePromptShieldGuardrail {
    build(params(api_base), &NoSecrets)
}

pub(crate) fn usage_with(chunks: &[&str]) -> BillingUsage {
    let mut usage = BillingUsage::default();
    for chunk in chunks {
        usage.record_chunk(chunk);
    }
    usage
}

pub(crate) struct Log {
    pub(crate) base: GuardrailBase,
    pub(crate) data: BTreeMap<String, GuardrailValue>,
}

impl Log {
    pub(crate) fn new(base: &GuardrailBase, data: BTreeMap<String, GuardrailValue>) -> Self {
        Self {
            base: base.clone(),
            data,
        }
    }

    pub(crate) fn context<'log>(
        &'log mut self,
        sink: &'log dyn GuardrailSpanSink,
    ) -> GuardrailRecordContext<'log> {
        GuardrailRecordContext {
            base: &self.base,
            request_data: &mut self.data,
            log_target: None,
            sink,
        }
    }
}

pub(crate) struct DiscardSink;

impl GuardrailSpanSink for DiscardSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

// serde_json hands a parsed float to the untagged enum as this one-key object
fn plain(value: GuardrailValue) -> GuardrailValue {
    use GuardrailValue::{
        Array, Boolean, Decimal, Null, Object, SignedInteger, Text, UnsignedInteger,
    };
    match value {
        Object(fields) => {
            if let Some(Text(digits)) = fields.get("$serde_json::private::Number") {
                return Decimal(digits.parse().unwrap());
            }
            Object(
                fields
                    .into_iter()
                    .map(|(key, field)| (key, plain(field)))
                    .collect(),
            )
        }
        Array(items) => Array(items.into_iter().map(plain).collect()),
        scalar @ (Null | Boolean(_) | SignedInteger(_) | UnsignedInteger(_) | Decimal(_)
        | Text(_)) => scalar,
    }
}

pub(crate) fn stored(
    data: &BTreeMap<String, GuardrailValue>,
) -> Vec<BTreeMap<String, GuardrailValue>> {
    let Some(GuardrailValue::Object(bucket)) = data.get("metadata") else {
        panic!("no metadata bucket in {data:?}");
    };
    let Some(GuardrailValue::Array(entries)) = bucket.get(RECORDS) else {
        panic!("no guardrail records in {bucket:?}");
    };
    entries
        .iter()
        .map(|entry| match plain(entry.clone()) {
            GuardrailValue::Object(fields) => fields,
            other @ (GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_)) => panic!("a record is an object, got {other:?}"),
        })
        .collect()
}

pub(crate) fn unit(record: &BTreeMap<String, GuardrailValue>, key: &str) -> i64 {
    let Some(GuardrailValue::Object(units)) = record.get("guardrail_usage") else {
        panic!("no guardrail_usage in {record:?}");
    };
    match units.get(key) {
        Some(GuardrailValue::SignedInteger(count)) => *count,
        other => panic!("no {key} count, got {other:?}"),
    }
}

pub(crate) fn cost(record: &BTreeMap<String, GuardrailValue>) -> Option<f64> {
    match record.get("guardrail_cost") {
        None => None,
        Some(GuardrailValue::Decimal(amount)) => Some(*amount),
        other => panic!("a cost is a decimal, got {other:?}"),
    }
}

#[derive(Clone)]
pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) query: Option<String>,
    pub(crate) subscription_key: Option<String>,
    pub(crate) content_type: Option<String>,
    pub(crate) body: String,
}

pub(crate) struct Mock {
    pub(crate) base: String,
    pub(crate) seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinSet<io::Result<()>>,
}

struct Reply {
    status: u16,
    payload: &'static str,
}

pub(crate) async fn mock(replies: Vec<&'static str>) -> Mock {
    let scripted = replies
        .into_iter()
        .map(|payload| Reply {
            status: 200,
            payload,
        })
        .collect();
    mock_with(scripted).await
}

pub(crate) async fn mock_status(status: u16, payload: &'static str) -> Mock {
    mock_with(vec![Reply { status, payload }]).await
}

// The mock answers the n-th request with the n-th reply and every later one with the last.
async fn mock_with(replies: Vec<Reply>) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    let script = Arc::new(replies);
    let answer = move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let request_log = Arc::clone(&recorder);
        let answers = Arc::clone(&script);
        async move {
            let index = record(&request_log, &uri, &headers, &bytes);
            let chosen = answers.get(index).or_else(|| answers.last()).unwrap();
            let mut reply_headers = HeaderMap::new();
            reply_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            (
                StatusCode::from_u16(chosen.status).unwrap(),
                reply_headers,
                chosen.payload,
            )
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

fn record(entries: &Mutex<Vec<Seen>>, uri: &Uri, headers: &HeaderMap, bytes: &Bytes) -> usize {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let mut log = entries.lock().unwrap();
    log.push(Seen {
        path: uri.path().to_owned(),
        query: uri.query().map(str::to_owned),
        subscription_key: header("ocp-apim-subscription-key"),
        content_type: header("content-type"),
        body: String::from_utf8(bytes.to_vec()).unwrap(),
    });
    log.len().saturating_sub(1)
}

pub(crate) fn seen_requests(server: &Mock) -> Vec<Seen> {
    server.seen.lock().unwrap().clone()
}
