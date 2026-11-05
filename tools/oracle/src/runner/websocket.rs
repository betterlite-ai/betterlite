use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::runtime::{Builder, Runtime};

use crate::compare;
use crate::gateway::{GatewayInput, start as start_gateway};
use crate::websocket_driver::{
    DriverError, TokioClock, TranscriptObservation, compare_transcript,
    gateway::{TokioTransport, exchange},
    provider::{ProviderHandle, ProviderObservation, start},
    service::{ServiceInput, ServiceTag},
};
use crate::websocket_oracle::{WebSocketBounds, WebSocketCase, output_matches, upstream_matches};
use crate::{
    OracleError,
    case::{
        HelperCase,
        helper::{HelperCompare, HelperKind},
    },
    cli::Options,
    helpers::{self, HelperCall},
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
    let (left, right) = helpers::run_pair(&helper, options.self_check)?;
    let fixture = output_matches(case, &left).and_then(|()| output_matches(case, &right));
    let pair = compare::compare_helper(&left, &right, &[]).map(|difference| difference.pointer);
    let Some(pointer) = fixture.err().or(pair) else {
        writeln!(io::stdout().lock(), "PASS {}", case.name).map_err(OracleError::Output)?;
        return Ok(false);
    };
    writeln!(io::stderr().lock(), "FAIL {}: {pointer}", case.name).map_err(OracleError::Output)?;
    Ok(true)
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
    let Some(Err(pointer)) = upstream.map(|record| upstream_matches(case, &record)) else {
        return Ok(false);
    };
    writeln!(io::stderr().lock(), "FAIL {}: {pointer}", case.name).map_err(OracleError::Output)?;
    Ok(true)
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
