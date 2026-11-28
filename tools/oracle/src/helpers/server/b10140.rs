use betterlite_db::team_rows::member::Member;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::team_membership_effects::audit::members_audit_value;
use serde::Deserialize;
use serde_json::Value;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    team_alias: Option<String>,
    members: Vec<Member>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let text =
            members_audit_value(input.team_alias.as_deref(), &input.members).or_value_error()?;
        let value: Value = serde_json::from_str(&text)?;
        Ok(value)
    }
}
