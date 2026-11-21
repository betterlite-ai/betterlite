use chrono::NaiveTime;

// B2317 parses litellm_settings into its own type; the db crate cannot depend on betterlite-config, so the composition root passes this mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetResetSettings {
    pub timezone: String,
    pub reset_time_of_day: NaiveTime,
}

impl Default for BudgetResetSettings {
    fn default() -> Self {
        Self {
            timezone: "UTC".to_owned(),
            reset_time_of_day: NaiveTime::MIN,
        }
    }
}

pub trait ResetSettingsSource {
    fn budget_reset_settings(&self) -> BudgetResetSettings;
}
