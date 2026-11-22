use std::collections::{BTreeMap, BTreeSet};

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_oracle_helper_api::error::HelperError;
use serde::Deserialize;

use super::super::invalid;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) token: Token,
    pub(crate) reservation_cost: serde_json::Number,
    pub(crate) fail_closed: bool,
    #[serde(default)]
    pub(crate) throttle_percentage: Option<f64>,
    #[serde(default)]
    pub(crate) user: Option<User>,
    #[serde(default)]
    pub(crate) memory: BTreeMap<String, serde_json::Number>,
    #[serde(default)]
    pub(crate) sources: BTreeMap<String, serde_json::Number>,
    #[serde(default)]
    pub(crate) failures: BTreeSet<String>,
    #[serde(default)]
    pub(crate) pipeline_fails: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Token {
    #[serde(rename = "token")]
    pub(crate) secret: String,
    #[serde(default)]
    pub(crate) max_budget: Option<serde_json::Number>,
    pub(crate) spend: serde_json::Number,
    #[serde(default)]
    pub(crate) tpm_limit: Option<i64>,
    #[serde(default)]
    pub(crate) rpm_limit: Option<i64>,
    pub(crate) throttle: bool,
    pub(crate) user_id: String,
    #[serde(default)]
    pub(crate) budget_limits: Option<Vec<Limit>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Limit {
    budget_duration: String,
    max_budget: serde_json::Number,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct User {
    #[serde(rename = "user_id")]
    pub(crate) id: String,
    pub(crate) max_budget: serde_json::Number,
    pub(crate) spend: serde_json::Number,
}

impl Token {
    // the token pydantic builds in the helper: the metadata carries the throttle opt-in, the rest is the row
    pub(crate) fn auth(&self) -> Result<UserAPIKeyAuth, Box<HelperError>> {
        let mut metadata = BTreeMap::new();
        let _throttle = metadata.insert(
            "throttle_on_budget_exceeded".to_owned(),
            JsonData::Bool(self.throttle),
        );
        let mut fields = BTreeMap::from([
            ("token".to_owned(), JsonData::String(self.secret.clone())),
            ("spend".to_owned(), JsonData::Number(self.spend.clone())),
            ("user_id".to_owned(), JsonData::String(self.user_id.clone())),
            ("metadata".to_owned(), JsonData::Object(metadata)),
        ]);
        self.add_limits(&mut fields);
        UserAPIKeyAuth::construct(AuthInput::Dict(fields))
            .map_err(|error| invalid(error.to_string()))
    }

    fn add_limits(&self, fields: &mut BTreeMap<String, JsonData>) {
        if let Some(max_budget) = &self.max_budget {
            let _budget = fields.insert(
                "max_budget".to_owned(),
                JsonData::Number(max_budget.clone()),
            );
        }
        if let Some(tpm_limit) = self.tpm_limit {
            let _tpm = fields.insert(
                "tpm_limit".to_owned(),
                JsonData::Number(serde_json::Number::from(tpm_limit)),
            );
        }
        if let Some(rpm_limit) = self.rpm_limit {
            let _rpm = fields.insert(
                "rpm_limit".to_owned(),
                JsonData::Number(serde_json::Number::from(rpm_limit)),
            );
        }
        let Some(limits) = &self.budget_limits else {
            return;
        };
        let windows: Vec<JsonData> = limits.iter().map(Limit::window).collect();
        let _limits = fields.insert("budget_limits".to_owned(), JsonData::Array(windows));
    }
}

impl Limit {
    // the budget_limits entry _coerce_window reads: a duration and the budget that covers it
    fn window(&self) -> JsonData {
        let mut fields = BTreeMap::new();
        let _duration = fields.insert(
            "budget_duration".to_owned(),
            JsonData::String(self.budget_duration.clone()),
        );
        let _budget = fields.insert(
            "max_budget".to_owned(),
            JsonData::Number(self.max_budget.clone()),
        );
        JsonData::Object(fields)
    }
}

impl User {
    pub(crate) fn row(&self) -> Result<LiteLLMUserTable, Box<HelperError>> {
        let value = serde_json::json!({
            "user_id": &self.id,
            "max_budget": &self.max_budget,
            "spend": &self.spend,
        });
        serde_json::from_value(value).map_err(|error| invalid(error.to_string()))
    }
}
