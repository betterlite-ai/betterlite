use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_server::mcp::outbound_credentials::WallClock;
use betterlite_server::mcp::outbound_credentials::client_credentials::source::{
    ClientCredentialsTokenSource, Tunables,
};
use betterlite_server::mcp::outbound_credentials::client_credentials::token_post::{
    M2mTokenEndpointPost, TokenEndpointOutcome,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::{
    ConfigSpec, ManualClock, OutcomeSpec, ScriptedPost, config_from, epoch, value_error,
};
use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, Deserialize)]
pub(crate) struct MintInput {
    config: ConfigSpec,
    outcomes: Vec<OutcomeSpec>,
    operations: Vec<OperationSpec>,
}

#[derive(Debug, Deserialize)]
struct OperationSpec {
    server_id: String,
    #[serde(default)]
    advance_ms: i64,
}

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

pub(super) async fn minted(input: &MintInput) -> HelperResult {
    let config = config_from(&input.config)?;
    let clock = Arc::new(ManualClock {
        now: Mutex::new(epoch(1_000_000)?),
    });
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
    let wall_clock: Arc<dyn WallClock> = Arc::<ManualClock>::clone(&clock);
    let source = ClientCredentialsTokenSource::new(poster, None, wall_clock, Tunables::default());
    let mut results = Vec::with_capacity(input.operations.len());
    for operation in &input.operations {
        clock.advance(operation.advance_ms);
        results.push(match source.get(&operation.server_id, &config).await {
            Ok(token) => json!({"op": "get", "token": {
                "access_token": token.access_token,
                "expires_at_ms": token.expires_at.map(|at| at.timestamp_millis()),
                "refresh_token": token.refresh_token,
                "scopes": token.scopes,
                "identity_binding_proof": token.identity_binding_proof,
            }}),
            Err(error) => json!({"op": "get", "error": error.to_string()}),
        });
    }
    let post_calls: Vec<Value> = {
        let calls = post.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls
            .iter()
            .map(|(url, form)| json!({"url": url, "form": form}))
            .collect()
    };
    Ok(json!({"results": results, "post_calls": post_calls}))
}
