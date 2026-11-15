use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{Usage as PricingUsage, UsageNumber};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use super::UsageConversionError;

// The cost extension becomes the stated cost; every other extension keeps its JSON kind.
pub(super) fn decoded(
    usage: &PricingUsage,
) -> Result<(Option<Decimal>, IndexMap<String, Json>), UsageConversionError> {
    let mut cost = None;
    let mut extras = IndexMap::new();
    for (name, number) in &usage.numeric_extras {
        match number {
            UsageNumber::Decimal(found) if name == "cost" => cost = Some(*found),
            UsageNumber::Decimal(found) => {
                extras.insert(name.clone(), Json::Number(found.to_string().parse()?));
            }
            UsageNumber::Integer(found) => {
                extras.insert(name.clone(), Json::Number(Number::from(*found)));
            }
            UsageNumber::Boolean(flag) => {
                extras.insert(name.clone(), Json::Bool(*flag));
            }
        }
    }
    Ok((cost, extras))
}
