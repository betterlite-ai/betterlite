use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::cli_context::CliContext;
use crate::token_records::record::CliTokenRecord;

pub type CliTokenData = CliTokenRecord;

pub type CliContextObj = CliContext;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CliTeam {
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub models: Vec<String>,
    pub max_budget: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct CliPollData {
    pub status: Option<String>,
    pub key: Option<SecretString>,
    pub user_id: Option<String>,
    pub teams: Vec<String>,
    // LiteLLM declares team_details as object and only its normalizer reads it, so it stays unparsed here.
    pub team_details: Option<Box<RawValue>>,
    pub requires_team_selection: bool,
    pub team_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CliSsoStartData {
    pub login_id: String,
    pub poll_secret: SecretString,
    pub user_code: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CliAuthResult {
    pub api_key: SecretString,
    pub user_id: Option<String>,
    pub teams: Vec<String>,
    pub team_id: Option<String>,
}
