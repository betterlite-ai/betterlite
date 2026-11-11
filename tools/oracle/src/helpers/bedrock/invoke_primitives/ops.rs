use std::collections::BTreeMap;
use std::fmt::Display;
use std::future::ready;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_bedrock::aws_endpoints::provider::{BedrockInvokeProvider, invoke_provider};
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::invoke_primitives::config::{
    Params, SUPPORTED_OPENAI_PARAMS, apply_config_to_params, map_openai_params,
};
use betterlite_bedrock::invoke_primitives::environment::{error_class, validate_environment};
use betterlite_bedrock::invoke_primitives::guardrail::guardrail_headers;
use betterlite_bedrock::invoke_primitives::prompt::{PromptConversion, convert_messages_to_prompt};
use betterlite_bedrock::invoke_primitives::signing::{SignRequest, sign_request};
use betterlite_bedrock::invoke_primitives::url::{
    CompleteUrl, InvokeEnvironment, invoke_model_url,
};
use betterlite_bedrock::request_policy::{
    BedrockRequestPolicy, Params as PolicyParams, RequestValue,
};
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use chrono::{DateTime, TimeZone as _, Utc};
use indexmap::IndexMap;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::input::{ErrorCase, Input, MergeCase, PromptCase, SigningCase, UrlCase};
use crate::helpers::{HelperError, HelperResult};

pub(crate) fn run(input: &Input) -> HelperResult {
    match input {
        Input::BaseOptions {
            non_default,
            optional,
            config,
        } => base_options(non_default, optional, config),
        Input::ProviderFamilySelection { models } => Ok(provider_selection(models)),
        Input::InvokeModelUrl { cases } => model_urls(cases),
        Input::GuardrailHeaders { values } => Ok(guardrail_cases(values)),
        Input::GuardrailMetadataMerge { merges, error } => metadata_merge(merges, error),
        Input::SigningForwarding(case) => signing(case),
        Input::PromptTemplateSelection { cases } => prompts(cases, true),
        Input::LlamaLocalTemplate {
            local_assets,
            cases,
        } => prompts(cases, *local_assets),
    }
}

fn failure<E>(class: &'static str, error: &E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    let mut builder = Builder::new_current_thread();
    builder.enable_all();
    builder.build().map_err(|error| failure("Runtime", &error))
}

fn epoch(seconds: i64) -> Result<SystemTime, Box<HelperError>> {
    let bounded = u64::try_from(seconds).map_err(|error| failure("ValueError", &error))?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(bounded))
        .ok_or_else(|| failure("ValueError", &"now is out of range"))
}

fn clock() -> Result<DateTime<Utc>, Box<HelperError>> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| failure("ValueError", &"the fixture clock is invalid"))
}

fn template_factory(local_assets: bool) -> PromptFactory {
    let cache = if local_assets {
        builtin_entries()
    } else {
        BTreeMap::new()
    };
    PromptFactory {
        cache: Mutex::new(cache),
        hub: Hub::Unreachable,
    }
}

fn base_options(non_default: &Params, optional: &Params, config: &Params) -> HelperResult {
    let mut params = map_openai_params(non_default, optional.clone());
    apply_config_to_params(config, &mut params);
    let mapped =
        serde_json::to_value(&params).map_err(|error| failure("SerializationError", &error))?;
    Ok(json!({"supported": SUPPORTED_OPENAI_PARAMS, "mapped": mapped}))
}

fn provider_selection(models: &[String]) -> Value {
    let providers: Vec<Option<&'static str>> = models
        .iter()
        .map(|model| invoke_provider(model).map(BedrockInvokeProvider::name))
        .collect();
    json!({"providers": providers})
}

fn model_urls(cases: &[UrlCase]) -> HelperResult {
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        results.push(model_url(case)?);
    }
    Ok(json!({"results": results}))
}

fn model_url(case: &UrlCase) -> Result<Value, Box<HelperError>> {
    let config = load(&case.environment)?;
    let settings = &config.sections.bedrock_aws_endpoints;
    let result = invoke_model_url(CompleteUrl {
        api_base: case.api_base.as_deref(),
        model: &case.model,
        optional_params: &case.optional_params,
        stream: case.stream.unwrap_or(false),
        environment: InvokeEnvironment {
            region_name: settings.aws_region_name.as_deref(),
            region: settings.aws_region.as_deref(),
            session_region: settings.session_region.as_deref(),
            runtime_endpoint: settings.aws_bedrock_runtime_endpoint.as_deref(),
        },
    });
    Ok(match result {
        Ok(url) => json!({
            "endpoint_url": url.endpoint_url,
            "model_id": url.model_id,
            "provider": url.provider.map(BedrockInvokeProvider::name),
            "region": url.region,
        }),
        Err(_) => json!({"error": true}),
    })
}

