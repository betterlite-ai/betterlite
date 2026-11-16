use serde_json::Value;

use super::error::RouterSettingsError;

// Each step is one LiteLLM method that the router's owner runs over its live state
pub trait RouterSettingsTarget {
    fn set_optional_pre_call_checks(&mut self, checks: &Value) -> Result<(), RouterSettingsError>;

    fn init_routing_strategy(
        &mut self,
        strategy: &Value,
        args: &Value,
    ) -> Result<(), RouterSettingsError>;

    fn apply_updated_strategy_args(&mut self) -> Result<(), RouterSettingsError>;

    fn apply_lar1(&mut self, args: Option<&Value>) -> Result<(), RouterSettingsError>;

    fn init_routing_groups(&mut self, input: Option<&Value>) -> Result<(), RouterSettingsError>;
}
