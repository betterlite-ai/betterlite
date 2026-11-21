use std::collections::BTreeMap;
use std::num::ParseFloatError;
use std::str::FromStr;

use betterlite_auth::auth_model_patterns::decision::check_model_access;
use betterlite_auth::auth_model_patterns::grant::ModelGrantPattern;
use betterlite_config::sections::request_headers::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::request_destination::RequestData;
use crate::api::request_headers::error::RequestHeadersError;
use crate::api::request_headers::{ForwardedHeaders, HeaderValue, RequestHeaders};
use crate::api::request_principal_metadata::PrincipalFacts;
use crate::api::request_principal_metadata::principal::UserKeyLogMetadata;

const TIMEOUT_HEADER: &str = "x-litellm-timeout";
const STREAM_TIMEOUT_HEADER: &str = "x-litellm-stream-timeout";
const KEEPALIVE_HEADER: &str = "x-litellm-keepalive-seconds";
const NUM_RETRIES_HEADER: &str = "x-litellm-num-retries";
const ORGANIZATION_HEADER: &str = "openai-organization";
const MODEL_FIELD: &str = "model";
// a caller-chosen timeout from any of these must not count as a deployment health signal
const CLIENT_TIMEOUT_FIELDS: [&str; 3] = ["timeout", "request_timeout", "stream_timeout"];

// LitellmDataForBackendLLMCall: what the request headers add to the provider call
#[derive(Debug, Clone, Default)]
pub struct BackendCallArguments {
    pub headers: Option<ForwardedHeaders>,
    pub organization: Option<String>,
    pub timeout: Option<f64>,
    pub stream_timeout: Option<f64>,
    pub client_side_timeout: bool,
    pub num_retries: Option<i64>,
    pub keepalive_seconds: Option<f64>,
}

pub fn backend_call_arguments(
    headers: &ForwardedHeaders,
    request_data: &RequestData,
    principal: &PrincipalFacts,
    settings: &Settings,
) -> Result<BackendCallArguments, RequestHeadersError> {
    let number = |header: &'static str, source: ParseFloatError| RequestHeadersError::Number {
        header,
        source,
    };
    let forwarded = settings
        .forward_client_headers
        .then(|| llm_call_headers(headers, principal, settings.add_user_information))
        .transpose()?
        .filter(|llm_headers| !llm_headers.is_empty());
    let timeout = parsed_header::<f64>(headers, TIMEOUT_HEADER, number)?;
    let stream_timeout = parsed_header::<f64>(headers, STREAM_TIMEOUT_HEADER, number)?;
    let client_side_timeout = timeout.is_some()
        || stream_timeout.is_some()
        || CLIENT_TIMEOUT_FIELDS.iter().any(|field| {
            request_data
                .get(*field)
                .is_some_and(|value| *value != Json::Null)
        });
    Ok(BackendCallArguments {
        headers: forwarded,
        organization: openai_organization(headers, settings.forward_openai_org_id),
        timeout,
        stream_timeout,
        client_side_timeout,
        num_retries: parsed_header::<i64>(headers, NUM_RETRIES_HEADER, |header, source| {
            RequestHeadersError::Integer { header, source }
        })?,
        keepalive_seconds: parsed_header::<f64>(headers, KEEPALIVE_HEADER, number)?,
    })
}

// every x- header except the OpenAI SDK's own x-stainless ones, plus anthropic-beta
pub fn llm_call_headers(
    headers: &ForwardedHeaders,
    principal: &PrincipalFacts,
    add_user_information: bool,
) -> Result<ForwardedHeaders, RequestHeadersError> {
    let mut forwarded: ForwardedHeaders = headers
        .iter()
        .filter(|(name, _value)| {
            let lower = name.to_lowercase();
            (lower.starts_with("x-") && !lower.starts_with("x-stainless"))
                || lower.starts_with("anthropic-beta")
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    if !add_user_information {
        return Ok(forwarded);
    }
    let encoded = serde_json::to_string(&UserKeyLogMetadata::from(principal))
        .map_err(|source| RequestHeadersError::Encode { source })?;
    let fields: IndexMap<String, Json> =
        serde_json::from_str(&encoded).map_err(|source| RequestHeadersError::Encode { source })?;
    for (field, value) in fields {
        let text = match &value {
            Json::Null => continue,
            Json::Text(text) => text.clone(),
            Json::Bool(flag) => flag.to_string(),
            Json::Number(number) => number.to_string(),
            Json::List(_) | Json::Object(_) => serde_json::to_string(&value)
                .map_err(|source| RequestHeadersError::Encode { source })?,
        };
        forwarded.insert(format!("x-litellm-{field}"), HeaderValue::Plain(text));
    }
    Ok(forwarded)
}

impl RequestHeaders<'_> {
    // model_group_settings forwards the headers only for the models it names, aliases and wildcards included
    pub fn model_group_headers(
        &self,
        data: &RequestData,
        headers: &ForwardedHeaders,
        principal: &PrincipalFacts,
        team_model_aliases: &BTreeMap<String, String>,
    ) -> Result<Option<ForwardedHeaders>, RequestHeadersError> {
        let (Some(models), Some(Json::Text(model))) =
            (&self.settings.model_group_forwarding, data.get(MODEL_FIELD))
        else {
            return Ok(None);
        };
        let grant = ModelGrantPattern {
            models: models.clone(),
            team_model_aliases: team_model_aliases.clone(),
            key_model_aliases: BTreeMap::new(),
            team_id: principal.team_id.clone(),
        };
        if !check_model_access(model, &grant, self.model_access) {
            return Ok(None);
        }
        let forwarded = llm_call_headers(headers, principal, self.settings.add_user_information)?;
        Ok((!forwarded.is_empty()).then_some(forwarded))
    }
}

// LiteLLM reads these with float() and int(), which also skip surrounding spaces
fn parsed_header<T>(
    headers: &ForwardedHeaders,
    header: &'static str,
    invalid: fn(&'static str, T::Err) -> RequestHeadersError,
) -> Result<Option<T>, RequestHeadersError>
where
    T: FromStr,
{
    headers
        .get(header)
        .map(|value| {
            value
                .expose()
                .trim()
                .parse::<T>()
                .map_err(|source| invalid(header, source))
        })
        .transpose()
}

fn openai_organization(headers: &ForwardedHeaders, forward: bool) -> Option<String> {
    if !forward {
        return None;
    }
    let (_name, organization) = headers
        .iter()
        .find(|(name, _value)| name.to_lowercase() == ORGANIZATION_HEADER)?;
    tracing::info!(
        "found openai org id: {}, sending to llm",
        organization.expose()
    );
    Some(organization.expose().to_owned())
}
