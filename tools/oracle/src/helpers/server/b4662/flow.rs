use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_mcp::server_record::OAuth2Flow;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::mcp::backfills::flow::backfill_null_oauth2_flows;
use betterlite_server::mcp::backfills::{
    FLOW_BACKFILL_ACTOR, FlowBackfillTable, FlowCandidateRow, FlowSignals, UserCredentialRow,
    classify_null_flow_row, flow_name,
};

use super::support::{self, FakeError, SilentLog};

#[derive(Debug, Deserialize)]
pub(crate) struct ClassifyInput {
    signals: Vec<SignalSpec>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SignalSpec {
    has_per_user_tokens: bool,
    authorization_url: Option<String>,
    registration_url: Option<String>,
    token_url: Option<String>,
    credentials: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FlowBackfillInput {
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
    credentials: Option<Value>,
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
    fn new(
        rows: &[FlowRowSpec],
        credentials: Vec<UserCredentialRow>,
    ) -> Result<Self, Box<HelperError>> {
        let mut states = Vec::with_capacity(rows.len());
        for spec in rows {
            states.push(FlowState {
                row: FlowCandidateRow {
                    server_id: spec.server_id.clone(),
                    authorization_url: spec.authorization_url.clone(),
                    registration_url: spec.registration_url.clone(),
                    token_url: spec.token_url.clone(),
                    credentials: decode_stored(spec.credentials.as_ref())?,
                },
                flow: None,
                updated_by: None,
            });
        }
        Ok(Self {
            rows: Mutex::new(states),
            credentials,
            stamps: Mutex::new(Vec::new()),
        })
    }

    fn row_views(&self) -> Vec<Value> {
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        rows.sort_by(|left, right| left.row.server_id.cmp(&right.row.server_id));
        rows.iter().map(row_view).collect()
    }

    fn ambiguous(&self) -> Vec<String> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let mut ids: Vec<String> = rows
            .iter()
            .filter(|state| state.flow.is_none())
            .map(|state| state.row.server_id.clone())
            .collect();
        drop(rows);
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
    let mut results = Vec::with_capacity(input.signals.len());
    for signal in &input.signals {
        let credentials = decode_stored(signal.credentials.as_ref())?;
        let classification = classify_null_flow_row(&FlowSignals {
            has_per_user_tokens: signal.has_per_user_tokens,
            authorization_url: signal.authorization_url.as_deref(),
            registration_url: signal.registration_url.as_deref(),
            token_url: signal.token_url.as_deref(),
            credentials: credentials.as_ref(),
        });
        results.push(json!({
            "flow": classification.flow.map(flow_name),
            "rule": classification.rule.as_str(),
        }));
    }
    Ok(json!({"results": results}))
}

// The case's raw JSON cannot decode straight into the stored types inside the tagged input enum, so it is converted here.
fn decode_stored<T>(value: Option<&Value>) -> Result<Option<T>, Box<HelperError>>
where
    T: DeserializeOwned,
{
    value
        .cloned()
        .map(|document| {
            serde_json::from_value::<T>(document)
                .map_err(|error| support::value_error(error.to_string()))
        })
        .transpose()
}

pub(super) async fn flow_backfill(input: &FlowBackfillInput) -> Result<Value, Box<HelperError>> {
    let table = FakeFlowTable::new(&input.flow_rows, credential_rows(&input.user_credentials))?;
    let measured = backfill_null_oauth2_flows(&table, &support::cipher(), &SilentLog)
        .await
        .map_err(|error| support::value_error(error.to_string()))?;
    let counts: BTreeMap<&str, usize> = measured
        .iter()
        .map(|(rule, count)| (rule.as_str(), *count))
        .collect();
    let stamps = table.stamps.lock().unwrap_or_else(PoisonError::into_inner);
    let views: Vec<Value> = stamps.iter().map(stamp_view).collect();
    drop(stamps);
    Ok(json!({
        "counts": counts,
        "rows": table.row_views(),
        "stamps": views,
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