fn load(environment: &BTreeMap<String, String>) -> Result<Config, Box<HelperError>> {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load("{}")
    .map_err(|error| failure("ConfigError", &error))
}

fn guardrail_cases(values: &[Box<RawValue>]) -> Value {
    let mut results = Vec::with_capacity(values.len());
    for raw in values {
        results.push(guardrail_result(raw));
    }
    json!({"results": results})
}

fn guardrail_result(raw: &RawValue) -> Value {
    match guardrail_headers(raw) {
        Ok(headers) => json!({"headers": headers.into_iter().collect::<BTreeMap<_, _>>()}),
        Err(error) => json!({"status": error.status_code()}),
    }
}

fn metadata_merge(merges: &[MergeCase], projection: &ErrorCase) -> HelperResult {
    let mut outcomes = Vec::with_capacity(merges.len());
    for case in merges {
        outcomes.push(metadata_outcome(case));
    }
    let supplied: Vec<(String, String)> = projection
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let projected = error_class(projection.message.clone(), projection.status_code, supplied)
        .map_err(|error| failure("BedrockError", &error))?;
    let headers = projected
        .error
        .headers
        .map(|pairs| pairs.into_iter().collect::<BTreeMap<_, _>>());
    Ok(json!({
        "merges": outcomes,
        "error": {
            "status": projected.error.status_code,
            "message": projected.error.message,
            "headers": headers,
        },
    }))
}

fn metadata_outcome(case: &MergeCase) -> Value {
    let headers: IndexMap<String, String> = case
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let mut optional = case.optional_params.clone();
    let params = policy_params(case);
    let owner = (!case.policy_fields.is_empty()).then_some(BedrockRequestPolicy {
        fields: case.policy_fields.iter().cloned().collect(),
    });
    validate_environment(&headers, &mut optional, &params, owner.as_ref()).map_or_else(
        |_| json!({"error": true}),
        |merged| json!({"headers": merged.into_iter().collect::<BTreeMap<_, _>>()}),
    )
}

fn policy_params(case: &MergeCase) -> PolicyParams {
    let mut fields: PolicyParams = case
        .identity
        .iter()
        .map(|(key, value)| (key.clone(), RequestValue::Text(value.clone())))
        .collect();
    if !case.spend_logs_metadata.is_empty() {
        let pairs: PolicyParams = case
            .spend_logs_metadata
            .iter()
            .map(|(key, value)| (key.clone(), RequestValue::Text(value.clone())))
            .collect();
        fields.insert(
            "spend_logs_metadata".to_owned(),
            RequestValue::Mapping(pairs),
        );
    }
    IndexMap::from([("metadata".to_owned(), RequestValue::Mapping(fields))])
}

fn signing(case: &SigningCase) -> HelperResult {
    let headers: Vec<(String, Option<String>)> = case
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let fixture = Credentials {
        access_key_id: case.credentials.access_key_id.clone(),
        secret_access_key: case.credentials.secret_access_key.clone(),
        session_token: case.credentials.session_token.clone(),
    };
    let request = SignRequest {
        headers: &headers,
        optional_params: &case.optional_params,
        request_data: &case.request_data,
        api_base: &case.api_base,
        api_key: case.api_key.as_ref(),
        configured_bearer: case.configured_bearer_token.as_ref(),
        model: case.model.as_deref(),
        stream: case.stream,
        fake_stream: case.fake_stream,
        region_inputs: RegionInputs::default(),
        now: epoch(case.now)?,
    };
    let signed = runtime()?
        .block_on(sign_request(request, move |_settings, _region| {
            ready(Ok::<Credentials, CredentialError>(fixture))
        }))
        .map_err(|error| failure("SigningError", &error))?;
    Ok(json!({
        "method": signed.method,
        "url": signed.url,
        "headers": signed.headers.into_iter().collect::<BTreeMap<_, _>>(),
        "body": STANDARD.encode(&signed.body),
    }))
}

fn prompts(cases: &[PromptCase], local_assets: bool) -> HelperResult {
    let prompts = template_factory(local_assets);
    let runtime = runtime()?;
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let conversion = PromptConversion {
            model: &case.model,
            messages: &case.messages,
            provider: case.provider,
            custom_prompt_dict: &case.custom_prompt_dict,
            prompts: &prompts,
            now: clock()?,
        };
        results.push(
            match runtime.block_on(convert_messages_to_prompt(conversion)) {
                Ok(converted) => {
                    json!({"prompt": converted.prompt, "chat_history": converted.chat_history})
                }
                Err(error) => json!({"message": error.to_string()}),
            },
        );
    }
    Ok(json!({"results": results}))
}
