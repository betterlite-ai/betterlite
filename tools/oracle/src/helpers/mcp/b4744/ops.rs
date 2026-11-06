use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_mcp::client::sessions::client::SessionSettings;
use betterlite_mcp::client::sessions::environment::safe_stdio_env;
use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::client::sessions::factory::{HttpClientConfig, SseRequest};
use betterlite_mcp::client::sessions::negotiate::{Callbacks, ProtocolVersion, initialize_session};
use betterlite_mcp::client::sessions::transport::{StdioServerParameters, TransportType};
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::helpers::{HelperError, HelperResult};

mod client;
use client::{make_client, tls_policy};

use super::fixture::{Fixture, InitStep, Plan, Script, Step};
use super::input::{
    AuthSourceInput, FactoryInput, InitializeInput, SettingsInput, StdioInput, TlsInput,
    TransportInput,
};

mod execute;
pub(crate) use execute::execute;

pub(crate) fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn config_json(config: &HttpClientConfig) -> Value {
    json!({
        "headers": config.headers,
        "timeout_ms": u64::try_from(config.timeout.as_millis()).unwrap_or(u64::MAX),
        "tls": config.tls,
        "auth": config.auth,
        "follow_redirects": config.follow_redirects,
        "redirect_guard": config.redirect_guard,
        "capture_error_responses": config.capture_error_responses,
    })
}

enum Recorded {
    Stdio(StdioServerParameters),
    Sse(SseRequest),
    Http(HttpClientConfig),
}

fn record_json(recorded: &Recorded, http_client: bool) -> Value {
    match recorded {
        Recorded::Stdio(parameters) => json!({
            "kind": "stdio",
            "http_client": http_client,
            "stdio": {
                "command": parameters.command,
                "args": parameters.args,
                "env": parameters.env,
            },
            "sse": null,
            "http": null,
            "error": null,
        }),
        Recorded::Sse(request) => json!({
            "kind": "sse",
            "http_client": http_client,
            "stdio": null,
            "sse": {
                "url": request.url,
                "timeout_ms": u64::try_from(request.timeout.as_millis()).unwrap_or(u64::MAX),
                "headers": request.headers,
                "client": config_json(&request.client),
            },
            "http": null,
            "error": null,
        }),
        Recorded::Http(config) => json!({
            "kind": "http",
            "http_client": http_client,
            "stdio": null,
            "sse": null,
            "http": config_json(config),
            "error": null,
        }),
    }
}

fn recorded_transport(script: &Script, http_client: bool) -> Option<Value> {
    let stdio = script
        .stdio
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let Some(parameters) = stdio {
        return Some(record_json(&Recorded::Stdio(parameters), http_client));
    }
    let sse = script
        .sse
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let Some(request) = sse {
        return Some(record_json(&Recorded::Sse(request), http_client));
    }
    let http = script
        .http
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .first()
        .cloned();
    http.map(|config| record_json(&Recorded::Http(config), http_client))
}

pub(crate) fn transport(input: &TransportInput) -> HelperResult {
    let fixture = Fixture::new(Plan::default());
    let client = make_client(
        &input.settings,
        fixture,
        TaskTracker::new(),
        CancellationToken::new(),
    )?;
    let script = Arc::clone(&client.runtime.script);
    match client.create_transport_context() {
        Ok(contexts) => {
            let http_client = contexts.http_client.is_some();
            Ok(recorded_transport(&script, http_client).unwrap_or_else(|| {
                json!({
                    "kind": input.settings.transport_type,
                    "http_client": http_client,
                    "stdio": null,
                    "sse": null,
                    "http": null,
                    "error": null,
                })
            }))
        }
        Err(SessionError::Configuration(_)) => Ok(json!({
            "kind": input.settings.transport_type,
            "http_client": false,
            "stdio": null,
            "sse": null,
            "http": null,
            "error": "missing_stdio_config",
        })),
        Err(error) => Err(invalid(error.to_string())),
    }
}

pub(crate) fn stdio_env(input: &StdioInput) -> Value {
    json!({
        "env": safe_stdio_env(input.provided.as_ref(), &input.environment, &input.npm_cache_dir),
    })
}

fn initialize_settings(input: &InitializeInput) -> Result<SessionSettings, Box<HelperError>> {
    let protocol_version = ProtocolVersion::try_from(input.protocol_version.clone())
        .map_err(|error| invalid(error.to_string()))?;
    Ok(SessionSettings {
        transport_type: TransportType::Http,
        stdio: None,
        environment: BTreeMap::new(),
        npm_cache_dir: String::new(),
        tls: tls_policy(&TlsInput::default()),
        protocol_version,
        client_version: input.client_version.clone(),
        callbacks: Callbacks {
            kinds: input.callbacks.iter().copied().collect(),
        },
    })
}

pub(crate) fn initialize(input: &InitializeInput) -> HelperResult {
    let settings = initialize_settings(input)?;
    let plan = Plan {
        transport_enter: Step::Ok,
        transport_exit: Step::Ok,
        session_enter: Step::Ok,
        session_exit: Step::Ok,
        initialize: InitStep::Ok,
        server_version: input.server_version.clone(),
        instructions: None,
        http_close: Step::Ok,
    };
    let fixture = Fixture::new(plan);
    let script = Arc::clone(&fixture.script);
    let mut session = fixture.session_client();
    let runtime = Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|error| invalid(error.to_string()))?;
    let outcome = runtime.block_on(initialize_session(&mut session, &settings));
    let (version, error_code) = match outcome {
        Ok(result) => (Some(result.version), None),
        Err(SessionError::Protocol(failure)) => (None, Some(failure.code)),
        Err(error) => return Err(invalid(error.to_string())),
    };
    let requests = script
        .requests
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    Ok(json!({
        "calls": script.calls(),
        "request": requests.first(),
        "version": version,
        "error_code": error_code,
    }))
}

fn config_for(settings: &SettingsInput, explicit_auth: bool) -> Result<Value, Box<HelperError>> {
    let fixture = Fixture::new(Plan::default());
    let client = make_client(
        settings,
        fixture,
        TaskTracker::new(),
        CancellationToken::new(),
    )?;
    let timeout = Duration::from_millis(settings.timeout_ms);
    let config = client.create_http_client_config(&BTreeMap::new(), timeout, explicit_auth);
    Ok(config_json(&config))
}

pub(crate) fn factory(input: &FactoryInput) -> HelperResult {
    let fixture = Fixture::new(Plan::default());
    let client = make_client(
        &input.settings,
        fixture,
        TaskTracker::new(),
        CancellationToken::new(),
    )?;
    let timeout = Duration::from_millis(input.settings.timeout_ms);
    let config = client.create_http_client_config(&BTreeMap::new(), timeout, input.explicit_auth);
    let mut variants = Vec::new();
    for variant in &input.auth_variants {
        let mut settings = input.settings.clone();
        settings.auth.resolved_auth = variant.sources.contains(&AuthSourceInput::Resolved);
        settings.auth.aws_auth = variant.sources.contains(&AuthSourceInput::Aws);
        let explicit = variant.sources.contains(&AuthSourceInput::Explicit);
        variants.push(config_for(&settings, explicit)?);
    }
    let mut redirects = Vec::new();
    for check in &input.redirect_checks {
        let mut headers = check.headers.clone();
        if let Some(guard) = config.redirect_guard.as_ref() {
            guard.apply(&check.url, &mut headers);
        }
        redirects.push(json!(headers));
    }
    Ok(json!({
        "config": config_json(&config),
        "variants": variants,
        "redirects": redirects,
    }))
}
