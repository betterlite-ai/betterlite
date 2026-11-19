use betterlite_config::sections::usage_routing as config;
use betterlite_router::strategy_dispatch::args::UsageArgs;

mod tests;

// Router.__init__ hands routing_strategy_args to RoutingArgs, whose ttl defaults to one minute when unset
#[must_use]
pub fn resolve(section: &config::Settings) -> UsageArgs {
    let declared = UsageArgs::default();
    UsageArgs {
        ttl_seconds: section.ttl_seconds.unwrap_or(declared.ttl_seconds),
    }
}
