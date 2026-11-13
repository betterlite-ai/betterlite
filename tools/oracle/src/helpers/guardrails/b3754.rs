use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::tls::VerifySetting;
use betterlite_guardrails::cato::error::CatoError;
use betterlite_guardrails::cato::{
    CallHook, CatoGuardrail, CatoParams, SUPPORTED_EVENT_HOOKS, WsSsl,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, ProviderCall, redact};

pub(crate) struct Op;

#[derive(Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Settings {
    api_key: Option<String>,
    api_base: Option<String>,
    inspect_embeddings: Option<bool>,
    ssl_verify: Option<VerifySetting>,
}

#[derive(serde::Deserialize)]
pub(crate) struct Construct {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HookName {
    #[serde(rename = "pre_call")]
    PreCall,
    #[serde(rename = "moderation")]
    Moderation,
    #[serde(rename = "output")]
    Output,
}

#[derive(serde::Deserialize)]
pub(crate) struct Analyze {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    data: BTreeMap<String, GuardrailValue>,
    hook: HookName,
    #[serde(default)]
    key_alias: Option<String>,
    #[serde(default)]
    user_email: Option<String>,
    #[serde(default)]
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Construct(Construct),
    Analyze(Box<Analyze>),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Construct(construct) => construct_answer(construct),
            Input::Analyze(analyze) => analyze_answer(analyze),
        }
    }
}

fn build(
    settings: &Settings,
    environment: &BTreeMap<String, String>,
) -> Result<CatoGuardrail, Box<HelperError>> {
    let params = CatoParams {
        guardrail_name: Some("cato".to_owned()),
        api_key: settings.api_key.clone().map(SecretString::from),
        api_base: settings.api_base.clone(),
        inspect_embeddings: settings.inspect_embeddings,
        ssl_verify: settings.ssl_verify.clone(),
        timeout: None,
    };
    CatoGuardrail::new(params, environment).map_err(|error| helper_error(&error))
}

fn helper_error(error: &CatoError) -> Box<HelperError> {
    let class = match error {
        CatoError::MissingApiKey => "CatoNetworksGuardrailMissingSecrets",
        _ => "CatoError",
    };
    fail(class, error.to_string())
}

fn construct_answer(construct: &Construct) -> HelperResult {
    let guardrail = build(&construct.settings, &construct.environment)?;
    Ok(json!({
        "api_key": guardrail.api_key().expose_secret(),
        "api_base": guardrail.api_base,
        "ws_api_base": guardrail.ws_api_base,
        "inspect_embeddings": guardrail.inspect_embeddings,
        "supported_event_hooks": SUPPORTED_EVENT_HOOKS,
        "ws_ssl": match &guardrail.ws_ssl {
            None => Value::Null,
            Some(WsSsl::Disabled) => json!("disabled"),
            Some(WsSsl::SystemRoots) => json!("system_roots"),
            Some(WsSsl::CaFile(path)) => json!({"ca_file": path}),
        },
    }))
}

fn analyze_answer(analyze: &Analyze) -> HelperResult {
    let io = |error: std::io::Error| fail("OSError", error.to_string());
    // with no answer the listener stays bound and silent, so the call fails instead of reaching Cato
    let silent = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(io)?;
    let mock = analyze
        .answer
        .as_ref()
        .map(|reply| {
            MockScript::try_from(std::slice::from_ref(reply)).and_then(MockProvider::start)
        })
        .transpose()
        .map_err(|error| fail("MockError", error.to_string()))?;
    let base = match &mock {
        Some(provider) => provider.base_url.clone(),
        None => format!("http://{}", silent.local_addr().map_err(io)?),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(io)?;
    let mut guardrail = build(&analyze.settings, &analyze.environment)?;
    guardrail.api_base = base;
    let mut data = analyze.data.clone();
    let hook = match analyze.hook {
        HookName::PreCall => CallHook::PreCall,
        HookName::Moderation => CallHook::Moderation,
        HookName::Output => CallHook::Output,
    };
    let outcome = runtime.block_on(guardrail.call(
        &mut data,
        hook,
        analyze.key_alias.as_deref(),
        analyze.user_email.as_deref(),
    ));
    drop(silent);
    let calls = mock
        .map(MockProvider::finish)
        .transpose()
        .map_err(|error| fail("MockError", error.to_string()))?
        .unwrap_or_default();
    let recorded = calls.first();
    let rejected = match outcome {
        Ok(()) => Value::Null,
        Err(CatoError::Blocked { .. } | CatoError::RedactionMismatch) => json!({"status": 400}),
        Err(error) => return Err(helper_error(error)),
    };
    Ok(json!({
        "path": recorded.map(|call| call.request.path.clone()),
        "headers": recorded.map(|call| headers_summary(&call.request.headers, guardrail)),
        "body": recorded.map(|call| call.request.body.clone()),
        "data": data,
        "rejected": rejected,
    }))
}

fn headers_summary(headers: &BTreeMap<String, String>, guardrail: &CatoGuardrail) -> Value {
    // the mock stores credential headers as a hash, so the expected header goes through the same hash
    let mut expected = BTreeMap::from([(
        "authorization".to_owned(),
        format!("Bearer {}", guardrail.api_key().expose_secret()),
    )]);
    redact(&mut expected);
    json!({
        "authorization_bearer_ok": headers.get("authorization") == expected.get("authorization"),
        "x-cato-litellm-hook": headers.get("x-cato-litellm-hook"),
        "x-cato-litellm-version": headers.get("x-cato-litellm-version"),
        "x-cato-call-id": headers.get("x-cato-call-id"),
        "x-cato-user-email": headers.get("x-cato-user-email"),
        "x-cato-gateway-key-alias": headers.get("x-cato-gateway-key-alias"),
        "content_type": headers.get("content-type"),
    })
}

fn fail(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
