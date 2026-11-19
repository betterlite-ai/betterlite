use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::mcp_server_encoding::credentials::CredentialBlob;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_mcp::server_record::OAuth2Flow;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::mcp::backfills::flow::backfill_null_oauth2_flows;
use betterlite_server::mcp::backfills::{
    FLOW_BACKFILL_ACTOR, FlowBackfillTable, FlowCandidateRow, FlowSignals, UserCredentialRow,
    classify_null_flow_row, flow_name,
};

use super::support::{self, FakeError, SilentLog};

#[derive(Debug, Deserialize)]
pub(super) struct ClassifyInput {
    signals: Vec<SignalSpec>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SignalSpec {
    has_per_user_tokens: bool,
    authorization_url: Option<String>,
    registration_url: Option<String>,
    token_url: Option<String>,
    credentials: Option<CredentialBlob>,
}

#[derive(Debug, Deserialize)]
pub(super) struct FlowBackfillInput {
    flow_rows: Vec<FlowRowSpec>,
    #[serde(default)]
    user_credentials: Vec<CredentialSpec>,
}

#[derive(Debug, Deserialize)]
pub(super) struct FlowRowSpec {
    server_id: String,
    authorization_url: Option<String>,
    registration_url: Option<String>,
    token_url: Option<String>,
    credentials: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CredentialSpec {
    server_id: String,
    credential_b64: String,
}

struct FlowState {
    row: FlowCandidateRow,
    flow: Option<OAuth2Flow>,
    updated_by: Option<String>,
}

struct StampCall {
    flow: OAuth2Flow,
    server_ids: Vec<String>,
}

pub(super) struct FakeFlowTable {
    rows: Mutex<Vec<FlowState>>,
    credentials: Vec<UserCredentialRow>,
    stamps: Mutex<Vec<StampCall>>,
}

impl FakeFlowTable {
    fn new(rows: &[FlowRowSpec], credentials: Vec<UserCredentialRow>) -> Self {
        let states = rows
            .iter()
            .map(|spec| FlowState {
                row: FlowCandidateRow {
                    server_id: spec.server_id.clone(),
                    authorization_url: spec.authorization_url.clone(),
                    registration_url: spec.registration_url.clone(),
                    token_url: spec.token_url.clone(),
                    credentials: spec.credentials.clone(),
                },
                flow: None,
                updated_by: None,
            })
            .collect();
        Self {
            rows: Mutex::new(states),
            credentials,
            stamps: Mutex::new(Vec::new()),
        }
    }

    fn row_views(&self) -> Vec<Value> {
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        rows.sort_by(|left, right| left.row.server_id.cmp(&right.row.server_id));
        rows.iter().map(row_view).collect()
    }

    fn stamp_views(&self) -> Vec<Value> {
        let stamps = self.stamps.lock().unwrap_or_else(PoisonError::into_inner);
        stamps.iter().map(stamp_view).collect()
    }

    fn ambiguous(&self) -> Vec<String> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let mut ids: Vec<String> = rows
            .iter()
            .filter(|state| state.flow.is_none())
            .map(|state| state.row.server_id.clone())
            .collect();
        ids.sort();
        ids
    }
}

#[async_trait]
impl FlowBackfillTable for FakeFlowTable {
    type Error = FakeError;

    async fn null_flow_rows(&self) -> Result<Vec<FlowCandidateRow>, Self::Error> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(rows
            .iter()
            .filter(|state| state.flow.is_none())
            .map(|state| state.row.clone())
            .collect())
    }

    async fn user_credentials(
        &self,
        server_ids: &[String],
    ) -> Result<Vec<UserCredentialRow>, Self::Error> {
        Ok(self
            .credentials
            .iter()
            .filter(|row| server_ids.contains(&row.server_id))
            .cloned()
            .collect())
    }

    async fn stamp_flow(&self, server_ids: &[String], flow: OAuth2Flow) -> Result<(), Self::Error> {
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        for state in rows.iter_mut() {
            if state.flow.is_none() && server_ids.contains(&state.row.server_id) {
                state.flow = Some(flow);
                state.updated_by = Some(FLOW_BACKFILL_ACTOR.to_owned());
            }
        }
        drop(rows);
        self.stamps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(StampCall {
                flow,
                server_ids: server_ids.to_vec(),
            });
        Ok(())
    }
}

fn row_view(state: &FlowState) -> Value {
    json!({
        "server_id": state.row.server_id,
        "oauth2_flow": state.flow.map(flow_name),
        "updated_by": state.updated_by,
    })
}

fn stamp_view(stamp: &StampCall) -> Value {
    let mut ids = stamp.server_ids.clone();
    ids.sort();
    json!({"flow": flow_name(stamp.flow), "server_ids": ids})
}

pub(super) fn classify(input: &ClassifyInput) -> Result<Value, Box<HelperError>> {
    let results = input
        .signals
        .iter()
        .map(|signal| {
            let classification = classify_null_flow_row(&FlowSignals {
                has_per_user_tokens: signal.has_per_user_tokens,
                authorization_url: signal.authorization_url.as_deref(),
                registration_url: signal.registration_url.as_deref(),
                token_url: signal.token_url.as_deref(),
                credentials: signal.credentials.as_ref(),
            });
            json!({
                "flow": classification.flow.map(flow_name),
                "rule": classification.rule.as_str(),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({"results": results}))
}

pub(super) async fn flow_backfill(input: &FlowBackfillInput) -> Result<Value, Box<HelperError>> {
    let table = FakeFlowTable::new(&input.flow_rows, credential_rows(&input.user_credentials));
    let counts = backfill_null_oauth2_flows(&table, &support::cipher(), &SilentLog)
        .await
        .map_err(|error| support::value_error(error.to_string()))?;
    let counts: BTreeMap<&str, usize> = counts
        .iter()
        .map(|(rule, count)| (rule.as_str(), *count))
        .collect();
    Ok(json!({
        "counts": counts,
        "rows": table.row_views(),
        "stamps": table.stamp_views(),
        "ambiguous": table.ambiguous(),
    }))
}

fn credential_rows(specs: &[CredentialSpec]) -> Vec<UserCredentialRow> {
    specs
        .iter()
        .map(|spec| UserCredentialRow {
            server_id: spec.server_id.clone(),
            credential_b64: spec.credential_b64.clone(),
        })
        .collect()
}
