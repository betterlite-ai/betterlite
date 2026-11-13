use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_mcp::credential_contracts::auth_config::oauth::ClientCredentialsConfig;
use betterlite_server::mcp::outbound_credentials::client_credentials::source::{
    ClientCredentialsTokenSource, Tunables,
};
use betterlite_server::mcp::outbound_credentials::client_credentials::token_post::{
    M2mTokenEndpointPost, TokenEndpointOutcome,
};
use betterlite_server::mcp::outbound_credentials::{TokenCacheBackend, WallClock};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::{
    ConfigSpec, ManualClock, OutcomeSpec, RecordingBackend, ScriptedPost, config_from, epoch,
    token_view, value_error,
};
use crate::helpers::{HelperError, HelperResult};

fn outcome(spec: &OutcomeSpec) -> Result<TokenEndpointOutcome, Box<HelperError>> {
    Ok(match spec {
        OutcomeSpec::Success { body } => TokenEndpointOutcome::Success {
            body: serde_json::from_str(body).map_err(|error| value_error(error.to_string()))?,
        },
        OutcomeSpec::Denied {
            status_code,
            detail,
        } => TokenEndpointOutcome::Denied {
            status_code: *status_code,
            detail: detail.clone(),
        },
        OutcomeSpec::Unreachable { detail } => TokenEndpointOutcome::Unreachable {
            detail: detail.clone(),
        },
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct MintInput {
    config: ConfigSpec,
    #[serde(default = "default_now_ms")]
    now_ms: i64,
    #[serde(default)]
    tunables: TunablesSpec,
    outcomes: Vec<OutcomeSpec>,
    #[serde(default)]
    operations: Vec<OperationSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct TunablesSpec {
    default_ttl_seconds: f64,
    expiry_skew_seconds: f64,
    min_cache_seconds: f64,
    max_locks: usize,
}

impl Default for TunablesSpec {
    fn default() -> Self {
        Self {
            default_ttl_seconds: 300.0,
            expiry_skew_seconds: 60.0,
            min_cache_seconds: 10.0,
            max_locks: 1024,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OperationSpec {
    op: OperationKind,
    server_id: String,
    #[serde(default)]
    failed_access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OperationKind {
    Get,
    Refetch,
}

const fn default_now_ms() -> i64 {
    1_000_000
}

pub(super) async fn minted(input: &MintInput) -> HelperResult {
    let config = config_from(&input.config)?;
    let (source, post, backend) = build_source(input)?;
    let results = mint_results(&source, &config, &input.operations).await;
    let post_calls = {
        let calls = post.calls.lock().unwrap_or_else(PoisonError::into_inner);
        Value::Array(
            calls
                .iter()
                .map(|(url, form)| json!({"url": url, "form": form}))
                .collect(),
        )
    };
    let (writes, deletes) = backend.state_view();
    Ok(json!({
        "results": results,
        "post_calls": post_calls,
        "writes": writes,
        "deletes": deletes,
    }))
}

fn build_source(
    input: &MintInput,
) -> Result<
    (
        ClientCredentialsTokenSource,
        Arc<ScriptedPost>,
        Arc<RecordingBackend>,
    ),
    Box<HelperError>,
> {
    let clock = Arc::new(ManualClock {
        now: epoch(input.now_ms)?,
    });
    let backend = Arc::new(RecordingBackend::default());
    let scripted: Vec<TokenEndpointOutcome> = input
        .outcomes
        .iter()
        .map(outcome)
        .collect::<Result<Vec<_>, _>>()?;
    let post = Arc::new(ScriptedPost {
        outcomes: Mutex::new(VecDeque::from(scripted)),
        calls: Mutex::new(Vec::new()),
    });
    let poster: Arc<dyn M2mTokenEndpointPost> = Arc::<ScriptedPost>::clone(&post);
    let cache: Arc<dyn TokenCacheBackend> = Arc::<RecordingBackend>::clone(&backend);
    let wall_clock: Arc<dyn WallClock> = Arc::<ManualClock>::clone(&clock);
    let source = ClientCredentialsTokenSource::new(
        poster,
        Some(cache),
        wall_clock,
        Tunables {
            default_ttl_seconds: input.tunables.default_ttl_seconds,
            expiry_skew_seconds: input.tunables.expiry_skew_seconds,
            min_cache_seconds: input.tunables.min_cache_seconds,
            max_locks: input.tunables.max_locks,
        },
    );
    Ok((source, post, backend))
}

async fn mint_results(
    source: &ClientCredentialsTokenSource,
    config: &ClientCredentialsConfig,
    operations: &[OperationSpec],
) -> Vec<Value> {
    let mut results = Vec::with_capacity(operations.len());
    for operation in operations {
        match operation.op {
            OperationKind::Get => {
                results.push(match source.get(&operation.server_id, config).await {
                    Ok(token) => json!({"op": "get", "token": token_view(&token)}),
                    Err(error) => json!({"op": "get", "error": error.to_string()}),
                });
            }
            OperationKind::Refetch => {
                let failed = operation.failed_access_token.clone().ok_or_else(|| {
                    value_error("failed_access_token is required for refetch".to_owned())
                });
                let Ok(failed_text) = failed else {
                    results.push(json!({"op": "refetch", "error": "missing failed_access_token"}));
                    continue;
                };
                let failed_token = SecretString::from(failed_text);
                let fresh = source
                    .refetch(&operation.server_id, config, &failed_token)
                    .await;
                results.push(json!({
                    "op": "refetch",
                    "access_token": fresh.map(|token| token.expose_secret().to_owned()),
                }));
            }
        }
    }
    results
}
