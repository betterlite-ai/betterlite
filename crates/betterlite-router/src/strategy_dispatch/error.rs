use crate::simple_shuffle::error::ShuffleError;
use crate::strategy_dispatch::selector::SelectorId;
use crate::strategy_dispatch::strategy::RoutingStrategy;

#[derive(Debug, thiserror::Error)]
pub enum StrategyDispatchError {
    #[error(
        "Invalid routing_strategy: '{value}'. Valid options: {valid}. Check 'router_settings.routing_strategy' in your config.yaml or the 'routing_strategy' parameter if using the Router SDK directly."
    )]
    InvalidRoutingStrategy { value: String, valid: &'static str },
    #[error("routing_strategy_args for '{strategy:?}' must be {expected}, not {supplied}")]
    ArgsMismatch {
        strategy: RoutingStrategy,
        expected: &'static str,
        supplied: &'static str,
    },
    #[error("selector {id:?} is not known to this router")]
    UnknownSelector { id: SelectorId },
    #[error("routing group '{group}' is declared more than once")]
    DuplicateGroup { group: String },
    #[error("model '{model}' is claimed by routing groups '{first}' and '{second}'")]
    DuplicateGroupModel {
        model: String,
        first: String,
        second: String,
    },
    #[error("the deployments cannot be weighed for a simple-shuffle pick")]
    Shuffle {
        #[from]
        source: ShuffleError,
    },
}
