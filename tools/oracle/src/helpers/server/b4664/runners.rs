use std::collections::BTreeMap;

use betterlite_mcp::sso_assertions::assertion::SsoIdentityAssertion;
use betterlite_mcp::sso_assertions::read::SsoAssertionStore as _;
use serde::Deserialize;
use serde_json::Value;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::fakes::Harness;
use super::support::{
    AccessFailure, AssertionSpec, OutcomeSpec, answered, assertion_from_spec, kind_text, view,
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
    let harness = Harness::new(
        rows.clone(),
        rows,
        input.failure,
        &input.vars,
        &input.outcome,
    )?;
    let answer = harness.refresher().refresh(&input.user_id, &starting).await;
    Ok(match &answer {
        Ok(renewed) => answered("renewed", &Value::Null, &view(renewed), &harness),
        Err(failure) => answered(
            kind_text(failure.kind),
            &Value::String(failure.detail.clone()),
            &Value::Null,
            &harness,
        ),
    })
}

pub(crate) async fn fetched(input: &StoreInput) -> HelperResult {
    let inner_rows = rows_from(&input.rows)?;
    let fresh_rows = match &input.fresh {
        Some(fresh) => rows_from(fresh)?,
        None => inner_rows.clone(),
    };
    let harness = Harness::new(
        inner_rows,
        fresh_rows,
        input.failure,
        &input.vars,
        &input.outcome,
    )?;
    let store = harness.store(input.loser, input.now_ms)?;
    let found = store.fetch(&input.user_id).await;
    let (result, detail, assertion) = match &found {
        Ok(Some(stored)) => ("assertion", Value::Null, view(stored)),
        Ok(None) => ("none", Value::Null, Value::Null),
        Err(problem) => (
            "unavailable",
            Value::String(problem.cause.to_string()),
            Value::Null,
        ),
    };
    Ok(answered(result, &detail, &assertion, &harness))
}
