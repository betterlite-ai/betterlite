mod tests;

use betterlite_core::cost::special::policy::SpecialPricePolicy;

use super::ProviderPricing;

// The search and Together formulas belong to later provider tasks, so every operation here still answers with its typed unavailable error.
impl SpecialPricePolicy for ProviderPricing {}
