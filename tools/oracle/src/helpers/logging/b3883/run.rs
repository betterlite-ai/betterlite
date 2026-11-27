use betterlite_oracle_helper_api::helper::HelperResult;

use super::entities::{
    customer_value, default_value, key_value, org_value, team_value, user_value,
};
use super::input::Input;

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Team { .. } => team_value(input),
        Input::Org { .. } => org_value(input),
        Input::Customer { .. } => customer_value(input),
        Input::Key { .. } => key_value(input),
        Input::User { .. } => user_value(input),
        Input::DefaultCustomerBudget { .. } => default_value(input),
    }
}
