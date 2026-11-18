#![cfg(test)]

use betterlite_config::sections::usage_routing as config;
use betterlite_router::strategy_dispatch::args::UsageArgs;

use super::resolve;

#[test]
fn usage_routing_resolve_keeps_the_declared_minute() {
    let args = resolve(&config::Settings::default());
    assert_eq!(args.ttl_seconds, 60);
    assert_eq!(args, UsageArgs::default());
}

#[test]
fn usage_routing_resolve_uses_the_configured_ttl() {
    let args = resolve(&config::Settings {
        ttl_seconds: Some(45),
    });
    assert_eq!(args.ttl_seconds, 45);
}

// an explicit zero is a ttl of its own, not a reason to fall back to the declared minute
#[test]
fn usage_routing_resolve_keeps_an_explicit_zero() {
    let args = resolve(&config::Settings {
        ttl_seconds: Some(0),
    });
    assert_eq!(args.ttl_seconds, 0);
}
