use std::net::TcpListener;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::content_filter::risk_scoring::GuardrailStatus;
use betterlite_guardrails::ibm_detector_client::client::IbmDetectorClient;
use betterlite_guardrails::ibm_detector_client::client::error::IbmClientError;
use betterlite_guardrails::ibm_detector_client::status::{
    detector_server_status, orchestrator_status,
};
use betterlite_guardrails::ibm_detector_client::wire::{Detection, ServerDetections};
use serde_json::json;

use super::request::request_values;
use super::{Environment, Settings, configure, failure};
use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

const MOCK_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn detect(
    settings: &Settings,
    environment: &Environment,
    contents: &[String],
    answer: Option<&UpstreamAnswer>,
) -> HelperResult {
    let (mock, silent, base) = mock_for(answer, contents.len())?;
    let mut parameters = settings.clone();
    parameters.insert("base_url".to_owned(), GuardrailValue::Text(base));
    let config = configure(&parameters, environment)?;
    let mut client = IbmDetectorClient::new(config)
        .map_err(|error| failure("IbmDetectorClientError", error.to_string()))?;
    client.timeout = MOCK_TIMEOUT;
    let runtime = runtime()?;
    let (status, detections) = if client.config.is_detector_server {
        let outcome: Result<ServerDetections, IbmClientError> =
            runtime.block_on(client.call_detector_server(contents));
        let status = detector_server_status(&client.config, &outcome);
        (status, outcome.ok())
    } else {
        scan_orchestrator(&client, &runtime, contents)
    };
    let recorded = finish_mock(mock)?;
    let requests = request_values(&recorded, &client.config);
    drop(silent);
    Ok(json!({"requests": requests, "detections": detections, "status": status}))
}

fn mock_for(
    answer: Option<&UpstreamAnswer>,
    calls: usize,
) -> Result<(Option<MockProvider>, TcpListener, String), Box<HelperError>> {
    let io = |error: std::io::Error| failure("OSError", error.to_string());
    // with no answer this listener stays bound and silent while the caller holds it, so the call times out
    let silent = TcpListener::bind(("127.0.0.1", 0)).map_err(io)?;
    // the orchestrator makes one call per text, and MockProvider serves one scripted reply per call
    let mock = answer
        .map(|reply| {
            let answers = vec![reply.clone(); calls.max(1)];
            MockScript::try_from(answers.as_slice()).and_then(MockProvider::start)
        })
        .transpose()
        .map_err(|error| failure("MockError", error.to_string()))?;
    let base = match &mock {
        Some(provider) => provider.base_url.clone(),
        None => format!("http://{}", silent.local_addr().map_err(io)?),
    };
    Ok((mock, silent, base))
}

fn finish_mock(
    mock: Option<MockProvider>,
) -> Result<Vec<crate::routing_mock_scripts::ProviderCall>, Box<HelperError>> {
    mock.map(MockProvider::finish)
        .transpose()
        .map_err(|error| failure("MockError", error.to_string()))
        .map(Option::unwrap_or_default)
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("OSError", error.to_string()))
}

fn scan_orchestrator(
    client: &IbmDetectorClient,
    runtime: &tokio::runtime::Runtime,
    contents: &[String],
) -> (GuardrailStatus, Option<Vec<Vec<Detection>>>) {
    let mut results = Vec::with_capacity(contents.len());
    let mut seen = Vec::with_capacity(contents.len());
    for content in contents {
        let outcome = runtime.block_on(client.call_orchestrator(content));
        seen.push(orchestrator_status(&client.config, &outcome));
        match outcome {
            Ok(detections) => results.push(detections),
            Err(_) => return (GuardrailStatus::GuardrailFailedToRespond, None),
        }
    }
    let status = if seen.contains(&GuardrailStatus::GuardrailIntervened) {
        GuardrailStatus::GuardrailIntervened
    } else {
        GuardrailStatus::Success
    };
    (status, Some(results))
}
