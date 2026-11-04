use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::runtime::{Builder, Runtime};

use crate::gateway::{GatewayInput, start as start_gateway};
use crate::websocket_driver::{
    DriverError, TranscriptObservation, compare_transcript,
    gateway::exchange,
    provider::{ProviderHandle, ProviderObservation, start},
    service::ServiceInput,
};
use crate::websocket_oracle::{WebSocketCase, close_matches, direction_matches};
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
        if let Some(service) = case.service {
            let helper = HelperCase {
                kind: HelperKind::Helper,
                name: case.name.clone(),
                env: case.env.clone(),
                compare: HelperCompare::default(),
                call: HelperCall {
                    operation: "websocket_oracle".to_owned(),
                    input: serde_json::to_value(ServiceInput {
                        service,
                        input: case.input.clone(),
                    })
                    .map_err(OracleError::Json)?,
                },
            };
            outcomes.push(matches!(
                helpers::run_case(&helper, options.self_check)?,
                HelperRun::Difference
            ));
        } else {
            let group = root.join("websocket").join(safe_name(&case.name));
            fs::create_dir_all(&group).map_err(|source| OracleError::Filesystem {
                path: group.clone(),
                source,
            })?;
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|source| OracleError::Filesystem {
                    path: group.clone(),
                    source,
                })?;
            outcomes.push(attempt(options, &group, case, &runtime, true)?);
            outcomes.push(attempt(options, &group, case, &runtime, false)?);
        }
    }
    Ok(outcomes)
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
    let provider =
        runtime
            .block_on(start(case.clone()))
            .map_err(|source| OracleError::ScenarioInvalid {
                message: source.to_string(),
            })?;
    let path = config_path(root, case, &provider)?;
    let input = GatewayInput {
        options,
        group_dir: root,
        config_path: &path,
        case_env: &case.env,
    };
    let mut gateway = start_gateway(&input, first)?;
    let result = runtime.block_on(capture(case, provider, &gateway.base_url));
    let stopped = gateway.stop();
    let (observed, upstream) = result.map_err(|source| OracleError::ScenarioInvalid {
        message: source.to_string(),
    })?;
    stopped?;
    let mismatch = compare_transcript(case, &observed).err();
    if let Some(pointer) = mismatch {
        return Err(OracleError::ScenarioInvalid {
            message: format!("{}: {pointer}", case.name),
        });
    }
    Ok(upstream.is_some_and(|record| upstream_differs(case, &record)))
}

async fn capture(
    case: &WebSocketCase,
    provider: ProviderHandle,
    base: &str,
) -> Result<(TranscriptObservation, Option<ProviderObservation>), DriverError> {
    let observed = match exchange(base, case).await {
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
