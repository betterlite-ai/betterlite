use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::mcp_stored_contracts::MCPAuth;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::mcp::backfills::issuer::heal_stamped_issuers;
use betterlite_server::mcp::backfills::{
    DISCOVERY_ACTOR, ISSUER_STAMP_BACKFILL_ACTOR, IssuerBackfillTable, IssuerCandidateRow,
};

use super::support::{self, FakeError, SilentLog};

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
        let mut ids: Vec<String> = self
            .updated
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        ids.sort();
        ids
    }

    fn failed(&self) -> Vec<String> {
        let attempted = self
            .attempted
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let updated = self.updated.lock().unwrap_or_else(PoisonError::into_inner);
        let failed: BTreeSet<&String> = attempted
            .iter()
            .filter(|server_id| !updated.contains(*server_id))
            .collect();
        failed.into_iter().cloned().collect()
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

pub(super) async fn issuer_backfill(
    input: &IssuerBackfillInput,
) -> Result<Value, Box<HelperError>> {
    let table = FakeIssuerTable::new(&input.issuer_rows, &input.fail_server_ids);
    let mut healed = 0;
    for _run in 0..input.runs {
        healed += heal_stamped_issuers(&table, &SilentLog)
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
