use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::resolved_key::self_access::allowed_inside_route;
use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    user_role: Option<String>,
    user_id: Option<String>,
    requested_user_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    rows: Vec<Row>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answers = Vec::with_capacity(input.rows.len());
        for row in &input.rows {
            let mut fields = BTreeMap::new();
            if let Some(role) = &row.user_role {
                fields.insert("user_role".to_owned(), JsonData::String(role.clone()));
            }
            if let Some(user_id) = &row.user_id {
                fields.insert("user_id".to_owned(), JsonData::String(user_id.clone()));
            }
            let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
            answers.push(allowed_inside_route(
                &auth,
                row.requested_user_id.as_deref(),
            ));
        }
        Ok(serde_json::to_value(&answers).or_value_error()?)
    }
}
