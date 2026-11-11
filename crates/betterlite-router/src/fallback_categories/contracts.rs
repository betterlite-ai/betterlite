use crate::weighted_failover::{FailoverEntry, FailoverInputs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackFailure {
    ContextWindow,
    ContentPolicy,
    Ordinary,
}

impl FallbackFailure {
    #[must_use]
    pub const fn skips_order_fallback(self) -> bool {
        match self {
            Self::ContextWindow | Self::ContentPolicy => true,
            Self::Ordinary => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackTarget {
    pub model: String,
    pub target_order: Option<i64>,
    pub excluded_deployment_ids: Vec<String>,
}

impl From<FailoverEntry> for FallbackTarget {
    fn from(entry: FailoverEntry) -> Self {
        Self {
            model: entry.model,
            target_order: None,
            excluded_deployment_ids: entry.excluded_deployment_ids,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackChain<'plan> {
    Absent,
    Unresolved,
    Resolved {
        targets: &'plan [FallbackTarget],
        has_unattempted: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrdinaryFallbacks<'plan> {
    Absent,
    NonStandard(FallbackChain<'plan>),
    Standard(FallbackChain<'plan>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderExternal<'plan> {
    Absent,
    Targets(&'plan [FallbackTarget]),
    ClientFallbacks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderFallbacks<'plan> {
    pub model_group: &'plan str,
    pub deployment_orders: &'plan [Option<i64>],
    pub current_target: Option<i64>,
    pub external: OrderExternal<'plan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataBucket {
    Metadata,
    LitellmMetadata,
}

impl MetadataBucket {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::LitellmMetadata => "litellm_metadata",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WeightedFailover<'plan> {
    pub enabled: bool,
    pub inputs: FailoverInputs<'plan>,
    pub metadata_key: MetadataBucket,
    pub prior_excluded_ids: &'plan [String],
}

#[derive(Debug, Clone, Copy)]
pub struct SuccessorRequest<'plan> {
    pub failure: FallbackFailure,
    pub original_model_group: Option<&'plan str>,
    pub disable_fallbacks: bool,
    pub guardrail_intervention: bool,
    pub order: OrderFallbacks<'plan>,
    pub weighted_failover: WeightedFailover<'plan>,
    pub context_window: FallbackChain<'plan>,
    pub content_policy: FallbackChain<'plan>,
    pub ordinary: OrdinaryFallbacks<'plan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    FallbacksDisabled,
    MissingModelGroup,
    GuardrailIntervention,
    NoContextWindowChain,
    NoContentPolicyChain,
    NoFallbackChain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetSource {
    ContextWindow,
    ContentPolicy,
    Ordinary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FallbackSuccessor {
    Stop(StopReason),
    RunOrder {
        targets: Vec<FallbackTarget>,
        client_fallbacks_after: bool,
    },
    RunClientFallbacks,
    RunTargets {
        source: TargetSource,
        targets: Vec<FallbackTarget>,
    },
    // the same-group retry also tells the caller which ids to stamp into the metadata bucket
    RunWeightedFailover {
        targets: Vec<FallbackTarget>,
        excluded_deployment_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefusalRequest<'plan> {
    pub fallbacks_disabled: bool,
    pub content_policy: FallbackChain<'plan>,
    pub ordinary: &'plan OrdinaryFallbacks<'plan>,
    pub router_fallbacks: &'plan [&'plan str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentPolicyResponse<'response> {
    pub has_choices: bool,
    pub first_finish_reason: Option<&'response str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalResponse {
    NotRefusal,
    RefusalWithoutStopDetails,
    RefusalWithStopDetails,
}
