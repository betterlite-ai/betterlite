use std::collections::{BTreeMap, HashMap};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use aws_smithy_async::{
    rt::sleep::{SharedAsyncSleep, TokioSleep},
    time::{SharedTimeSource, StaticTimeSource},
};
use aws_smithy_runtime_api::client::{
    http::{HttpConnector, HttpConnectorFuture, SharedHttpConnector, http_client_fn},
    orchestrator::{HttpRequest, HttpResponse},
    result::ConnectorError,
};
use aws_smithy_runtime_api::http::StatusCode;
use aws_smithy_types::body::SdkBody;
use aws_types::os_shim_internal::Fs;
use betterlite_bedrock::aws_endpoints::sts::{StsClientKwargs, StsSettings, sts_client_kwargs};
use betterlite_bedrock::aws_settings::sources::NativeSources;
use betterlite_config::tls::Verification;
use serde_json::{Value, json};
use url::{Url, form_urlencoded};

#[derive(Clone, Debug, Default)]
pub(super) struct Record(Arc<Mutex<Vec<Value>>>);

impl Record {
    pub(super) fn add(&self, call: Value) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Debug)]
pub(super) struct Answer {
    pub(super) status: u16,
    pub(super) body: String,
}

#[derive(Clone, Debug)]
pub(super) struct Transport {
    pub(super) answers: BTreeMap<String, Answer>,
    pub(super) record: Record,
}

fn refused(message: &'static str) -> HttpConnectorFuture {
    HttpConnectorFuture::ready(Err(ConnectorError::other(
        Box::new(io::Error::other(message)),
        None,
    )))
}

fn form_fields(request: &HttpRequest) -> BTreeMap<String, String> {
    let body = request.body().bytes().unwrap_or_default();
    form_urlencoded::parse(body)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect()
}

fn credential_scope(request: &HttpRequest) -> Vec<String> {
    let authorization = request.headers().get("authorization").unwrap_or_default();
    authorization
        .split("Credential=")
        .nth(1)
        .and_then(|credential| credential.split(',').next())
        .map(|credential| credential.split('/').map(str::to_owned).collect())
        .unwrap_or_default()
}

fn tags_json(fields: &BTreeMap<String, String>) -> Value {
    let mut tags = Vec::new();
    let mut position = 1_usize;
    loop {
        let key = fields.get(&format!("Tags.member.{position}.Key"));
        let value = fields.get(&format!("Tags.member.{position}.Value"));
        let (Some(found_key), Some(found_value)) = (key, value) else {
            break;
        };
        tags.push(json!([found_key, found_value]));
        position = position.saturating_add(1);
    }
    if tags.is_empty() {
        Value::Null
    } else {
        json!(tags)
    }
}

impl HttpConnector for Transport {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let action = self.record_call(&request);
        let Some(answer) = self.answers.get(&action) else {
            return refused("the fixture has no answer for this action");
        };
        let Ok(status) = StatusCode::try_from(answer.status) else {
            return refused("the fixture answer status is invalid");
        };
        HttpConnectorFuture::ready(Ok(HttpResponse::new(
            status,
            SdkBody::from(answer.body.clone()),
        )))
    }
}

impl Transport {
    fn record_call(&self, request: &HttpRequest) -> String {
        let fields = form_fields(request);
        let scope = credential_scope(request);
        let action = fields.get("Action").cloned().unwrap_or_default();
        let endpoint = Url::parse(request.uri())
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_owned));
        let text = |name: &str| fields.get(name).map_or(Value::Null, |value| json!(value));
        self.record.add(json!({
            "action": action,
            "access_key_id": scope.first().map_or(Value::Null, |value| json!(value)),
            "session_token": request.headers().get("x-amz-security-token").map_or(Value::Null, |value| json!(value)),
            "region": scope.get(2).map_or(Value::Null, |value| json!(value)),
            "endpoint": endpoint.map_or(Value::Null, |value| json!(value)),
            "role_arn": text("RoleArn"),
            "session_name": text("RoleSessionName"),
            "web_identity_token": text("WebIdentityToken"),
            "duration_seconds": fields.get("DurationSeconds").and_then(|value| value.parse::<i64>().ok()).map_or(Value::Null, |value| json!(value)),
            "policy": text("Policy"),
            "external_id": text("ExternalId"),
            "tags": tags_json(&fields),
        }));
        action
    }
}

pub(super) fn sources(
    environment: &BTreeMap<String, String>,
    files: &[(String, Vec<u8>)],
    transport: Transport,
) -> NativeSources {
    NativeSources {
        environment: environment.clone(),
        filesystem: Fs::from_map(
            files
                .iter()
                .map(|(path, bytes)| (path.clone(), bytes.clone()))
                .collect::<HashMap<_, _>>(),
        ),
        client: http_client_fn(move |_, _| SharedHttpConnector::new(transport.clone())),
        clock: SharedTimeSource::new(StaticTimeSource::new(UNIX_EPOCH)),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(5),
    }
}

pub(super) fn kwargs(
    environment: &BTreeMap<String, String>,
    endpoint: Option<&str>,
    region: Option<&str>,
    verify: Option<bool>,
) -> Result<StsClientKwargs, String> {
    let settings = StsSettings {
        endpoint,
        aws_region: environment.get("AWS_REGION").map(String::as_str),
        aws_default_region: environment.get("AWS_DEFAULT_REGION").map(String::as_str),
        configured_region: region,
    };
    let verification = if verify.unwrap_or(true) {
        Verification::Enabled
    } else {
        Verification::Disabled
    };
    sts_client_kwargs(&settings, verification).map_err(|error| error.to_string())
}
