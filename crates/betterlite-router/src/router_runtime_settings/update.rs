use std::iter;

use betterlite_config::sections::settings::JsonValue;

use crate::router_runtime_settings::RouterRuntimeSettings;
use crate::router_runtime_settings::error::RouterSettingsError;
use crate::router_runtime_settings::integer::int_setting;
use crate::router_runtime_settings::retry_policy_value::retry_policy_value;
use crate::router_runtime_settings::target::RouterSettingsTarget;

const INTEGER_SETTINGS: [&str; 5] = [
    "timeout",
    "num_retries",
    "retry_after",
    "allowed_fails",
    "cooldown_time",
];

// The strategy is compared with the value from before the first pair, not the one stored by an earlier pair
pub(super) struct UpdatePass {
    pub(super) previous_strategy: Option<JsonValue>,
    pub(super) rebuild_routing_groups: bool,
    pub(super) routing_args_updated: bool,
}

pub(super) fn apply_pair(
    settings: &mut RouterRuntimeSettings,
    target: &mut dyn RouterSettingsTarget,
    kwargs: &[(String, JsonValue)],
    (name, value): (&str, &JsonValue),
    pass: &mut UpdatePass,
) -> Result<(), RouterSettingsError> {
    let stored = match name {
        "routing_groups" => {
            pass.rebuild_routing_groups = true;
            return Ok(());
        }
        "optional_pre_call_checks" => return target.set_optional_pre_call_checks(value),
        "retry_policy" => match retry_policy_value(value)? {
            Some(policy) => policy,
            None => return Ok(()),
        },
        "routing_strategy" => {
            change_strategy(target, kwargs, value, pass)?;
            value.clone()
        }
        "routing_strategy_args" => {
            pass.routing_args_updated = settings.values.get(name) != Some(value);
            value.clone()
        }
        integer if INTEGER_SETTINGS.contains(&integer) => int_setting(name, value)?,
        _ => value.clone(),
    };
    settings.values.insert(name.to_owned(), stored);
    Ok(())
}

fn change_strategy(
    target: &mut dyn RouterSettingsTarget,
    kwargs: &[(String, JsonValue)],
    strategy: &JsonValue,
    pass: &mut UpdatePass,
) -> Result<(), RouterSettingsError> {
    if pass.previous_strategy.as_ref() == Some(strategy) {
        return Ok(());
    }
    let args = kwargs
        .iter()
        .find(|(name, _)| name == "routing_strategy_args")
        .map(|(_, args)| args);
    if strategy.as_str() == Some("lar1") {
        target.apply_lar1(args.filter(|candidate| !candidate.is_null()))?;
    } else {
        let empty = JsonValue::Object(iter::empty().collect());
        target.init_routing_strategy(strategy, args.unwrap_or(&empty))?;
    }
    pass.rebuild_routing_groups = true;
    Ok(())
}
