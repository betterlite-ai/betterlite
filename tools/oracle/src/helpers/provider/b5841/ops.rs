use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_provider::databricks_auth::environment::DatabricksEnv;
use betterlite_provider::databricks_auth::oauth::{
    OauthReply, OauthTokenRequest, OauthTokenSource,
};
use betterlite_provider::databricks_chat::messages::transform_messages;
use betterlite_provider::databricks_chat::request::params::map_openai_params;
use betterlite_provider::databricks_chat::request::{
    EnvironmentRequest, get_complete_url, validate_environment,
};
use betterlite_provider::databricks_chat::response::{ResponseInput, transform_response};
use betterlite_provider::databricks_chat::stream::ChunkParser;
use betterlite_provider::ollama_chat::ids::RandomIds;
use secrecy::SecretString;
use serde_json::json;
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::capabilities::FakeCapabilities;
use super::cases::{HeadersCase, MessagesCase, ParamsCase, ResponseCase, StreamCase, UrlCase};

const PROVIDER: &str = "databricks";
const VERSION: &str = "1.105.1";

pub(super) fn url_of(case: &UrlCase) -> serde_json::Value {
    match get_complete_url(case.api_base.as_deref(), &case.model) {
        Ok(url) => json!({"url": url}),
        Err(error) => json!({"status": error.status_code()}),
    }
}

pub(super) fn headers(cases: &[HeadersCase]) -> HelperResult {
    let runtime = Builder::new_current_thread().build().or_config_error()?;
    let results = cases
        .iter()
        .map(|case| headers_of(case, &runtime))
        .collect::<Vec<_>>();
    Ok(json!(results))
}

fn headers_of(case: &HeadersCase, runtime: &Runtime) -> serde_json::Value {
    let mut optional = case.optional_params.clone();
    let key = case.api_key.as_deref().map(SecretString::from);
    let request = EnvironmentRequest {
        api_key: key.as_ref(),
        api_base: case.api_base.as_deref(),
        headers: case.headers.clone(),
        litellm_user_agent: case.user_agent.as_deref(),
        env_user_agent: None,
    };
    let env = DatabricksEnv {
        client_id: None,
        client_secret: None,
        api_base: None,
        user_agent_override: None,
        version: VERSION.to_owned(),
    };
    match runtime.block_on(validate_environment(
        request,
        &mut optional,
        &env,
        &NoTokens,
    )) {
        Ok(found) => json!({"headers": found, "optional_params": optional}),
        Err(error) => json!({"status": error.status_code()}),
    }
}

// an api key request never asks for an OAuth token, so the source refuses instead of sending
struct NoTokens;

#[async_trait]
impl OauthTokenSource for NoTokens {
    async fn send(&self, _request: &OauthTokenRequest) -> OauthReply {
        OauthReply::Transport(String::from("the b5841 helper has no OAuth source"))
    }
}

pub(super) fn params(cases: &[ParamsCase]) -> HelperResult {
    let runtime = Builder::new_current_thread().build().or_config_error()?;
    let results = cases
        .iter()
        .map(|case| params_of(case, &runtime))
        .collect::<Vec<_>>();
    Ok(json!(results))
}

fn params_of(case: &ParamsCase, runtime: &Runtime) -> serde_json::Value {
    let mut optional = case.optional_params.clone();
    let mut capabilities = FakeCapabilities;
    let mapping = map_openai_params(
        &case.non_default_params,
        &mut optional,
        &case.model,
        true,
        &mut capabilities,
    );
    if let Err(error) = runtime.block_on(mapping) {
        return json!({"status": error.status_code()});
    }
    json!({"params": optional})
}

pub(super) fn messages_of(case: &MessagesCase) -> serde_json::Value {
    match transform_messages(&case.messages, &case.model, &RefuseDownloads) {
        Ok(transformed) => json!({"messages": transformed}),
        Err(error) => json!({"status": error.status_code()}),
    }
}

pub(super) fn response_of(case: &ResponseCase) -> serde_json::Value {
    let Ok(body) = serde_json::to_string(&case.body) else {
        return json!({"status": 500});
    };
    let outcome = transform_response(&ResponseInput {
        body: &body,
        status_code: 200,
        headers: BTreeMap::new(),
        custom_llm_provider: Some(PROVIDER),
        json_mode: case.json_mode,
        ids: &RandomIds,
    });
    match outcome {
        Ok(reply) => json!({"reply": reply}),
        Err(error) => json!({"status": error.status_code()}),
    }
}

pub(super) fn stream_of(case: &StreamCase) -> serde_json::Value {
    let mut parser = ChunkParser {
        json_mode: case.json_mode,
        last_function_name: None,
    };
    let mut chunks = Vec::with_capacity(case.chunks.len());
    for chunk in &case.chunks {
        let Ok(text) = serde_json::to_string(chunk) else {
            return json!({"status": 500});
        };
        match parser.parse(&text, &RandomIds) {
            Ok(parsed) => chunks.push(parsed),
            Err(error) => return json!({"status": error.status_code()}),
        }
    }
    json!({"chunks": chunks})
}
