use std::collections::BTreeMap;
use std::sync::PoisonError;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::jwt_claims::TypedValue;
use betterlite_mcp::sso_assertions::assertion::{SsoIdentityAssertion, assertion_from_sso_login};
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::failure::{
    RefreshFailure, RefreshFailureKind,
};
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::transport::RefreshResponseBody;
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::HelperError;

use super::fakes::Harness;

const HEADER_JSON: &[u8] = br#"{"alg":"none"}"#;

pub(crate) type TransportAnswer = Result<RefreshResponseBody, RefreshFailure>;

#[derive(Debug, Deserialize)]
pub(crate) struct ClientConfigInput {
    pub(crate) vars: BTreeMap<String, String>,
}

// The case names an id_token by the claims it carries, so both sides mint the same three-segment text.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AssertionSpec {
    claims: BTreeMap<String, Value>,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct BodySpec {
    #[serde(default)]
    claims: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    id_token: Option<Value>,
    #[serde(default)]
    refresh_token: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum OutcomeSpec {
    Ok {
        #[serde(default)]
        body: BodySpec,
    },
    Rejected {
        detail: String,
    },
    Unavailable {
        detail: String,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccessFailure {
    #[default]
    None,
    Read,
    Write,
}

pub(crate) fn runtime() -> Result<Runtime, Box<HelperError>> {
    let executor = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| value_error(error.to_string()))?;
    Ok(executor)
}

pub(crate) fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn id_token_text(claims: &BTreeMap<String, Value>) -> Result<String, Box<HelperError>> {
    let payload = serde_json::to_string(claims).map_err(|error| value_error(error.to_string()))?;
    let header = URL_SAFE_NO_PAD.encode(HEADER_JSON);
    let body = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    Ok(format!("{header}.{body}."))
}

pub(crate) fn assertion_from_spec(
    spec: &AssertionSpec,
) -> Result<SsoIdentityAssertion, Box<HelperError>> {
    let token = TypedValue {
        json: Value::String(id_token_text(&spec.claims)?),
    };
    let refresh = spec.refresh_token.as_ref().map(|text| TypedValue {
        json: Value::String(text.clone()),
    });
    assertion_from_sso_login(Some(&token), refresh.as_ref())
        .ok_or_else(|| value_error("the case's assertion does not decode".to_owned()))
}

fn body_fields(spec: &BodySpec) -> Result<RefreshResponseBody, Box<HelperError>> {
    let mut body = BTreeMap::new();
    if spec.claims.is_some() && spec.id_token.is_some() {
        return Err(value_error(
            "a body names either claims or id_token, not both".to_owned(),
        ));
    }
    if let Some(claims) = &spec.claims {
        body.insert("id_token".to_owned(), Value::String(id_token_text(claims)?));
    }
    if let Some(raw) = &spec.id_token {
        body.insert("id_token".to_owned(), raw.clone());
    }
    if let Some(rotated) = &spec.refresh_token {
        body.insert("refresh_token".to_owned(), rotated.clone());
    }
    Ok(body)
}

pub(crate) fn scripted(outcome: &OutcomeSpec) -> Result<TransportAnswer, Box<HelperError>> {
    match outcome {
        OutcomeSpec::Ok { body } => Ok(Ok(body_fields(body)?)),
        OutcomeSpec::Rejected { detail } => Ok(Err(RefreshFailure {
            kind: RefreshFailureKind::Rejected,
            detail: detail.clone(),
        })),
        OutcomeSpec::Unavailable { detail } => Ok(Err(RefreshFailure {
            kind: RefreshFailureKind::Unavailable,
            detail: detail.clone(),
        })),
    }
}

pub(crate) fn view(assertion: &SsoIdentityAssertion) -> Value {
    json!({
        "id_token": assertion.id_token.expose_secret(),
        "refresh_token": assertion.refresh_token.as_ref().map(ExposeSecret::expose_secret),
        "issuer": assertion.issuer,
        "expires_at_ms": assertion.expires_at.map(|moment| moment.timestamp_millis()),
    })
}

pub(crate) const fn kind_text(kind: RefreshFailureKind) -> &'static str {
    match kind {
        RefreshFailureKind::Rejected => "rejected",
        RefreshFailureKind::Unavailable => "unavailable",
    }
}

fn pairs(map: &BTreeMap<String, String>) -> Vec<[String; 2]> {
    map.iter()
        .map(|(name, value)| [name.clone(), value.clone()])
        .collect()
}

pub(crate) fn answered(
    result: &str,
    detail: &Value,
    assertion: &Value,
    harness: &Harness,
) -> Value {
    let records = harness.calls.lock().unwrap_or_else(PoisonError::into_inner);
    let first = records.first().cloned();
    let posts = records.len();
    drop(records);
    let made = harness
        .writes
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let writes: Vec<Value> = made
        .iter()
        .map(|(user_id, stored)| json!({"user_id": user_id, "assertion": view(stored)}))
        .collect();
    drop(made);
    json!({
        "result": result,
        "detail": detail,
        "assertion": assertion,
        "url": first.as_ref().map(|call| call.url.clone()),
        "form": first.as_ref().map_or_else(Vec::new, |call| pairs(&call.form)),
        "headers": first.as_ref().map_or_else(Vec::new, |call| pairs(&call.headers)),
        "writes": writes,
        "posts": posts,
    })
}
