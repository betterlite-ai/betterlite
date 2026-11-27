pub(crate) mod customer;
pub(crate) mod key;
pub(crate) mod org;
pub(crate) mod team;
pub(crate) mod user;

use betterlite_logging::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use betterlite_logging::prometheus::budget_lookups::customer_default::get_default_customer_budget;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::Input;
use super::lookups::ScriptedLookups;
use super::output::budget_output;
use super::support::runtime;

pub(crate) fn default_value(input: &Input) -> HelperResult {
    let scripted = ScriptedLookups::new(input.lookup.clone(), true);
    let row: Option<BudgetTableSnapshot> = runtime()?.block_on(get_default_customer_budget(
        &scripted,
        input.default_budget_id.as_deref(),
    ));
    Ok(budget_output(row, &scripted.calls()))
}
