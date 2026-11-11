use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::future::ready;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::bedrock_settings::ErrorHeaders;
use betterlite_bedrock::claude_platform_inputs::body::{
    PlatformWarning, filter_request_body, resolve_unsupported_override,
};
use betterlite_bedrock::claude_platform_inputs::endpoint::{
    UrlInputs, complete_url, resolve_region,
};
use betterlite_bedrock::claude_platform_inputs::error::{PlatformInputError, error_class};
use betterlite_bedrock::claude_platform_inputs::route::strip_claude_platform_route;
use betterlite_bedrock::claude_platform_inputs::signing::{
    PlatformSignRequest, PlatformSignature, sign_platform_request,
};
use betterlite_bedrock::claude_platform_inputs::workspace::resolve_workspace_id;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use serde_json::{Value, json};

use super::input::{
    BodyCall, ErrorClassCall, Input, OverrideCall, RegionCase, SignCall, SigningCall, UrlCase,
    WorkspaceCase,
};
use crate::helpers::{HelperError, HelperResult};

const FIXTURE_CONFIG: &str =
    "model_list:\n- model_name: unit\n  litellm_params: {model: bedrock/unit}\n";

fn invalid<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn config(environment: &BTreeMap<String, String>) -> Result<Config, Box<HelperError>> {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load(FIXTURE_CONFIG)
    .map_err(invalid)
}

fn config_from(
    source: &str,
    environment: &BTreeMap<String, String>,
) -> Result<Config, Box<HelperError>> {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load(source)
    .map_err(invalid)
}

const fn error_code(error: &PlatformInputError) -> &'static str {
    match error {
        PlatformInputError::MissingRegion => "missing_region",
        PlatformInputError::Region(_) => "invalid_region",
        PlatformInputError::ValueType { .. } | PlatformInputError::Signing(_) => "other",
    }
}

fn warning_json(warning: &PlatformWarning) -> Value {
    match warning {
        PlatformWarning::DroppedUnsupported { params } => {
            json!({"action": "drop", "params": params})
        }
        PlatformWarning::IgnoredOverride { type_name } => {
            json!({"action": "ignore", "type_name": type_name})
        }
    }
}

fn workspace_config(case: &WorkspaceCase) -> Result<Config, Box<HelperError>> {
    let mut params = serde_json::Map::new();
    params.insert("model".to_owned(), Value::String("bedrock/unit".to_owned()));
    for (key, value) in &case.deployment {
        params.insert(key.clone(), Value::String(value.clone()));
    }
    let document = json!({
        "model_list": [{"model_name": "unit", "litellm_params": params}],
        "litellm_settings": case.litellm_settings,
    });
    let source = serde_saphyr::to_string(&document).map_err(invalid)?;
    config_from(&source, &case.environment)
}

fn workspaces(cases: &[WorkspaceCase]) -> HelperResult {
    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        let config = workspace_config(case)?;
        let settings = &config.sections.claude_platform_inputs;
        let deployment = settings.deployments.first().cloned().unwrap_or_default();
        let workspace = resolve_workspace_id(
            &case.optional_params,
            &deployment,
            &settings.litellm_settings,
            &settings.environment,
        )
        .map_err(invalid)?;
        outcomes.push(workspace);
    }
    Ok(json!({"workspaces": outcomes}))
}

fn region_outcome(outcome: Result<String, PlatformInputError>) -> Value {
    match outcome {
        Ok(region) => json!({"region": region}),
        Err(error) => json!({"error": error_code(&error)}),
    }
}

fn regions(cases: &[RegionCase]) -> HelperResult {
    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        let config = config(&case.environment)?;
        outcomes.push(region_outcome(resolve_region(
            &case.optional_params,
            &config.sections.bedrock_aws_endpoints,
        )));
    }
    Ok(json!({"regions": outcomes}))
}

fn urls(cases: &[UrlCase]) -> HelperResult {
    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        let config = config(&case.environment)?;
        let outcome = complete_url(
            &UrlInputs {
                api_base: case.api_base.as_deref(),
                litellm_api_base: case.litellm_api_base.as_deref(),
                optional_params: &case.optional_params,
            },
            &config.sections.claude_platform_inputs.environment,
            &config.sections.bedrock_aws_endpoints,
        );
        outcomes.push(match outcome {
            Ok(url) => json!({"url": url}),
            Err(error) => json!({"error": error_code(&error)}),
        });
    }
    Ok(json!({"urls": outcomes}))
}

fn route_models(models: &[String]) -> Value {
    let stripped: Vec<String> = models
        .iter()
        .map(|model| strip_claude_platform_route(model))
        .collect();
    json!({"models": stripped})
}

