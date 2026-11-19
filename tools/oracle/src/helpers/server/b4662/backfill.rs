use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_db::mcp_server_encoding::credentials::CredentialBlob;
use betterlite_db::mcp_stored_contracts::{JsonValue, MCPAuth};
use betterlite_mcp::server_record::OAuth2Flow;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::mcp::backfills::flow::backfill_null_oauth2_flows;
use betterlite_server::mcp::backfills::issuer::backfill_discovery_stamped_issuers;
use betterlite_server::mcp::backfills::log::{
    FlowBackfillEvent, FlowBackfillLog, IssuerBackfillEvent, IssuerBackfillLog,
};
use betterlite_server::mcp::backfills::{
    DISCOVERY_ACTOR, FLOW_BACKFILL_ACTOR, FlowBackfillTable, FlowCandidateRow, FlowSignals,
    ISSUER_STAMP_BACKFILL_ACTOR, IssuerBackfillTable, IssuerCandidateRow, UserCredentialRow,
    classify_null_flow_row, flow_name,
};

use super::support::{self, FakeError};

struct SilentLog;

impl FlowBackfillLog for SilentLog {
    fn flow_event(&self, _event: FlowBackfillEvent<'_>) {}
}

impl IssuerBackfillLog for SilentLog {
    fn issuer_event(&self, _event: IssuerBackfillEvent<'_>) {}
}

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
    kind: CredentialKind,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CredentialKind {
    Oauth2,
    Key,
    Garbage,
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
        rows.iter()
            .map(|state| {
                json!({
                    "server_id": state.row.server_id,
                    "oauth2_flow": state.flow.map(flow_name),
                    "updated_by": state.updated_by,
                })
            })
            .collect()
    }

    fn stamp_views(&self) -> Vec<Value> {
        let stamps = self.stamps.lock().unwrap_or_else(PoisonError::into_inner);
        stamps
            .iter()
            .map(|stamp| {
                let mut ids = stamp.server_ids.clone();
                ids.sort();
                json!({"flow": flow_name(stamp.flow), "server_ids": ids})
            })
            .collect()
    }

    fn ambiguous(&self) -> Vec<String> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        support::sorted_ids(
            rows.iter()
                .filter(|state| state.flow.is_none())
                .map(|state| state.row.server_id.clone()),
        )
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

#[derive(Debug, Deserialize)]
pub(super) struct IssuerBackfillInput {
    issuer_rows: Vec<IssuerRowSpec>,
    #[serde(default)]
    fail_server_ids: Vec<String>,
    runs: usize,
}

#[derive(Debug, Deserialize)]
pub(super) struct IssuerRowSpec {
    server_id: String,
    alias: Option<String>,
    server_name: Option<String>,
    auth_type: Option<MCPAuth>,
    issuer: Option<String>,
    authorization_url: Option<String>,
    token_url: Option<String>,
    registration_url: Option<String>,
    updated_by: Option<String>,
}

pub(super) struct FakeIssuerTable {
    rows: Mutex<Vec<IssuerCandidateRow>>,
    fail: Vec<String>,
    attempted: Mutex<Vec<String>>,
    updated: Mutex<Vec<String>>,
}

impl FakeIssuerTable {
    fn new(rows: &[IssuerRowSpec], fail: &[String]) -> Self {
        let rows = rows
            .iter()
            .map(|spec| IssuerCandidateRow {
                server_id: spec.server_id.clone(),
                alias: spec.alias.clone(),
                server_name: spec.server_name.clone(),
                auth_type: spec.auth_type,
                issuer: spec.issuer.clone(),
                authorization_url: spec.authorization_url.clone(),
                token_url: spec.token_url.clone(),
                registration_url: spec.registration_url.clone(),
                updated_by: spec.updated_by.clone(),
            })
            .collect();
        Self {
            rows: Mutex::new(rows),
            fail: fail.to_vec(),
            attempted: Mutex::new(Vec::new()),
            updated: Mutex::new(Vec::new()),
        }
    }

    fn row_views(&self) -> Vec<Value> {
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        rows.sort_by(|left, right| left.server_id.cmp(&right.server_id));
        rows.iter()
            .map(|row| {
                json!({
                    "server_id": row.server_id,
                    "issuer": row.issuer,
                    "updated_by": row.updated_by,
                })
            })
            .collect()
    }

    fn updated(&self) -> Vec<String> {
        support::sorted_ids(
            self.updated
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .cloned(),
        )
    }

    fn failed(&self) -> Vec<String> {
        let attempted = self
            .attempted
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let updated = self.updated.lock().unwrap_or_else(PoisonError::into_inner);
        support::sorted_ids(
            attempted
                .iter()
                .filter(|server_id| !updated.contains(*server_id))
                .cloned(),
        )
    }
}

#[async_trait]
impl IssuerBackfillTable for FakeIssuerTable {
    type Error = FakeError;

    async fn discovery_stamped_rows(&self) -> Result<Vec<IssuerCandidateRow>, Self::Error> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(rows
            .iter()
            .filter(|row| row.updated_by.as_deref() == Some(DISCOVERY_ACTOR))
            .filter(|row| {
                matches!(
                    row.auth_type,
                    Some(MCPAuth::Oauth2 | MCPAuth::TruePassthrough | MCPAuth::OauthDelegate)
                )
            })
            .cloned()
            .collect())
    }

    async fn clear_issuer(&self, server_id: &str) -> Result<(), Self::Error> {
        self.attempted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(server_id.to_owned());
        if self.fail.iter().any(|failed| failed.as_str() == server_id) {
            return Err(FakeError("the fake issuer write failed"));
        }
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        for row in rows.iter_mut().filter(|row| row.server_id == server_id) {
            row.issuer = None;
            row.updated_by = Some(ISSUER_STAMP_BACKFILL_ACTOR.to_owned());
        }
        drop(rows);
        self.updated
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(server_id.to_owned());
        Ok(())
    }
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

pub(super) async fn issuer_backfill(
    input: &IssuerBackfillInput,
) -> Result<Value, Box<HelperError>> {
    let table = FakeIssuerTable::new(&input.issuer_rows, &input.fail_server_ids);
    let mut healed = 0;
    for _run in 0..input.runs {
        healed += backfill_discovery_stamped_issuers(&table, &SilentLog)
            .await
            .map_err(|error| support::value_error(error.to_string()))?;
    }
    Ok(json!({
        "healed": healed,
        "rows": table.row_views(),
        "updated": table.updated(),
        "failed": table.failed(),
    }))
}

fn credential_rows(specs: &[CredentialSpec]) -> Vec<UserCredentialRow> {
    specs
        .iter()
        .map(|spec| UserCredentialRow {
            server_id: spec.server_id.clone(),
            credential_b64: credential_blob(spec.kind),
        })
        .collect()
}

fn credential_blob(kind: CredentialKind) -> String {
    match kind {
        CredentialKind::Oauth2 => STANDARD.encode(r#"{"type":"oauth2","access_token":"tok"}"#),
        CredentialKind::Key => STANDARD.encode("a-plain-api-key"),
        CredentialKind::Garbage => "!!!not-base64!!!".to_owned(),
    }
}
