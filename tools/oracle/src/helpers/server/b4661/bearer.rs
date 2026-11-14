use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_mcp::credential_contracts::header_carrier::HeaderCarrier;
use betterlite_mcp::credential_contracts::httpx_auth::{Auth, HeaderMap};
use betterlite_server::mcp::outbound_credentials::client_credentials::bearer_auth::{
    AuthorizedCall, ClientCredentialsBearerAuth, Refetch,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::json;

use super::support::{runtime, value_error};
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
pub(crate) struct BearerInput {
    scenarios: Vec<ScenarioSpec>,
}

#[derive(Debug, Deserialize)]
struct ScenarioSpec {
    access_token: String,
    statuses: Vec<u16>,
    #[serde(default)]
    refetch: Vec<Option<String>>,
}

struct ScriptedCall {
    statuses: Mutex<VecDeque<u16>>,
    sends: Mutex<Vec<Vec<[String; 2]>>>,
}

#[async_trait]
impl AuthorizedCall for ScriptedCall {
    type Output = u16;

    async fn send(&self, auth: &(dyn Auth + Sync)) -> u16 {
        let mut headers = HeaderMap::new();
        auth.apply(&mut headers);
        let recorded: Vec<[String; 2]> = headers
            .iter()
            .map(|(name, value)| {
                [
                    name.as_str().to_owned(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                ]
            })
            .collect();
        self.sends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        self.statuses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default()
    }

    fn is_unauthorized(&self, output: &u16) -> bool {
        *output == 401
    }
}

struct ScriptedRefetch {
    answers: Mutex<VecDeque<Option<SecretString>>>,
    asked: Mutex<Vec<String>>,
}

#[async_trait]
impl Refetch for ScriptedRefetch {
    async fn refetch(&self, failed_access_token: &SecretString) -> Option<SecretString> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(failed_access_token.expose_secret().to_owned());
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .flatten()
    }
}

pub(super) fn run(input: &BearerInput) -> HelperResult {
    let executor = runtime()?;
    let mut scenarios = Vec::with_capacity(input.scenarios.len());
    for case in &input.scenarios {
        scenarios.push(executor.block_on(scenario_view(case))?);
    }
    Ok(json!({"scenarios": scenarios}))
}

async fn scenario_view(scenario: &ScenarioSpec) -> HelperResult {
    let mut answers = VecDeque::new();
    answers.extend(
        scenario
            .refetch
            .iter()
            .map(|answer| answer.clone().map(SecretString::from)),
    );
    let refetch = Arc::new(ScriptedRefetch {
        answers: Mutex::new(answers),
        asked: Mutex::new(Vec::new()),
    });
    let refetcher: Arc<dyn Refetch> = Arc::<ScriptedRefetch>::clone(&refetch);
    let access_token = SecretString::from(scenario.access_token.clone());
    let bearer =
        ClientCredentialsBearerAuth::new(&access_token, refetcher, HeaderCarrier::default())
            .map_err(|error| value_error(error.to_string()))?;
    let call = ScriptedCall {
        statuses: Mutex::new(VecDeque::from(scenario.statuses.clone())),
        sends: Mutex::new(Vec::new()),
    };
    let final_status = bearer
        .send_with_retry(&call)
        .await
        .map_err(|error| value_error(error.to_string()))?;
    let sends = call
        .sends
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let asked_tokens = refetch
        .asked
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(json!({
        "sends": sends,
        "final_status": final_status,
        "refetched": asked_tokens,
    }))
}
