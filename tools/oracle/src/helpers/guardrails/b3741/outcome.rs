use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::agent_365_tokens::Agent365Guardrail;
use betterlite_guardrails::agent_365_tokens::error::Agent365TokenError;
use betterlite_guardrails::agent_365_tokens::exchange::entra_assertion;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use url::form_urlencoded;

use super::poster::{ScriptedReplies, guardrail, runtime};
use super::{FIXED_NOW, Step, TokenExchangeArgs};
use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

fn token_outcome(result: Result<SecretString, Agent365TokenError>) -> Value {
    match result {
        Ok(token) => json!({"kind": "token", "access_token": token.expose_secret()}),
        Err(Agent365TokenError::Exchange(error)) => json!({
            "kind": "token",
            "error": "exchange",
            "status_code": error.status_code,
            "error_code": error.error_code,
            "description": error.description,
            "aadsts_codes": error.aadsts_codes,
            "gateway_owned": error.gateway_owned(),
        }),
        Err(Agent365TokenError::Throttled { status }) => {
            json!({"kind": "token", "error": "throttled", "status_code": status})
        }
        Err(Agent365TokenError::Malformed { message }) => {
            json!({"kind": "token", "error": "malformed", "message": message})
        }
        Err(Agent365TokenError::HttpStatus { status }) => {
            json!({"kind": "token", "error": "http_status", "status_code": status})
        }
        Err(Agent365TokenError::Poster(error)) => {
            json!({"kind": "token", "error": "poster", "message": error.to_string()})
        }
    }
}

async fn range_outcome(guardrail: &Agent365Guardrail, prefix: &str, count: usize) -> Value {
    let mut token = String::new();
    for index in 0..count {
        match guardrail.get_obo_token(&format!("{prefix}{index}")).await {
            Ok(fetched) => token = fetched.expose_secret().to_owned(),
            Err(error) => return token_outcome(Err(error)),
        }
    }
    json!({"kind": "token_range", "access_token": token})
}

fn request_headers(entries: &BTreeMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in entries {
        let header_name = HeaderName::from_bytes(name.as_bytes()).or_value_error()?;
        let header_value = HeaderValue::from_str(value).or_value_error()?;
        let _previous = headers.insert(header_name, header_value);
    }
    Ok(headers)
}

async fn post_outcome(
    guardrail: &Agent365Guardrail,
    url: &str,
    headers: &BTreeMap<String, String>,
    data: &BTreeMap<String, String>,
) -> Result<Value, Box<HelperError>> {
    let built = request_headers(headers)?;
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (name, value) in data {
        serializer.append_pair(name, value);
    }
    let body = serializer.finish();
    match guardrail
        .post_allowing_error_status(url, &built, body.as_bytes())
        .await
    {
        Ok(reply) => Ok(json!({
            "kind": "post",
            "status": reply.status.as_u16(),
            "body": serde_json::from_slice::<Value>(&reply.body).ok(),
        })),
        Err(error) => Ok(token_outcome(Err(error))),
    }
}

async fn step_outcome(
    guardrail: &Agent365Guardrail,
    step: &Step,
) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Token { assertion } => Ok(token_outcome(guardrail.get_obo_token(assertion).await)),
        Step::TokenRange { prefix, count } => Ok(range_outcome(guardrail, prefix, *count).await),
        Step::Post { url, headers, data } => post_outcome(guardrail, url, headers, data).await,
    }
}

async fn run_exchange(
    guardrail: &Agent365Guardrail,
    steps: &[Step],
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut outcomes = Vec::new();
    for step in steps {
        outcomes.push(step_outcome(guardrail, step).await?);
    }
    Ok(outcomes)
}

fn ttl_seconds(expires_at: f64) -> u64 {
    // float_arithmetic is denied; a fused multiply-add subtracts the fixed clock time
    let remaining = FIXED_NOW.mul_add(-1.0_f64, expires_at);
    Duration::try_from_secs_f64(remaining).map_or(0, |duration| duration.as_secs())
}

fn cache_entries(guardrail: &Agent365Guardrail) -> Vec<Value> {
    let cache = guardrail
        .cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    cache
        .snapshot()
        .into_iter()
        .map(|(key, token)| {
            let expires_at = cache.expiry(&key).unwrap_or(FIXED_NOW);
            json!([key, token.expose_secret(), ttl_seconds(expires_at)])
        })
        .collect()
}

pub(super) fn exchange_result(args: &TokenExchangeArgs) -> HelperResult {
    let poster = Arc::new(ScriptedReplies::new(&args.replies, args.repeat_last_reply));
    let guardrail = guardrail(Arc::clone(&poster))?;
    let outcomes = runtime()?.block_on(run_exchange(&guardrail, &args.steps))?;
    Ok(json!({
        "requests": poster.recorded(),
        "steps": outcomes,
        "cache": cache_entries(&guardrail),
    }))
}

pub(super) fn assertion_result(values: &[String]) -> HelperResult {
    let mut accepted = Vec::new();
    for value in values {
        let guard = GuardrailValue::Text(value.clone());
        if entra_assertion(&guard).is_some() {
            accepted.push(value);
        }
    }
    Ok(json!({"accepted": accepted}))
}
