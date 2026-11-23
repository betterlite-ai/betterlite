use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::JsonData;

use crate::state::shared::Shared;

// the section default, for a request that reaches the stage without a loaded config
const DEFAULT_ALERT_FRACTION: f64 = 0.8;

// the two pinned settings the key budget stage reads: the global threshold map and the legacy alert fraction
#[derive(Debug)]
pub struct KeyBudgetSettings {
    pub global_alert_emails: Option<BTreeMap<String, JsonData>>,
    pub alert_fraction: f64,
}

impl Default for KeyBudgetSettings {
    fn default() -> Self {
        Self {
            global_alert_emails: None,
            alert_fraction: DEFAULT_ALERT_FRACTION,
        }
    }
}

pub struct Slot {
    pub settings: Arc<KeyBudgetSettings>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        let section = &config.sections.auth_key_budgets;
        Self {
            settings: Arc::new(KeyBudgetSettings {
                global_alert_emails: section.default_alert_emails.clone(),
                alert_fraction: section.alert_fraction,
            }),
        }
    }
}
