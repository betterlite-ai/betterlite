use betterlite_config::sections::settings::JsonValue;

use crate::router_runtime_settings::error::RouterSettingsError;

// Each step is one LiteLLM method that the router's owner runs over its live state
pub trait RouterSettingsTarget {
    fn set_optional_pre_call_checks(
        &mut self,
        checks: &JsonValue,
    ) -> Result<(), RouterSettingsError>;

    fn init_routing_strategy(
        &mut self,
        strategy: &JsonValue,
        args: &JsonValue,
    ) -> Result<(), RouterSettingsError>;

    fn apply_updated_strategy_args(&mut self) -> Result<(), RouterSettingsError>;

    fn apply_lar1(&mut self, args: Option<&JsonValue>) -> Result<(), RouterSettingsError>;

    fn init_routing_groups(&mut self, input: Option<&JsonValue>)
    -> Result<(), RouterSettingsError>;
}
