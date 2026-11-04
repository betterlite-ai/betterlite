use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::runtime::{Builder, Runtime};

use crate::compare::helper::HelperOutput;
use crate::gateway::{GatewayInput, start as start_gateway};
use crate::websocket_driver::{
    DriverError, TokioClock, TranscriptObservation, compare_transcript,
    gateway::{TokioTransport, exchange},
    provider::{ProviderHandle, ProviderObservation, start},
    service::{ServiceInput, ServiceTag},
};
use crate::websocket_oracle::{
    AuthObservation, RouteObservation, WebSocketBounds, WebSocketCase, close_matches,
    direction_matches, route_auth_matches,
};
use crate::{
    OracleError,
    case::{
        HelperCase,
        helper::{HelperCompare, HelperKind},
    },
    cli::Options,
    helpers::{self, HelperCall, HelperRun},
    paths::safe_name,
};

pub(super) fn run_cases(
    options: &Options,
    root: &Path,
    cases: &[WebSocketCase],
) -> Result<Vec<bool>, OracleError> {
    let mut outcomes = Vec::new();
    for case in cases {
        match case.service {
            Some(service) => outcomes.push(run_service_case(options, case, service)?),
            None => outcomes.extend(run_gateway_case(options, root, case)?),
        }
    }
    Ok(outcomes)
}

fn run_service_case(
    options: &Options,
    case: &WebSocketCase,
    service: ServiceTag,
) -> Result<bool, OracleError> {
    let service_input = ServiceInput {
        service,
        input: case.input.clone(),
    };
    let helper = HelperCase {
        kind: HelperKind::Helper,
        name: case.name.clone(),
        env: case.env.clone(),
        compare: HelperCompare::default(),
        call: HelperCall {
            operation: "websocket_oracle".to_owned(),
            input: serde_json::to_value(service_input.to_wire()).map_err(OracleError::Json)?,
        },
    };
    verify_native_against_fixture(case, service, &helpers::invoke(&helper.call))?;
    Ok(matches!(
        helpers::run_case(&helper, options.self_check)?,
        HelperRun::Difference
    ))
}

fn prepare_gateway_group(
    root: &Path,
    case: &WebSocketCase,
) -> Result<(PathBuf, Runtime), OracleError> {
    let group = root.join("websocket").join(safe_name(&case.name));
    let to_error = |source| OracleError::Filesystem {
        path: group.clone(),
        source,
    };
    fs::create_dir_all(&group).map_err(to_error)?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(to_error)?;
    Ok((group, runtime))
}

fn run_gateway_case(
    options: &Options,
    root: &Path,
    case: &WebSocketCase,
) -> Result<[bool; 2], OracleError> {
    let (group, runtime) = prepare_gateway_group(root, case)?;
    let both_directions = [true, false];
    let mut outcomes = [false, false];
    for (slot, first) in outcomes.iter_mut().zip(both_directions) {
        *slot = attempt(options, &group, case, &runtime, first)?;
    }
    Ok(outcomes)
}

fn scenario_invalid(case: &WebSocketCase, detail: &str) -> OracleError {
    OracleError::ScenarioInvalid {
        message: format!("{}: {detail}", case.name),
    }
}

fn verify_native_against_fixture(
    case: &WebSocketCase,
    service: ServiceTag,
    output: &HelperOutput,
) -> Result<(), OracleError> {
    if output.ok {
        let Some(value) = &output.value else {
            return Ok(());
        };
        let observed: TranscriptObservation =
            serde_json::from_value(value.clone()).map_err(OracleError::Json)?;
        return compare_transcript(case, &observed)
            .map_err(|pointer| scenario_invalid(case, &pointer));
    }
    let expected_missing = DriverError::AdapterMissing(service).to_string();
    let is_adapter_missing = output
        .error
        .as_ref()
        .is_some_and(|error| error.class == "ServiceError" && error.message == expected_missing);
    if is_adapter_missing {
        return Ok(());
    }
    Err(scenario_invalid(
        case,
        &format!("native service adapter failed: {:?}", output.error),
    ))
}

fn config_path(
    root: &Path,
    case: &WebSocketCase,
    provider: &ProviderHandle,
) -> Result<PathBuf, OracleError> {
    let template = if case.config.is_empty() {
        include_str!("../../fixtures/websocket-oracle/default.yaml")
    } else {
        &case.config
    };
    let config = template.replace(
        "__WEBSOCKET_PROVIDER__",
        &format!("ws://{}", provider.address),
    );
    let path = root.join("config.yaml");
    fs::write(&path, config).map_err(|source| OracleError::Filesystem {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

fn attempt(
    options: &Options,
    root: &Path,
    case: &WebSocketCase,
    runtime: &Runtime,
    first: bool,
) -> Result<bool, OracleError> {
    let bounds = WebSocketBounds::default();
    let provider = runtime.block_on(start(
        case.clone(),
        bounds.limits,
        bounds.settings,
        &TokioClock,
    ))?;
    let path = config_path(root, case, &provider)?;
    let input = GatewayInput {
        options,
        group_dir: root,
        config_path: &path,
        case_env: &case.env,
    };
    let mut gateway = start_gateway(&input, first)?;
    let result = runtime.block_on(capture(case, provider, &gateway.base_url, &bounds));
    let stopped = gateway.stop();
    let (observed, upstream) = result?;
    stopped?;
    compare_transcript(case, &observed).map_err(|pointer| scenario_invalid(case, &pointer))?;
    if let Some(record) = &upstream {
        route_auth_matches(
            &RouteObservation {
                model: case.route_model.clone(),
                guardrails: case.route_guardrails.clone(),
            },
            &record.route,
            &AuthObservation {
                detail: case.authentication_detail.clone(),
                attempted_close_code: case.attempted_close_code,
            },
            &record.auth,
        )
        .map_err(|pointer| scenario_invalid(case, &pointer))?;
    }
    Ok(upstream.is_some_and(|record| upstream_differs(case, &record)))
}

async fn capture(
    case: &WebSocketCase,
    provider: ProviderHandle,
    base: &str,
    bounds: &WebSocketBounds,
) -> Result<(TranscriptObservation, Option<ProviderObservation>), DriverError> {
    let observed = match exchange(base, case, bounds, &TokioClock, &TokioTransport).await {
        Ok(observation) => observation,
        Err(error) => {
            provider.cancel().await?;
            return Err(error);
        }
    };
    let upstream = if observed.handshake.status == 101 {
        Some(
            provider
                .finish(Duration::from_secs(case.execution_seconds))
                .await?,
        )
    } else {
        let connections = provider.cancel().await?;
        if connections != 0 {
            return Err(DriverError::Transport(
                "preaccept refusal opened an upstream connection".to_owned(),
            ));
        }
        None
    };
    Ok((observed, upstream))
}

fn upstream_differs(case: &WebSocketCase, record: &ProviderObservation) -> bool {
    let query = record
        .request_uri
        .split_once('?')
        .map_or("", |(_, query)| query);
    record.connection_count != 1
        || query != case.expected_upstream_query.as_deref().unwrap_or("")
        || (case.upstream_status.is_none()
            && (direction_matches(&case.frames, &record.frames).is_err()
                || close_matches(case.close.as_ref(), record.close.as_ref()).is_err()))
}
