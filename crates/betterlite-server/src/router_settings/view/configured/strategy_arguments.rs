use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_config::sections::settings::JsonValue;
use betterlite_config::types::{Config, RoutingStrategy};

use crate::router_settings::view::number;

// the arguments the router keeps for the strategies betterlite models; any other strategy reads none
pub(super) fn strategy_args(config: &Config) -> BTreeMap<String, JsonValue> {
    let mut args = BTreeMap::new();
    match config.sections.router_core.routing_strategy {
        RoutingStrategy::LatencyBasedRouting => {
            let parsed = &config.sections.latency_observations;
            insert_number(&mut args, "ttl", parsed.ttl_seconds);
            insert_number(
                &mut args,
                "lowest_latency_buffer",
                parsed.lowest_latency_buffer,
            );
            if let Some(size) = parsed.max_latency_list_size {
                args.insert(String::from("max_latency_list_size"), JsonValue::from(size));
            }
            insert_number(&mut args, "ttft_percentile", parsed.ttft_percentile);
        }
        RoutingStrategy::UsageBasedRouting => {
            if let Some(ttl) = config.sections.usage_routing.ttl_seconds {
                args.insert(String::from("ttl"), JsonValue::from(ttl));
            }
        }
        RoutingStrategy::Lar1 => {
            let parsed = &config.sections.lar1_routing.args;
            if let Some(thresholds) = parsed {
                insert_decimal(&mut args, "confidence_threshold_low", thresholds.low);
                insert_decimal(&mut args, "confidence_threshold_medium", thresholds.medium);
                insert_decimal(&mut args, "confidence_threshold_high", thresholds.high);
            }
        }
        RoutingStrategy::SimpleShuffle
        | RoutingStrategy::LeastBusy
        | RoutingStrategy::CostBasedRouting
        | RoutingStrategy::UsageBasedRoutingV2 => {}
    }
    args
}

fn insert_number(args: &mut BTreeMap<String, JsonValue>, key: &str, value: Option<f64>) {
    if let Some(amount) = value {
        args.insert(String::from(key), number(amount));
    }
}

fn insert_decimal<T>(args: &mut BTreeMap<String, JsonValue>, key: &str, value: Option<T>)
where
    T: Display,
{
    if let Some(amount) = value {
        args.insert(String::from(key), decimal(amount));
    }
}

fn decimal<T>(value: T) -> JsonValue
where
    T: Display,
{
    value
        .to_string()
        .parse::<f64>()
        .map_or(JsonValue::Null, number)
}
