use betterlite_config::sections::alert_settings::types::AlertType;
use rust_decimal::Decimal;
use strum::{EnumCount as _, IntoEnumIterator as _};

pub mod cache_keys;
pub mod records;
pub mod trace;

mod tests;

pub const SPEND_THRESHOLD_5_PERCENT: Decimal = Decimal::from_parts(5, 0, 0, false, 2);
pub const SPEND_THRESHOLD_15_PERCENT: Decimal = Decimal::from_parts(15, 0, 0, false, 2);
pub const MAX_OLDEST_HANGING_REQUESTS: usize = 20;
pub const HANGING_ALERT_BUFFER_SECONDS: u64 = 60;
pub const LITELLM_LOGO_URL: &str = "https://litellm-listing.s3.amazonaws.com/litellm_logo.png";
pub const LITELLM_SUPPORT_CONTACT: &str = "support@berri.ai";

// LiteLLM's SlackAlerting._all_possible_alert_types feeds the UI, so every enum member appears here
#[must_use]
pub fn all_possible_alert_types() -> Vec<AlertType> {
    let mut alert_types = Vec::with_capacity(AlertType::COUNT);
    alert_types.extend(AlertType::iter());
    alert_types
}
