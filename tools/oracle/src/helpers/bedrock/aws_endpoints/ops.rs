use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs;
use std::path::Path;

use betterlite_bedrock::aws_endpoints::model_id::bedrock_model_id;
use betterlite_bedrock::aws_endpoints::provider::{
    BedrockEmbeddingProvider, BedrockInvokeProvider, embedding_provider, invoke_provider,
};
use betterlite_bedrock::aws_endpoints::region::{self, LlmRegion};
use betterlite_bedrock::aws_endpoints::runtime::{
    RuntimeEndpointInput, default_endpoint_url, runtime_endpoint,
};
use betterlite_bedrock::aws_endpoints::sts::{
    StsSettings, resolve_sts_region, sts_client_kwargs, sts_region_from_endpoint,
};
use betterlite_bedrock::aws_settings::tls::resolve_tls;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::tls::{Verification, VerifySetting};
use betterlite_config::types::Config;
use serde_json::{Value, json};

use super::input::{
    BedrockModelIdCase, DefaultEndpointCase, Input, LlmRegionCase, NonLlmRegionCase,
    RuntimeEndpointCase, StsKwargsCase,
};
use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, serde::Serialize)]
struct KwargsJson {
    verify: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    endpoint_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    region_name: Option<String>,
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "AwsEndpointError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn load(environment: &BTreeMap<String, String>) -> Result<Config, Box<HelperError>> {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load("{}")
    .map_err(failure)
}

fn model_arn_region(models: &[Option<String>]) -> Value {
    let regions: Vec<Option<String>> = models
        .iter()
        .map(|model| region::region_from_model_arn(model.as_deref()))
        .collect();
    json!({"regions": regions})
}

fn load_case(case: &LlmRegionCase) -> Result<Config, Box<HelperError>> {
    if case.aws_config.is_none() && case.aws_credentials.is_none() {
        return load(&case.environment);
    }
    let folder = tempfile::tempdir().map_err(failure)?;
    let mut environment = case.environment.clone();
    let home = folder
        .path()
        .to_str()
        .ok_or_else(|| failure("fixture path is not UTF-8"))?;
    environment.insert("HOME".to_owned(), home.to_owned());
    let fixtures = [
        ("config", "AWS_CONFIG_FILE", case.aws_config.as_deref()),
        (
            "credentials",
            "AWS_SHARED_CREDENTIALS_FILE",
            case.aws_credentials.as_deref(),
        ),
    ];
    for (name, variable, text) in fixtures {
        if let Some(content) = text {
            write_fixture(folder.path(), name, variable, content, &mut environment)?;
        }
    }
    load(&environment)
}

fn write_fixture(
    folder: &Path,
    name: &str,
    variable: &str,
    text: &str,
    environment: &mut BTreeMap<String, String>,
) -> Result<(), Box<HelperError>> {
    let directory = folder.join(".aws");
    fs::create_dir_all(&directory).map_err(failure)?;
    let path = directory.join(name);
    fs::write(&path, text).map_err(failure)?;
    if !environment.contains_key(variable) {
        let path_text = path
            .to_str()
            .ok_or_else(|| failure("fixture path is not UTF-8"))?;
        environment.insert(variable.to_owned(), path_text.to_owned());
    }
    Ok(())
}

fn llm_region_name(case: &LlmRegionCase) -> HelperResult {
    let config = load_case(case)?;
    let settings = &config.sections.bedrock_aws_endpoints;
    let explicit = case
        .optional_params
        .get("aws_region_name")
        .and_then(Option::as_deref);
    let result = region::llm_region_name(LlmRegion {
        explicit,
        model: case.model.as_deref(),
        model_id: case.model_id.as_deref(),
        environment_region_name: settings.aws_region_name.as_deref(),
        environment_region: settings.aws_region.as_deref(),
        session_region: settings.session_region.as_deref(),
    });
    Ok(result.map_or_else(
        |_error| json!({"error": true}),
        |region| json!({"region": region}),
    ))
}

fn llm_region_names(cases: &[LlmRegionCase]) -> HelperResult {
    let mut regions = Vec::with_capacity(cases.len());
    for case in cases {
        regions.push(llm_region_name(case)?);
    }
    Ok(json!({"regions": regions}))
}

fn non_llm_region_names(cases: &[NonLlmRegionCase]) -> HelperResult {
    let mut regions = Vec::with_capacity(cases.len());
    for case in cases {
        let config = load(&case.environment)?;
        let settings = &config.sections.bedrock_aws_endpoints;
        let result = region::non_llm_region_name(
            case.explicit.as_deref(),
            settings.aws_region_name.as_deref(),
            settings.aws_region.as_deref(),
        );
        regions.push(result.map_or_else(
            |_error| json!({"error": true}),
            |region| json!({"region": region}),
        ));
    }
    Ok(json!({"regions": regions}))
}

