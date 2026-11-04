use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::OracleError;
use crate::cli::Options;
use crate::gateway::{GatewayInput, start as start_gateway};
use crate::websocket_driver::{compare_transcript, exchange_gateway, start_provider};
use crate::websocket_oracle::{MAX_EXECUTION_SECONDS, WebSocketCase};

struct WebSocketGatewayAttempt<'input> {
    options: &'input Options,
    root: &'input Path,
    case: &'input WebSocketCase,
    runtime: &'input tokio::runtime::Runtime,
    first: bool,
}

pub(super) fn run_websocket_gateway_case(
    options: &Options,
    root: &Path,
    case: &WebSocketCase,
) -> Result<Vec<bool>, OracleError> {
    let group_dir = root
        .join("websocket")
        .join(crate::paths::safe_name(&case.name));
    fs::create_dir_all(&group_dir).map_err(|source| OracleError::Filesystem {
        path: group_dir.clone(),
        source,
    })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| OracleError::ScenarioInvalid {
            message: source.to_string(),
        })?;
    let first = run_websocket_gateway_attempt(WebSocketGatewayAttempt {
        options,
        root,
        case,
        runtime: &runtime,
        first: true,
    })?;
    let second = run_websocket_gateway_attempt(WebSocketGatewayAttempt {
        options,
        root,
        case,
        runtime: &runtime,
        first: false,
    })?;
    Ok(vec![first, second])
}

fn run_websocket_gateway_attempt(
    attempt: WebSocketGatewayAttempt<'_>,
) -> Result<bool, OracleError> {
    let WebSocketGatewayAttempt {
        options,
        root,
        case,
        runtime,
        first,
    } = attempt;
    let provider = runtime
        .block_on(start_provider(case.clone()))
        .map_err(|error| OracleError::ScenarioInvalid {
            message: error.to_string(),
        })?;
    let provider_url = format!("ws://{}", provider.address);
    let default_config = "model_list:\n  - model_name: realtime-a\n    litellm_params:\n      model: openai/realtime-a\n      api_base: __WEBSOCKET_PROVIDER__\n      api_key: sk-provider\n  - model_name: gpt-realtime-whisper\n    litellm_params:\n      model: openai/gpt-realtime-whisper\n      api_base: __WEBSOCKET_PROVIDER__\n      api_key: sk-provider\n";
    let template = if case.config.is_empty() {
        default_config
    } else {
        &case.config
    };
    let config = template.replace("__WEBSOCKET_PROVIDER__", &provider_url);
    let config_path = root.join("websocket").join(format!(
        "{}-{}.yaml",
        crate::paths::safe_name(&case.name),
        first
    ));
    if let Err(source) = fs::write(&config_path, config) {
        let _cancelled = runtime.block_on(provider.cancel());
        return Err(OracleError::Filesystem {
            path: config_path,
            source,
        });
    }
    let input = GatewayInput {
        options,
        group_dir: root,
        config_path: &config_path,
        case_env: &case.env,
    };
    let mut gateway = match start_gateway(&input, first) {
        Ok(gateway) => gateway,
        Err(error) => {
            let _cancelled = runtime.block_on(provider.cancel());
            return Err(error);
        }
    };
    let captured = runtime.block_on(exchange_gateway(&gateway.base_url, case));
    let stop_result = gateway.stop();
    let observed = match captured {
        Ok(observed) => observed,
        Err(error) => {
            let _cancelled = runtime.block_on(provider.cancel());
            stop_result?;
            return Err(OracleError::ScenarioInvalid {
                message: error.to_string(),
            });
        }
    };
    stop_result?;
    let provider_observation = if observed.handshake.status == 101 {
        Some(
            runtime
                .block_on(provider.finish(Duration::from_secs(MAX_EXECUTION_SECONDS)))
                .map_err(|error| OracleError::ScenarioInvalid {
                    message: error.to_string(),
                })?,
        )
    } else {
        runtime
            .block_on(provider.cancel())
            .map_err(|error| OracleError::ScenarioInvalid {
                message: error.to_string(),
            })?;
        None
    };
    let mut difference = compare_transcript(case, &observed).is_err();
    if let Some(provider_observation) = provider_observation {
        difference |= provider_observation.connection_count != 1;
        difference |= crate::websocket_oracle::close_matches(
            case.close.as_ref(),
            provider_observation.close.as_ref(),
        )
        .is_err();
        difference |= provider_observation
            .request_uri
            .split_once('?')
            .map(|(_, query)| query)
            .unwrap_or("")
            != case.expected_upstream_query.as_deref().unwrap_or("");
        difference |=
            crate::websocket_oracle::direction_matches(&case.frames, &provider_observation.frames)
                .is_err();
    }
    Ok(difference)
}
