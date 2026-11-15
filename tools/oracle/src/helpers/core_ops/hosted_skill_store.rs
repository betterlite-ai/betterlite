use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_core::skills::store::{
    HostedSkillStore, SKILL_ID_PREFIX, SkillIdSource, StoreError,
};
use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};

mod principal;
mod scripted;
mod steps;
mod warnings;

use scripted::Scripted;
use warnings::WarnLog;

const CACHE_CLOCK: f64 = 1000.0;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    now: String,
    uuid: String,
    #[serde(default)]
    rows: Vec<RowFields>,
    #[serde(default)]
    page: Vec<RowFields>,
    #[serde(default)]
    faults: BTreeMap<String, bool>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    action: String,
    data: Option<NewSkillFields>,
    user_id: Option<String>,
    principal: Option<PrincipalFields>,
    skill_id: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrincipalFields {
    user_id: Option<String>,
    team_id: Option<String>,
    org_id: Option<String>,
    api_key: Option<String>,
    token: Option<String>,
    #[serde(default)]
    admin: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewSkillFields {
    display_title: Option<String>,
    description: Option<String>,
    instructions: Option<String>,
    metadata: Option<BTreeMap<String, JsonData>>,
    file_content: Option<String>,
    file_name: Option<String>,
    file_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RowFields {
    skill_id: String,
    display_title: Option<String>,
    description: Option<String>,
    instructions: Option<String>,
    source: Option<String>,
    latest_version: Option<String>,
    file_content: Option<String>,
    file_name: Option<String>,
    file_type: Option<String>,
    metadata: Option<BTreeMap<String, JsonData>>,
    created_at: Option<String>,
    created_by: Option<String>,
    updated_at: Option<String>,
    updated_by: Option<String>,
}

pub(crate) struct Op;

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn store_error(error: StoreError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "StoreError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let scripted = Scripted::new(input)?;
        let uuid = input.uuid.clone();
        let ids: SkillIdSource = Arc::new(move || format!("{SKILL_ID_PREFIX}{uuid}"));
        let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| CACHE_CLOCK);
        let store = HostedSkillStore::new(scripted.clone(), ids, clock).map_err(store_error)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| invalid_input(error.to_string()))?;
        let log = WarnLog::default();
        let subscriber = tracing_subscriber::registry().with(log.clone());
        let steps = tracing::subscriber::with_default(subscriber, || {
            runtime.block_on(steps::run_steps(&store, &input.steps))
        })?;
        Ok(scripted.response(&steps, &log.entries()))
    }
}
