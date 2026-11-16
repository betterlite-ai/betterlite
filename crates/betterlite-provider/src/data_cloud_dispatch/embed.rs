use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::compatible_embedding::request_body;
use betterlite_openai::compatible_wire::header_map;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::gitlab_client::payload::decode_text;
use http::Method;
use secrecy::{ExposeSecret as _, SecretString};

use crate::databricks_auth::environment::DatabricksEnv;
use crate::databricks_auth::oauth::OauthTokenSource;
use crate::databricks_embedding::{EmbeddingPreparation, prepare_embedding};
use crate::oci_common::Clock;
use crate::oci_common::credentials::resolve_oci_credentials;
use crate::oci_common::environment::{DEFAULT_LITELLM_VERSION, OciEnvironment};
use crate::oci_common::signing::sign_oci_request;

use super::call::{EmbeddingCall, EmbeddingReply};
use super::context::{DispatchGlobals, EnvironmentSource, oci_explicit_params};
use super::error::DispatchError;
use super::logging::convert;
use super::select::EmbeddingConfig;
use super::transport::{PostTransport, WireRequest};

mod snowflake;

pub struct EmbeddingContext<'request> {
    pub globals: DispatchGlobals<'request>,
    pub env: EnvironmentSource<'request>,
    pub transport: &'request dyn PostTransport,
    pub tokens: &'request dyn OauthTokenSource,
    pub clock: &'request dyn Clock,
}

pub async fn dispatch(
    call: &EmbeddingCall,
    config: EmbeddingConfig,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingReply, DispatchError> {
    match config {
        EmbeddingConfig::Databricks => databricks(call, context).await,
        EmbeddingConfig::Snowflake => snowflake::complete(call, context).await,
        EmbeddingConfig::Oci => oci(call, context).await,
    }
}

// LiteLLM's DatabricksBase reads the client id, the secret and the user agent from the environment at call time
pub fn databricks_environment(
    environment: &EnvironmentSource<'_>,
) -> Result<DatabricksEnv, DispatchError> {
    let user_agent_override = match environment.text("LITELLM_USER_AGENT")? {
        Some(agent) => Some(agent),
        None => environment.text("DATABRICKS_USER_AGENT")?,
    };
    Ok(DatabricksEnv {
        client_id: environment.text("DATABRICKS_CLIENT_ID")?,
        client_secret: environment.secret("DATABRICKS_CLIENT_SECRET")?,
        api_base: environment.text("DATABRICKS_API_BASE")?,
        user_agent_override,
        version: DEFAULT_LITELLM_VERSION.to_owned(),
    })
}

// the databricks and snowflake arms encode a JSON body next to their provider headers
pub(super) async fn send_json<T: serde::Serialize>(
    context: &EmbeddingContext<'_>,
    url: &str,
    headers: &Headers,
    body: &T,
    deadline: Duration,
) -> Result<UpstreamResponse, DispatchError> {
    let body = serde_json::to_vec(body).map_err(|source| DispatchError::Encode { source })?;
    let headers = header_map(headers).map_err(|source| DispatchError::Headers { source })?;
    let request = WireRequest {
        method: Method::POST,
        url,
        headers: &headers,
        body: &body,
        deadline,
    };
    context.transport.post(&request).await
}

// LiteLLM walks base and key through the call, litellm's globals and the environment
fn databricks_endpoint(
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Result<(Option<String>, Option<SecretString>), DispatchError> {
    let api_base = match call.api_base.clone().filter(|value| !value.is_empty()) {
        Some(base) => Some(base),
        None => match context.globals.api_base {
            Some(base) => Some(base.to_owned()),
            None => context.env.text("DATABRICKS_API_BASE")?,
        },
    };
    let candidates = [
        call.api_key.as_ref(),
        context.globals.api_key,
        context.globals.databricks_key,
    ];
    let usable = candidates
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty());
    let api_key = match usable {
        Some(key) => Some(key.clone()),
        None => context.env.secret("DATABRICKS_API_KEY")?,
    };
    Ok((api_base, api_key))
}

// The databricks arm resolves base and key like _complete_databricks, then B5980's prepare_embedding
async fn databricks(
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingReply, DispatchError> {
    let (api_base, api_key) = databricks_endpoint(call, context)?;
    let environment = databricks_environment(&context.env)?;
    let prepared = prepare_embedding(
        EmbeddingPreparation {
            optional_params: call.optional_params.clone(),
            api_key: api_key.as_ref(),
            api_base: api_base.as_deref(),
            custom_endpoint: false,
            // the databricks embedding arm passes no headers, so extra_headers never reach the SDK fallback
            headers: None,
        },
        &environment,
        context.tokens,
    )
    .await?;
    let data = request_body(
        call.model.clone(),
        &convert(&call.input)?,
        convert(&prepared.optional_params)?,
    );
    let response = send_json(
        context,
        &prepared.url,
        &prepared.headers,
        &data,
        call.timeout,
    )
    .await?;
    if !(200..300).contains(&response.status) {
        return Err(DispatchError::Upstream {
            status: response.status,
            body: decode_text(&response.body),
        });
    }
    let reply: EmbeddingResponse = serde_json::from_slice(&response.body)
        .map_err(|source| DispatchError::Decode { source })?;
    Ok(EmbeddingReply::Databricks(Box::new(reply)))
}

// The oci arm turns absent headers into an empty map, resolves the credentials at call time, then signs
async fn oci(
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingReply, DispatchError> {
    let credentials = resolve_oci_credentials(
        &oci_explicit_params(&call.optional_params)?,
        &context.env.oci()?,
    );
    let given = call.headers.clone().unwrap_or_default();
    let mut headers = header_map(&given).map_err(|source| DispatchError::Headers { source })?;
    crate::oci_embed::validate_environment(&mut headers, &credentials, &OciEnvironment::default())?;
    // LiteLLM's OCIEmbedConfig.get_complete_url reads api_base or litellm.api_base
    let url = crate::oci_embed::get_complete_url(
        &credentials,
        call.api_base.as_deref().or(context.globals.api_base),
    )?;
    let params: crate::oci_embed::OciEmbedParams = convert(&call.optional_params)?;
    let input: EmbeddingInput = convert(&call.input)?;
    let request =
        crate::oci_embed::transform_embedding_request(&call.model, &input, &params, &credentials)?;
    let signed = sign_oci_request(&mut headers, &credentials, &request, &url, context.clock)?;
    let wire = WireRequest {
        method: Method::POST,
        url: &url,
        headers: &headers,
        body: &signed,
        deadline: call.timeout,
    };
    let response = context.transport.post(&wire).await?;
    let reply = crate::oci_embed::response::decode_embed_response(response.status, &response.body)?;
    Ok(EmbeddingReply::Oci(Box::new(reply)))
}
