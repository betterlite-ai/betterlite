use betterlite_auth::end_user_registry::predicate::{column_is_set, restricted_end_user_where};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ColumnIsSet { column: String },
    RestrictedEndUserWhere,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let filter = match input {
            Input::ColumnIsSet { column } => column_is_set(column),
            Input::RestrictedEndUserWhere => restricted_end_user_where(),
        };
        Ok(serde_json::to_value(filter)?)
    }
}

#[cfg(test)]
mod tests;
