use std::convert::Infallible;
use std::future::{Future, ready};

use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::ThinkingError;
use betterlite_anthropic::capabilities::reasoning::ReasoningEffortError;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_core::api::messages_native_transform::capabilities::{
    LegacyShapes, Thinking, ThinkingCapabilities,
};
use betterlite_core::api::vertex_messages::{NativeMessages, VertexMessagesOverrides, adapt};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json as ContractJson;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::credential_sources::getters::VertexParams;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::probe::AccessToken;
use betterlite_vertex::vertex_messages::body::{self, OutputSanitizer};
use betterlite_vertex::vertex_messages::environment::{
    Environment, EnvironmentRequest, get_complete_url, validate_environment,
};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

mod tests;

const BETA_HEADER: &str = "anthropic-beta";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    access_token: String,
    credential_project: String,
    model: String,
    messages: Vec<Json>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    headers: IndexMap<String, String>,
    #[serde(default)]
    vertex_project: Option<String>,
    #[serde(default)]
    vertex_location: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_config_error()?;
        runtime.block_on(answer(input))
    }
}

// The cases send no thinking parameter, so the parent transform never asks a capability question but the mid-system one.
struct NoCapabilities;

impl ThinkingCapabilities for NoCapabilities {
    fn supports(
        &mut self,
        _model: &str,
        _key: &str,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn map_effort<'seam>(
        &'seam mut self,
        _model: &'seam str,
        _effort: &'seam str,
        _provider: &'seam str,
    ) -> impl Future<Output = Result<Option<Thinking>, ReasoningEffortError>> + Send + 'seam {
        ready(Ok(None))
    }

    fn validate_effort(
        &mut self,
        _model: &str,
        _effort: Option<&str>,
        _provider: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn supports_effort_param(
        &mut self,
        _model: &str,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn drop_disabled_thinking(
        &mut self,
        _model: &str,
        _params: &mut OptionalParams,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn translate_legacy_thinking<'seam>(
        &'seam mut self,
        _request: &'seam mut LegacyShapes<'seam>,
    ) -> impl Future<Output = Result<(), ThinkingError>> + Send + 'seam {
        ready(Ok(()))
    }
}

// B1333 owns the output-parameter policy with its own cases; no case here carries a body-level output_config.
struct NoSanitizer;

impl OutputSanitizer for NoSanitizer {
    fn sanitize<'body>(
        &'body mut self,
        _body: &'body mut IndexMap<String, ContractJson>,
        _model: &'body str,
    ) -> impl Future<Output = ()> + Send + 'body {
        ready(())
    }
}

struct VertexOverrides;

impl VertexMessagesOverrides for VertexOverrides {
    type Error = Infallible;

    fn apply<'body>(
        &'body mut self,
        body: &'body mut Object,
        model: &'body str,
    ) -> impl Future<Output = Result<(), Infallible>> + Send + 'body {
        async move {
            body::apply(body, model, &mut NoSanitizer).await;
            Ok(())
        }
    }
}

async fn answer(input: &Input) -> Result<Value, Box<HelperError>> {
    let (headers, beta, url) = headers_answer(input).await?;
    let body = body_answer(input).await?;
    // the cases prove the failure branch too, since the Python side raises the same message.
    let missing_base = match get_complete_url(None) {
        Ok(_base) => None,
        Err(error) => Some(error.to_string()),
    };
    Ok(json!({
        "headers": headers,
        "beta": beta,
        "url": url,
        "missing_base": missing_base,
        "body": body,
    }))
}

async fn environment_answer(input: &Input) -> Result<Environment, Box<HelperError>> {
    let catalog = ModelCatalog::parse(b"{}", UnknownFields::Warn).or_config_error()?;
    let settings = VertexSettings::default();
    let headers = AnthropicHeaders {
        values: input.headers.clone(),
    };
    let token = input.access_token.clone();
    let fallback = input.credential_project.clone();
    validate_environment(
        &EnvironmentRequest {
            headers: &headers,
            model: &input.model,
            messages: &input.messages,
            optional_params: &input.optional_params,
            params: VertexParams {
                project: input.vertex_project.as_deref(),
                location: input.vertex_location.as_deref(),
                ..VertexParams::default()
            },
            settings: &settings,
            catalog: &catalog,
            api_base: input.api_base.as_deref(),
        },
        move |_credentials, project| {
            ready(Ok::<AccessToken, CredentialCacheError>(AccessToken {
                token: SecretString::from(token),
                project: project.map_or_else(|| fallback.clone(), str::to_owned),
            }))
        },
    )
    .await
    .or_value_error()
}

async fn headers_answer(input: &Input) -> Result<(Object, Vec<String>, String), Box<HelperError>> {
    let environment = environment_answer(input).await?;
    let mut headers: Object = environment
        .headers
        .values
        .into_iter()
        .map(|(name, value)| (name, Json::Text(value)))
        .collect();
    let mut beta: Vec<String> = match headers.shift_remove(BETA_HEADER) {
        Some(Json::Text(text)) => text.split(',').map(str::to_owned).collect(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => Vec::new(),
    };
    beta.sort();
    // Python joins a set, so both sides report the membership sorted.
    let url = get_complete_url(Some(&environment.api_base)).or_value_error()?;
    Ok((headers, beta, url))
}

async fn body_answer(input: &Input) -> Result<Object, Box<HelperError>> {
    adapt(
        &mut NoCapabilities,
        NativeMessages {
            model: &input.model,
            messages: input.messages.clone(),
            optional_params: input.optional_params.clone(),
        },
        &mut VertexOverrides,
    )
    .await
    .or_value_error()
}
