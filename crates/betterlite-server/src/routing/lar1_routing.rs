use betterlite_config::sections::lar1_routing as config;
use betterlite_router::lar1_routing::strategy::Lar1Strategy;
use betterlite_router::lar1_routing::thresholds::{ThresholdArgs, ThresholdError};

mod tests;

// Router.__init__ builds the strategy from routing_strategy_args, so an unordered set fails startup
pub fn resolve(section: &config::Settings) -> Result<Lar1Strategy, ThresholdError> {
    let args = section.args.as_ref().map(|configured| ThresholdArgs {
        low: configured.low,
        medium: configured.medium,
        high: configured.high,
    });
    Lar1Strategy::new(args.as_ref())
}
