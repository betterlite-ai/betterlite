use serde_json::{Map, Value};

use super::error::RouterSettingsError;
use super::integer::int_setting;
use super::retry_policy_value::retry_policy_value;
use super::target::RouterSettingsTarget;
use super::{RUNTIME_UPDATABLE_ROUTER_SETTINGS, RouterRuntimeSettings};

const INTEGER_SETTINGS: [&str; 5] = [
    "timeout",
    "num_retries",
    "retry_after",
    "allowed_fails",
    "cooldown_time",
];

// The strategy is compared with the value from before the first pair, not the one stored by an earlier pair
struct UpdatePass {
    previous_strategy: Option<Value>,
    rebuild_routing_groups: bool,
    routing_args_updated: bool,
}

fn change_strategy(
    target: &mut dyn RouterSettingsTarget,
    kwargs: &[(String, Value)],
    strategy: &Value,
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
        target.init_routing_strategy(strategy, args.unwrap_or(&Value::Object(Map::new())))?;
    }
    pass.rebuild_routing_groups = true;
    Ok(())
}

impl RouterRuntimeSettings {
    pub fn update_settings(
        &mut self,
        target: &mut dyn RouterSettingsTarget,
        kwargs: &[(String, Value)],
    ) -> Result<(), RouterSettingsError> {
        let mut pass = UpdatePass {
            previous_strategy: self.values.get("routing_strategy").cloned(),
            rebuild_routing_groups: false,
            routing_args_updated: false,
        };
        for (name, value) in kwargs {
            if RUNTIME_UPDATABLE_ROUTER_SETTINGS.contains(&name.as_str()) {
                self.apply_pair(target, kwargs, (name.as_str(), value), &mut pass)?;
            } else {
                tracing::debug!("Setting {name} is not allowed");
            }
        }
        if pass.routing_args_updated {
            target.apply_updated_strategy_args()?;
        }
        if pass.rebuild_routing_groups {
            let input = match kwargs.iter().find(|(name, _)| name == "routing_groups") {
                Some((_, groups)) if !groups.is_null() => Some(groups.clone()),
                Some(_) => None,
                None => self.routing_groups_input.clone(),
            };
            target.init_routing_groups(input.as_ref())?;
            self.routing_groups_input = input;
        }
        tracing::debug!("Updated Router settings: {:?}", self.values);
        Ok(())
    }

    fn apply_pair(
        &mut self,
        target: &mut dyn RouterSettingsTarget,
        kwargs: &[(String, Value)],
        (name, value): (&str, &Value),
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
                pass.routing_args_updated = self.values.get(name) != Some(value);
                value.clone()
            }
            integer if INTEGER_SETTINGS.contains(&integer) => int_setting(name, value)?,
            _ => value.clone(),
        };
        self.values.insert(name.to_owned(), stored);
        Ok(())
    }
}
