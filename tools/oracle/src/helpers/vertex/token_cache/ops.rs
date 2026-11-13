use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_vertex::token_cache::key::CredentialKey;
use betterlite_vertex::token_cache::probe::{try_cached_token, try_usable_cached_token};
use betterlite_vertex::token_cache::record::{
    CacheRecord, CredentialSnapshot, unpack_cached_credentials,
};
use betterlite_vertex::token_cache::state::{TokenState, token_state};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::input::{CredentialCase, Input, ProbeCase, RecordCase, Scenario, StateName};
use crate::helpers::HelperError;

const NOW: SystemTime = SystemTime::UNIX_EPOCH;

pub(crate) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    let scenarios = input
        .scenarios
        .iter()
        .map(scenario_row)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "scenarios": scenarios }))
}

fn scenario_row(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let snapshots = scenario
        .credentials
        .iter()
        .map(|case| (&case.name, Arc::new(snapshot(case))))
        .collect::<Vec<_>>();
    let states = snapshots
        .iter()
        .map(|(name, credential)| {
            json!({"name": name, "state": state_label(token_state(credential, NOW))})
        })
        .collect::<Vec<_>>();
    let by_name = snapshots
        .iter()
        .map(|(name, credential)| (name.as_str(), Arc::clone(credential)))
        .collect::<BTreeMap<_, _>>();
    let built = build_records(&scenario.records, &by_name)?;
    let probes = scenario
        .probes
        .iter()
        .map(|probe| probe_row(&built, probe))
        .collect::<Vec<_>>();
    Ok(json!({"credentials": states, "probes": probes}))
}

fn snapshot(case: &CredentialCase) -> CredentialSnapshot {
    CredentialSnapshot {
        token: case.token.clone().map(SecretString::from),
        state: case.state.map(StateName::state),
        expires_at: None,
        expired: case.expired,
        valid: case.valid,
        quota_project_id: case.quota_project_id.clone(),
        project_id: case.project_id.clone(),
    }
}

fn build_records(
    cases: &[RecordCase],
    credentials: &BTreeMap<&str, Arc<CredentialSnapshot>>,
) -> Result<BTreeMap<CredentialKey, CacheRecord>, Box<HelperError>> {
    let mut records = BTreeMap::new();
    for case in cases {
        let name = case.credential.as_str();
        let credential = credentials
            .get(name)
            .ok_or_else(|| invalid(format!("no credential is named {name}")))?;
        let record = if case.legacy {
            CacheRecord::Legacy {
                credential: Arc::clone(credential),
            }
        } else {
            CacheRecord::Paired {
                credential: Arc::clone(credential),
                project: case.cached_project.clone(),
            }
        };
        let key = CredentialKey {
            credential: case.credentials.clone().map(SecretString::from),
            project: case.project.clone(),
        };
        records.insert(key, record);
    }
    Ok(records)
}

fn probe_row(records: &BTreeMap<CredentialKey, CacheRecord>, probe: &ProbeCase) -> Value {
    let key = CredentialKey {
        credential: probe.credentials.clone().map(SecretString::from),
        project: probe.project.clone(),
    };
    let requested = probe.requested_project.as_deref();
    let (cached, cached_project) = unpack_cached_credentials(records.get(&key));
    let unpacked = json!({"cached": cached.is_some(), "project": cached_project});
    let fresh = try_cached_token(records, &key, requested, NOW)
        .map(|access| json!({"token": access.token.expose_secret(), "project": access.project}));
    let usable = try_usable_cached_token(records, &key, requested, NOW).map(|usable| {
        json!({
            "token": usable.token.expose_secret(),
            "project": usable.project,
            "state": state_label(usable.state),
            "cached_project": usable.cached_project,
        })
    });
    json!({"unpacked": unpacked, "fresh": fresh, "usable": usable})
}

const fn state_label(state: TokenState) -> &'static str {
    match state {
        TokenState::Fresh => "fresh",
        TokenState::Stale => "stale",
        TokenState::Invalid => "invalid",
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
