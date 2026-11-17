use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::credentials::{ExplicitCredentials, LiteLlmGlobals, get_openai_credentials};
use betterlite_openai::embedding::EncodingFormat;
use betterlite_openai::openai_base_config::{Headers, Object, OpenAiSettings};
use http::Method;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::Number;

use crate::operation::OperationError;
use crate::prepared::body::{chat_fields, encode, request_fields};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedChat, PreparedHttp, PreparedRequest};
use crate::transform_retry::TransformRetry;

use super::EMBEDDING_BODY_LIMIT;
use super::call::{ChatCall, Composition, Credentials, EmbeddingCall, Resolved};
use super::error::OpenAiProviderError;
use super::headers::{chat_headers, embedding_headers, present_text, resolved_headers};
use super::metadata::requester_metadata;

const NONE_FORMAT: &str = "none";

// What a prepared chat call keeps for the failure log.
pub struct ChatPlan {
    pub prepared: PreparedRequest,
    pub api_base: String,
    pub headers: Option<Headers>,
}

// B500 resolves the base, the key and the organization the same way for both the chat and the text branch.
#[must_use]
pub fn resolve(composition: &Composition, explicit: &Credentials) -> Resolved {
    let globals = LiteLlmGlobals {
        api_base: composition.globals.api_base.clone(),
        api_key: composition.globals.api_key.clone(),
        openai_key: composition.globals.openai_key.clone(),
        organization: composition.globals.organization.clone(),
    };
    let resolved = get_openai_credentials(
        ExplicitCredentials {
            api_base: explicit.api_base.clone(),
            api_key: explicit.api_key.clone(),
            organization: explicit.organization.clone(),
        },
        globals,
        &composition.credentials,
        composition.env_api_key.clone(),
    );
    Resolved {
        api_base: resolved.api_base,
        api_key: resolved.api_key,
        organization: resolved.organization,
    }
}

pub fn chat_plan(
    composition: &Composition,
    call: &ChatCall,
) -> Result<ChatPlan, OpenAiProviderError> {
    let credentials = resolve(composition, &call.credentials);
    // the experimental handler never receives the organization, so that branch sends no organization header
    let organization = if composition.provider.experimental_base_llm_http_handler {
        None
    } else {
        credentials.organization.clone()
    };
    let deployment = build_deployment(&call.model, call.timeout, &credentials, organization)?;
    let mut body = chat_fields(&call.request, &call.model, &composition.catalog)?;
    if composition.provider.enable_preview_features {
        if let Some(metadata) = call.metadata.as_ref().and_then(requester_metadata) {
            body.insert(String::from("metadata"), metadata);
        }
    }
    merge_defaults(&mut body, &composition.chat_defaults)?;
    let body = encode(body)?;
    // completion() hands this branch the caller's headers already merged with its extra headers
    let headers = resolved_headers(
        composition,
        call.headers.as_ref(),
        call.extra_headers.as_ref(),
    );
    let http = PreparedHttp {
        method: Method::POST,
        url: deployment.endpoint.clone(),
        headers: chat_headers(
            &deployment,
            headers.as_ref(),
            composition.provider.experimental_base_llm_http_handler,
        )?,
        body,
        deadline: deployment.timeout,
        body_limit: None,
    };
    let prepared = PreparedChat {
        http,
        retry: TransformRetry::DISABLED,
        // the provider's own model names the reply, exactly as the OpenAI transform leaves it
        client_model: None,
    };
    let prepared = if call.request.stream {
        PreparedRequest::Stream(Box::new(prepared))
    } else {
        PreparedRequest::Chat(Box::new(prepared))
    };
    Ok(ChatPlan {
        prepared,
        api_base: deployment.api_base,
        headers,
    })
}