fn invalid_region(values: &[Option<String>]) -> Value {
    let valid: Vec<bool> = values
        .iter()
        .map(|value| region::validate_region(value.as_deref()).is_ok())
        .collect();
    json!({"valid": valid})
}

fn bedrock_model_ids(cases: &[BedrockModelIdCase]) -> HelperResult {
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let mut optional_params = case.optional_params.clone();
        match bedrock_model_id(&mut optional_params, case.provider, &case.model) {
            Ok(model_id) => {
                let remaining = serde_json::to_value(&optional_params).map_err(failure)?;
                results.push(json!({"model_id": model_id, "optional_params": remaining}));
            }
            Err(_) => results.push(json!({"error": true})),
        }
    }
    Ok(json!({"results": results}))
}

fn provider_selection(models: &[String]) -> Value {
    let providers: Vec<Value> = models
        .iter()
        .map(|model| {
            json!({
                "invoke": invoke_provider(model).map(BedrockInvokeProvider::name),
                "embedding": embedding_provider(model).map(BedrockEmbeddingProvider::name),
            })
        })
        .collect();
    json!({"providers": providers})
}

fn default_endpoints(cases: &[DefaultEndpointCase]) -> Value {
    let urls: Vec<String> = cases
        .iter()
        .map(|case| default_endpoint_url(case.endpoint_type.unwrap_or_default(), &case.region))
        .collect();
    json!({"urls": urls})
}

fn runtime_endpoint_cases(cases: &[RuntimeEndpointCase]) -> HelperResult {
    let mut endpoints = Vec::with_capacity(cases.len());
    for case in cases {
        let config = load(&case.environment)?;
        let settings = &config.sections.bedrock_aws_endpoints;
        let result = runtime_endpoint(RuntimeEndpointInput {
            api_base: case.api_base.as_deref(),
            configured_endpoint: case.aws_bedrock_runtime_endpoint.as_deref(),
            environment_endpoint: settings.aws_bedrock_runtime_endpoint.as_deref(),
            region: &case.region,
            endpoint_type: case.endpoint_type.unwrap_or_default(),
        });
        endpoints.push(json!([result.endpoint_url, result.proxy_endpoint_url]));
    }
    Ok(json!({"endpoints": endpoints}))
}

fn sts_endpoint_regions(endpoints: &[Option<String>]) -> Value {
    let regions: Vec<Value> = endpoints
        .iter()
        .map(|endpoint| {
            sts_region_from_endpoint(endpoint.as_deref()).map_or_else(
                |_error| json!({"error": true}),
                |region| json!({"region": region}),
            )
        })
        .collect();
    json!({"regions": regions})
}

fn verification_json(verification: Verification) -> Value {
    match verification {
        Verification::Enabled => json!(true),
        Verification::Disabled => json!(false),
        Verification::CaFile(path) => Value::String(path),
    }
}

fn sts_kwarg_cases(cases: &[StsKwargsCase]) -> HelperResult {
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let config = load(&case.environment)?;
        let settings = &config.sections.bedrock_aws_endpoints;
        let explicit = case.verify.map(VerifySetting::Bool);
        let verification = resolve_tls(&config, &case.environment, explicit.as_ref())
            .map_err(failure)?
            .verification;
        let sts = StsSettings {
            endpoint: case.endpoint.as_deref(),
            aws_region: settings.aws_region.as_deref(),
            aws_default_region: settings.aws_default_region.as_deref(),
            configured_region: case.configured_region.as_deref(),
        };
        let region = resolve_sts_region(&sts).map_err(failure)?;
        let kwargs = sts_client_kwargs(&sts, verification).map_err(failure)?;
        results.push(json!({
            "region": region,
            "kwargs": KwargsJson {
                verify: verification_json(kwargs.verify),
                endpoint_url: kwargs.endpoint_url,
                region_name: kwargs.region_name,
            },
        }));
    }
    Ok(json!({"results": results}))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::ModelArnRegion { models } => Ok(model_arn_region(models)),
        Input::LlmRegionName { cases } => llm_region_names(cases),
        Input::NonLlmRegionName { cases } => non_llm_region_names(cases),
        Input::InvalidRegion { values } => Ok(invalid_region(values)),
        Input::BedrockModelId { cases } => bedrock_model_ids(cases),
        Input::ProviderSelection { models } => Ok(provider_selection(models)),
        Input::DefaultEndpoint { cases } => Ok(default_endpoints(cases)),
        Input::RuntimeEndpoint { cases } => runtime_endpoint_cases(cases),
        Input::StsEndpointRegion { endpoints } => Ok(sts_endpoint_regions(endpoints)),
        Input::StsKwargs { cases } => sts_kwarg_cases(cases),
    }
}
