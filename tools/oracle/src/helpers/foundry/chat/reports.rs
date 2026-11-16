use betterlite_config::sections::Section;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_config::sections::azure_url_policy::Raw as KeyRaw;
use betterlite_foundry::chat::capabilities::{RequestParams, map_params, supported_params};
use betterlite_foundry::chat::environment::{
    EnvironmentRequest, TokenSource, validate_environment,
};
use betterlite_foundry::chat::provider::{OpenAiModelLists, ProviderRequest, provider_info};
use betterlite_foundry::chat::request::transform_request;
use betterlite_foundry::chat::response::transform_response;
use betterlite_foundry::chat::url::get_complete_url;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::chat::response::{ChatChoice, ChatResponse};
use betterlite_openai::reasoning_models::DropParams;
use betterlite_prompts::arguments::json::Json;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::fixture::Installed;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

use super::input::{AuthCall, MapCall, ProviderCall, RequestCall, ResponseCall, UrlCall};
use super::sources::Offline;

pub(super) fn url_report(call: &UrlCall) -> Value {
    let Ok(url) = get_complete_url(call.api_base.as_deref(), call.api_version.as_deref()) else {
        return json!({"error": {"kind": "ValueError"}});
    };
    json!({"value": url})
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
    let Ok(body) = transform_request(&call.model, &call.messages, &call.optional) else {
        return json!({"ok": false, "value": null});
    };
    json!({"ok": true, "value": body})
}

pub(super) fn response_report(call: &ResponseCall) -> Value {
    let Ok(body) = serde_json::from_str::<Json>(&call.body) else {
        return json!({"error": {"kind": "JSONDecodeError"}});
    };
    let Ok(response) = transform_response(&call.model, body) else {
        return json!({"error": {"kind": "ResponseError"}});
    };
    project_response(&response)
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

// an empty variable counts as unset
fn environment_defaults(call: &ProviderCall) -> (Option<&str>, Option<SecretString>) {
    let base = call
        .environment
        .get("AZURE_AI_API_BASE")
        .filter(|value| !value.is_empty())
        .map(String::as_str);
    let key = call
        .environment
        .get("AZURE_AI_API_KEY")
        .filter(|value| !value.is_empty())
        .map(|value| SecretString::from(value.as_str()));
    (base, key)
}

pub(super) fn provider_report(installed: &Installed, call: &ProviderCall) -> Value {
    let (environment_base, environment_key) = environment_defaults(call);
    let key = call.api_key.as_deref().map(SecretString::from);
    // get_llm_provider strips the provider prefix before the config tests the model, one prefix in all
    let info = provider_info(&ProviderRequest {
        model: &call.model,
        api_base: call.api_base.as_deref(),
        api_key: key.as_ref(),
        custom_llm_provider: ProviderId::AzureAi,
        environment_base,
        environment_key: environment_key.as_ref(),
        lists: OpenAiModelLists {
            chat: &installed.catalog.chat_models,
            text: &installed.catalog.text_models,
            embedding: &installed.embedding,
        },
    });
    json!({
        "provider": info.custom_llm_provider.to_string(),
        "api_base": info.api_base,
        "api_key": info.api_key.as_ref().map(|value| value.expose_secret().to_owned()),
    })
}

// litellm.api_key, litellm.azure_key and the Azure key variables, through the production config section
pub(super) fn auth_report(call: &AuthCall) -> Result<Value, Box<HelperError>> {
    let argument = call.api_key.as_deref().map(SecretString::from);
    let params = call
        .litellm_params
        .get("api_key")
        .and_then(Value::as_str)
        .map(SecretString::from);
    let source = json!({
        "litellm_settings": {"api_key": null, "azure_key": null},
        "model_list": [{"litellm_params": {}}],
    });
    let raw: KeyRaw = serde_json::from_value(source).or_config_error()?;
    let resolved = Section::convert(raw, &call.environment).or_config_error()?;
    let configured = resolved
        .deployments
        .into_iter()
        .next()
        .and_then(|item| item.api_key);
    let credentials = DeploymentSettings {
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: String::from("https://cognitiveservices.azure.com/.default"),
        azure_username: None,
        azure_password: None,
    };
    let validated = validate_environment(EnvironmentRequest {
        headers: call.headers.clone(),
        argument_key: argument.as_ref(),
        params_key: params.as_ref(),
        configured_key: configured.as_ref(),
        token: TokenSource {
            credentials: &credentials,
            refresh_enabled: false,
            provider: None,
            sources: &Offline,
        },
        api_base: call.api_base.as_deref(),
    });
    let Ok(headers) = validated else {
        return Ok(json!({"error": {"kind": "EnvironmentError"}}));
    };
    Ok(json!(headers))
}
