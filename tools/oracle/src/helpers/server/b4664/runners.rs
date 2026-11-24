use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use betterlite_mcp::sso_assertions::assertion::SsoIdentityAssertion;
use betterlite_mcp::sso_assertions::read::SsoAssertionStore as _;
use betterlite_server::mcp::outbound_credentials::refresh_store::InProcessRefreshCoordinator;
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::refresher::AssertionRefresher;
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::store::{
    DEFAULT_EXPIRY_SKEW, RefreshingAssertionStore,
};
use serde::Deserialize;
use serde_json::Value;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::support::{
    AccessFailure, AssertionSpec, FixedClient, FixedFactory, InnerStore, MemoryAccess, OutcomeSpec,
    ScriptedTransport, answered, assertion_from_spec, clock, kind_text, scripted, transcript, view,
};

#[derive(Debug, Deserialize)]
pub(crate) struct RefreshInput {
    user_id: String,
    assertion: AssertionSpec,
    #[serde(default)]
    row: Option<AssertionSpec>,
    vars: BTreeMap<String, String>,
    outcome: OutcomeSpec,
    #[serde(default)]
    failure: AccessFailure,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StoreInput {
    user_id: String,
    now_ms: i64,
    rows: BTreeMap<String, AssertionSpec>,
    #[serde(default)]
    fresh: Option<BTreeMap<String, AssertionSpec>>,
    vars: BTreeMap<String, String>,
    outcome: OutcomeSpec,
    #[serde(default)]
    loser: bool,
    #[serde(default)]
    failure: AccessFailure,
}

fn rows_from(
    specs: &BTreeMap<String, AssertionSpec>,
) -> Result<BTreeMap<String, SsoIdentityAssertion>, Box<HelperError>> {
    specs
        .iter()
        .map(|(user_id, spec)| Ok((user_id.clone(), assertion_from_spec(spec)?)))
        .collect()
}

pub(crate) async fn refreshed(input: &RefreshInput) -> HelperResult {
    let starting = assertion_from_spec(&input.assertion)?;
    let rows = match &input.row {
        Some(spec) => BTreeMap::from([(input.user_id.clone(), assertion_from_spec(spec)?)]),
        None => BTreeMap::new(),
    };
    let access = Arc::new(MemoryAccess {
        rows: Mutex::new(rows),
        writes: Mutex::new(Vec::new()),
        failure: input.failure,
    });
    let transport = ScriptedTransport {
        outcomes: Arc::new(Mutex::new(VecDeque::from([scripted(&input.outcome)?]))),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let refresher = AssertionRefresher {
        transport: transport.clone(),
        client_source: FixedClient {
            vars: input.vars.clone(),
        },
        access: Arc::clone(&access),
    };
    let answer = refresher.refresh(&input.user_id, &starting).await;
    let sent = transcript(&transport, &access.writes);
    match &answer {
        Ok(renewed) => Ok(answered("renewed", &Value::Null, &view(renewed), &sent)),
        Err(failure) => Ok(answered(
            kind_text(failure.kind),
            &Value::String(failure.detail.clone()),
            &Value::Null,
            &sent,
        )),
    }
}

pub(crate) async fn fetched(input: &StoreInput) -> HelperResult {
    let inner_rows = rows_from(&input.rows)?;
    let fresh_rows = match &input.fresh {
        Some(fresh) => rows_from(fresh)?,
        None => inner_rows.clone(),
    };
    let access = Arc::new(MemoryAccess {
        rows: Mutex::new(fresh_rows),
        writes: Mutex::new(Vec::new()),
        failure: input.failure,
    });
    let transport = ScriptedTransport {
        outcomes: Arc::new(Mutex::new(VecDeque::from([scripted(&input.outcome)?]))),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let store = RefreshingAssertionStore {
        inner: InnerStore {
            rows: Mutex::new(inner_rows),
        },
        refresher: AssertionRefresher {
            transport: transport.clone(),
            client_source: FixedClient {
                vars: input.vars.clone(),
            },
            access: Arc::clone(&access),
        },
        access,
        factory: FixedFactory { loser: input.loser },
        in_process: Arc::new(InProcessRefreshCoordinator::default()),
        distributed: Mutex::new(None),
        skew: DEFAULT_EXPIRY_SKEW,
        clock: clock(input.now_ms)?,
    };
    let found = store.fetch(&input.user_id).await;
    let sent = transcript(&transport, &store.access.writes);
    match &found {
        Ok(Some(assertion)) => Ok(answered("assertion", &Value::Null, &view(assertion), &sent)),
        Ok(None) => Ok(answered("none", &Value::Null, &Value::Null, &sent)),
        Err(unavailable) => Ok(answered(
            "unavailable",
            &Value::String(unavailable.cause.to_string()),
            &Value::Null,
            &sent,
        )),
    }
}
