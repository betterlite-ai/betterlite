use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod entities;
mod gauges;
mod input;
mod lookups;
mod output;
mod probe;
mod support;
mod tests;

use input::Function;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::Team => entities::team::team_value(input),
            Function::Org => entities::org::org_value(input),
            Function::Customer => entities::customer::customer_value(input),
            Function::Key => entities::key::key_value(input),
            Function::User => entities::user::user_value(input),
            Function::DefaultCustomerBudget => entities::default_value(input),
        }
    }
}