fn overrides(calls: &[OverrideCall]) -> Value {
    let mut outcomes = Vec::with_capacity(calls.len());
    for call in calls {
        let resolution = resolve_unsupported_override(
            call.deployment.as_ref(),
            call.optional.as_ref(),
            call.log_invalid,
        );
        outcomes.push(json!({
            "names": resolution.names.map(|names| names.into_iter().collect::<Vec<_>>()),
            "warning": resolution.warning.as_ref().map_or(Value::Null, warning_json),
        }));
    }
    json!({"calls": outcomes})
}

fn bodies(calls: &[BodyCall]) -> HelperResult {
    let mut outcomes = Vec::with_capacity(calls.len());
    for call in calls {
        let unsupported: Option<BTreeSet<String>> = call
            .unsupported
            .as_ref()
            .map(|names| names.iter().cloned().collect());
        let filtered = filter_request_body(&call.params, unsupported.as_ref(), call.log_dropped);
        let warnings: Vec<Value> = filtered.warning.iter().map(warning_json).collect();
        outcomes.push(json!({
            "body": serde_json::to_value(&filtered.body).map_err(invalid)?,
            "warnings": warnings,
        }));
    }
    Ok(json!({"calls": outcomes}))
}

fn epoch(seconds: i64) -> Result<SystemTime, Box<HelperError>> {
    let bounded = u64::try_from(seconds).map_err(invalid)?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(bounded))
        .ok_or_else(|| invalid("now is out of range"))
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(invalid)
}

fn sign_call(call: &SignCall) -> HelperResult {
    let config = config(&call.environment)?;
    let platform = &config.sections.claude_platform_inputs;
    let bearer = config
        .sections
        .bedrock_settings
        .deployments
        .first()
        .and_then(|deployment| deployment.bearer_token.clone());
    let headers: Vec<(String, Option<String>)> = call
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let request = PlatformSignRequest {
        headers: &headers,
        optional_params: &call.optional_params,
        request_data: &call.request_data,
        api_base: &call.api_base,
        api_key: call.api_key.as_ref(),
        environment_api_key: platform.environment.anthropic_aws_api_key.as_ref(),
        configured_bearer: bearer.as_ref(),
        model: call.model.as_deref(),
        region_inputs: RegionInputs::default(),
        now: epoch(call.now)?,
    };
    let credentials = Credentials {
        access_key_id: call.credentials.access_key_id.clone(),
        secret_access_key: call.credentials.secret_access_key.clone(),
        session_token: call.credentials.session_token.clone(),
    };
    let outcome = runtime()?.block_on(sign_platform_request(request, move |_settings, _region| {
        ready(Ok::<Credentials, CredentialError>(credentials))
    }));
    Ok(signature_json(outcome))
}

fn signature_json(outcome: Result<PlatformSignature, PlatformInputError>) -> Value {
    match outcome {
        Ok(PlatformSignature::Supplied { headers }) => json!({
            "signed": false,
            "headers": headers.iter().cloned().collect::<BTreeMap<_, _>>(),
            "body": Value::Null,
        }),
        Ok(PlatformSignature::Signed(signed)) => json!({
            "signed": true,
            "method": signed.method,
            "url": signed.url,
            "headers": signed.headers.iter().cloned().collect::<BTreeMap<_, _>>(),
            "body": STANDARD.encode(&signed.body),
        }),
        Err(error) => json!({"error": true, "status": error.status_code()}),
    }
}

fn error_class_call(call: &ErrorClassCall) -> Value {
    match error_class(
        call.message.clone(),
        call.status_code,
        Some(ErrorHeaders::Http(call.headers.clone())),
    ) {
        Ok(error) => json!({"ok": true, "status_code": error.error.status_code}),
        Err(_) => json!({"ok": false}),
    }
}

fn signing(calls: &[SigningCall]) -> HelperResult {
    let mut outcomes = Vec::with_capacity(calls.len());
    for call in calls {
        outcomes.push(match call {
            SigningCall::Sign(sign) => sign_call(sign)?,
            SigningCall::ErrorClass(error) => error_class_call(error),
        });
    }
    Ok(json!({"calls": outcomes}))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::WorkspaceAliasPrecedence { cases } => workspaces(cases),
        Input::ExplicitRegionRequired { cases } => regions(cases),
        Input::ApiBaseBeforeRegion { cases } => urls(cases),
        Input::PlatformRoutePrefix { models } => Ok(route_models(models)),
        Input::UnsupportedOverride { calls } => Ok(overrides(calls)),
        Input::FilteredBodyFields { calls } => bodies(calls),
        Input::PlatformSigning { calls } => signing(calls),
    }
}
