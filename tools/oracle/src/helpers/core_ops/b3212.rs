use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_core::api::stored_skill_reads::StoredSkillReads;
use betterlite_core::api::stored_skill_reads::cache::SkillCacheLimits;
use betterlite_core::api::stored_skill_reads::error::StoredSkillReadError;
use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod principal;
mod scripted;
mod steps;

use scripted::Scripted;

const CACHE_CLOCK: f64 = 1000.0;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    now: String,
    #[serde(default)]
    rows: Vec<RowFields>,
    #[serde(default)]
    page: Vec<RowFields>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RowFields {
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    action: String,
    principal: Option<PrincipalFields>,
    skill_id: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
    available: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrincipalFields {
    user_id: Option<String>,
    team_id: Option<String>,
    org_id: Option<String>,
    api_key: Option<String>,
    token: Option<String>,
    #[serde(default)]
    admin: bool,
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

fn read_error(error: &StoredSkillReadError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "StoredSkillReadError".to_owned(),
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
        let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| CACHE_CLOCK);
        let reads = StoredSkillReads::new(scripted.clone(), SkillCacheLimits::default(), clock)
            .map_err(|error| read_error(&error))?;
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| invalid_input(error.to_string()))?;
        let steps = runtime.block_on(steps::run_steps(&reads, &scripted, &input.steps))?;
        Ok(scripted.response(&steps))
    }
}
