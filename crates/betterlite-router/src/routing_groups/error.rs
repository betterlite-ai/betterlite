use crate::strategy_dispatch::error::StrategyDispatchError;

// Every way routing_groups validation stops, with the messages parse_routing_groups raises.
#[derive(Debug, thiserror::Error)]
pub enum RoutingGroupError {
    #[error("routing_groups: group_name must be non-empty.")]
    EmptyName,
    #[error("routing_groups: 'default' is reserved for the implicit fallback group.")]
    ReservedDefault,
    #[error("routing_groups: group names must be unique, duplicate group_name '{group}'.")]
    DuplicateName { group: String },
    #[error("routing_groups: {conflicts}. Each model may belong to at most one group.")]
    ModelOwnerConflict { conflicts: String },
    #[error("model_priorities requires routing_strategy='priority'")]
    PrioritiesRequirePriority,
    #[error("Priority routing groups require nonempty, distinct models")]
    PriorityModels,
    #[error("model_priorities must contain exactly the group's models")]
    PriorityMembership,
    #[error("Priority routing groups use model_priorities, not routing_strategy_args")]
    PriorityArgs,
    #[error("Priority routing group names must not shadow a model or alias")]
    PriorityShadow,
    #[error("routing_groups.{field} must be {expected}")]
    Field {
        field: &'static str,
        expected: &'static str,
    },
    #[error(
        "routing_groups.model_priorities['{model}'] must be an integer from 1 to 9007199254740991"
    )]
    PriorityValue { model: String },
    #[error(transparent)]
    Strategy(#[from] StrategyDispatchError),
}