pub fn embedding_plan(
    composition: &Composition,
    call: &EmbeddingCall,
) -> Result<PreparedRequest, OpenAiProviderError> {
    let mut credentials = resolve(composition, &call.credentials);
    // the embedding branch reads no explicit organization, so the deployment's chain starts at the global one
    credentials.organization = present_text(composition.globals.organization.as_deref())
        .or_else(|| present_text(composition.credentials.organization.as_deref()))
        .map(str::to_owned);
    let deployment = build_deployment(
        &call.model,
        call.timeout,
        &credentials,
        credentials.organization.clone(),
    )?;
    let mut fields = request_fields(&call.request, "embeddings")?;
    fields.insert(String::from("model"), JsonValue::Text(call.model.clone()));
    fields.shift_remove("mock_response");
    fields.shift_remove("mock_tool_calls");
    match requested_encoding_format(composition, call) {
        Some(format) => {
            fields.insert(String::from("encoding_format"), JsonValue::Text(format));
        }
        None => {
            fields.shift_remove("encoding_format");
        }
    }
    let headers = embedding_headers(&deployment, call.headers.as_ref())?;
    let http = PreparedHttp {
        method: Method::POST,
        url: deployment.endpoint_for("embeddings"),
        headers,
        body: encode(fields)?,
        deadline: deployment.timeout,
        body_limit: Some(EMBEDDING_BODY_LIMIT),
    };
    Ok(PreparedRequest::Embedding(Box::new(PreparedChat {
        http,
        retry: TransformRetry::DISABLED,
        client_model: call.request.model.clone(),
    })))
}

// The explicit argument wins, then the body's value, then the configured default; a none sentinel drops the field.
fn requested_encoding_format(composition: &Composition, call: &EmbeddingCall) -> Option<String> {
    let explicit = present_text(call.encoding_format.as_deref());
    let requested = explicit
        .map(str::to_owned)
        .or_else(|| {
            call.request
                .encoding_format
                .map(encoding_text)
                .map(str::to_owned)
        })
        .or_else(|| {
            composition
                .provider
                .default_embedding_encoding_format
                .clone()
        });
    requested.filter(|format| !is_none_sentinel(format))
}

const fn encoding_text(format: EncodingFormat) -> &'static str {
    match format {
        EncodingFormat::Float => "float",
        EncodingFormat::Base64 => "base64",
    }
}

fn is_none_sentinel(format: &str) -> bool {
    format.trim().to_lowercase() == NONE_FORMAT
}

// The chat branch merges every configured default that the call and its provider filter left absent.
fn merge_defaults(
    body: &mut IndexMap<String, JsonValue>,
    defaults: &OpenAiSettings,
) -> Result<(), OpenAiProviderError> {
    let serialized = serde_json::to_value(defaults)
        .map_err(|source| OpenAiProviderError::Operation(OperationError::Encode { source }))?;
    let fields: Object = serde_json::from_value(serialized)
        .map_err(|source| OpenAiProviderError::Operation(OperationError::Encode { source }))?;
    for (name, value) in fields {
        if body.contains_key(&name) {
            continue;
        }
        let converted: JsonValue = serde_json::from_value(value)
            .map_err(|source| OpenAiProviderError::Operation(OperationError::Encode { source }))?;
        body.insert(name, converted);
    }
    Ok(())
}

fn build_deployment(
    model: &str,
    timeout: Option<f64>,
    credentials: &Resolved,
    organization: Option<String>,
) -> Result<Deployment, OpenAiProviderError> {
    let mut params = IndexMap::new();
    params.insert(String::from("model"), JsonValue::Text(model.to_owned()));
    params.insert(
        String::from("api_base"),
        JsonValue::Text(credentials.api_base.clone()),
    );
    if let Some(key) = &credentials.api_key {
        params.insert(
            String::from("api_key"),
            JsonValue::Text(key.expose_secret().to_owned()),
        );
    }
    if let Some(organization) = organization {
        params.insert(String::from("organization"), JsonValue::Text(organization));
    }
    if let Some(seconds) = timeout {
        let number = Number::from_f64(seconds).ok_or(OpenAiProviderError::FieldType {
            field: "timeout",
            expected: "a number of seconds",
        })?;
        params.insert(String::from("timeout"), JsonValue::Number(number));
    }
    Ok(Deployment::from_params(params)?)
}
