use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_foundry::chat::CUSTOM_LLM_PROVIDER;
use betterlite_foundry::chat::capabilities::{RequestParams, map_params, supported_params};
use betterlite_foundry::chat::environment::{
    EnvironmentRequest, TokenSource, validate_environment,
};
use betterlite_foundry::chat::fields::required_params;
use betterlite_foundry::chat::provider::{
    OpenAiModelLists, ProviderEnvironment, ProviderRequest, get_openai_compatible_provider_info,
};
use betterlite_foundry::chat::request::transform_request;
use betterlite_foundry::chat::response::transform_response;
use betterlite_foundry::chat::url::get_complete_url;
use betterlite_openai::chat::response::{ChatChoice, ChatResponse};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::reasoning_models::DropParams;
use betterlite_prompts::arguments::json::Json;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::fixture::Installed;
use super::input::{AuthCall, MapCall, ProviderCall, RequestCall, ResponseCall, UrlCall};

pub(super) fn url_report(call: &UrlCall) -> Value {
    match get_complete_url(call.api_base.as_deref(), call.api_version.as_deref()) {
        Ok(url) => json!({"value": url}),
        Err(_) => json!({"error": {"kind": "ValueError"}}),
    }
}

pub(super) fn supported_report(runtime: &Runtime, installed: &mut Installed, model: &str) -> Value {
    let params = runtime.block_on(supported_params(
        &mut installed.capabilities,
        model,
        &installed.catalog,
        &installed.costs,
    ));
    json!(params)
}

pub(super) fn map_report(runtime: &Runtime, installed: &mut Installed, call: &MapCall) -> Value {
    let mut non_default = call.non_default.clone();
    let mut optional = call.optional.clone();
    let outcome = runtime.block_on(map_params(
        &mut installed.capabilities,
        &mut RequestParams {
            model: &call.model,
            non_default_params: &mut non_default,
            optional_params: &mut optional,
            drop_params: DropParams {
                global: call.global_drop_params,
                request: call.drop_params,
            },
            catalog: &installed.catalog,
        },
        &installed.costs,
    ));
    match outcome {
        Ok(()) => json!({"ok": true, "value": optional, "status": null}),
        Err(error) => json!({"ok": false, "value": null, "status": error.status_code()}),
    }
}

pub(super) fn request_report(call: &RequestCall) -> Value {
    match transform_request(&call.model, &call.messages, &call.optional) {
        Ok(body) => json!({"ok": true, "value": body}),
        Err(_) => json!({"ok": false, "value": null}),
    }
}

pub(super) fn response_report(call: &ResponseCall) -> Value {
    let Ok(body) = serde_json::from_str::<Json>(&call.body) else {
        return json!({"error": {"kind": "JSONDecodeError"}});
    };
    match transform_response(&call.model, body) {
        Ok(response) => project_response(&response),
        Err(_) => json!({"error": {"kind": "ResponseError"}}),
    }
}

fn project_response(response: &ChatResponse) -> Value {
    let usage = json!({
        "prompt_tokens": response.usage.prompt_tokens,
        "completion_tokens": response.usage.completion_tokens,
        "total_tokens": response.usage.total_tokens,
    });
    let choices: Vec<Value> = response.choices.iter().map(project_choice).collect();
    json!({
        "id": response.id,
        "object": response.object,
        "created": response.created,
        "model": response.model,
        "system_fingerprint": response.system_fingerprint,
        "usage": usage,
        "choices": choices,
    })
}

fn project_choice(choice: &ChatChoice) -> Value {
    json!({
        "index": choice.index,
        "message": {"role": choice.message.role, "content": choice.message.content},
        "finish_reason": choice.finish_reason,
    })
}

pub(super) fn provider_report(installed: &Installed, call: &ProviderCall) -> Value {
    let environment_base = environment_value(&call.environment, "AZURE_AI_API_BASE");
    let environment_key =
        environment_value(&call.environment, "AZURE_AI_API_KEY").map(SecretString::from);
    let key = call.api_key.as_deref().map(SecretString::from);
    // get_llm_provider strips the provider prefix before the config tests the model, one prefix in all
    let info = get_openai_compatible_provider_info(ProviderRequest {
        model: &call.model,
        api_base: call.api_base.as_deref(),
        api_key: key.as_ref(),
        custom_llm_provider: CUSTOM_LLM_PROVIDER,
        environment: ProviderEnvironment {
            api_base: environment_base.as_deref(),
            api_key: environment_key.as_ref(),
        },
        lists: OpenAiModelLists {
            chat: &installed.catalog.chat_models,
            text: &installed.catalog.text_models,
            embedding: &installed.embedding,
        },
    });
    json!({
        "provider": info.custom_llm_provider,
        "api_base": info.api_base,
        "api_key": info.api_key.as_ref().map(|value| value.expose_secret().to_owned()),
    })
}

pub(super) fn auth_report(call: &AuthCall) -> Value {
    let argument = call.api_key.as_deref().map(SecretString::from);
    let params = call
        .litellm_params
        .get("api_key")
        .and_then(Value::as_str)
        .map(SecretString::from);
    let configured = configured_key(&call.environment);
    let credentials = deployment_settings();
    let headers: Headers = call.headers.clone();
    let validated = validate_environment(EnvironmentRequest {
        headers,
        argument_key: argument.as_ref(),
        params_key: params.as_ref(),
        configured_key: configured.as_ref(),
        token: TokenSource {
            credentials: &credentials,
            refresh_enabled: false,
            provider: None,
            sources: &NoSources,
        },
        api_base: call.api_base.as_deref(),
    });
    match validated {
        Ok(headers) => json!(headers),
        Err(_) => json!({"error": {"kind": "EnvironmentError"}}),
    }
}

pub(super) fn field_report() -> Value {
    json!(required_params())
}

fn environment_value(environment: &BTreeMap<String, String>, name: &str) -> Option<String> {
    let value = environment.get(name)?;
    (!value.is_empty()).then(|| value.clone())
}

fn configured_key(environment: &BTreeMap<String, String>) -> Option<SecretString> {
    ["AZURE_OPENAI_API_KEY", "AZURE_API_KEY"]
        .iter()
        .find_map(|name| environment_value(environment, name))
        .map(SecretString::from)
}

fn deployment_settings() -> DeploymentSettings {
    DeploymentSettings {
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: String::from("https://cognitiveservices.azure.com/.default"),
        azure_username: None,
        azure_password: None,
    }
}

struct NoSources;

fn missing(kind: &str) -> SourceFailure {
    SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: format!("no {kind} source is configured"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(missing("client secret"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(missing("username and password"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(missing("SDK refresh"))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(missing("OIDC"))
    }
}
