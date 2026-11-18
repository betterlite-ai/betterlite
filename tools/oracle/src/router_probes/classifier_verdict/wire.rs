use std::collections::BTreeMap;

use betterlite_config::sections::tier_definitions as section;
use betterlite_models::decisions::Json;
use betterlite_router::classifier_verdict::rubric::ClassificationRubric;
use betterlite_router::complexity_scoring::tier::ComplexityTier;
use betterlite_router::tier_definitions::checks::ClassifierKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct Payload {
    pub(super) function: Function,
    pub(super) rubric: Option<ClassificationRubric>,
    pub(super) labeled_tiers: Option<Vec<LabeledTier>>,
    pub(super) window: Option<u64>,
    pub(super) prompt: Option<String>,
    pub(super) examples: Option<String>,
    pub(super) definitions: Option<Vec<section::TierDefinition>>,
    pub(super) tier_config: Option<TierConfigInput>,
    pub(super) reply: Option<Reply>,
    pub(super) request_kwargs: Option<Json>,
    pub(super) label: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Function {
    #[default]
    SystemPrompt,
    BuiltInPrompt,
    CustomPrompt,
    CalibrationExamples,
    ResponseFormat,
    ResolveLabel,
    ParseReply,
    LoggingFlag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LabeledTier {
    pub(super) tier: ComplexityTier,
    pub(super) label: String,
}

// the tier section's typed view plus the two checks a custom tier set's validation reads
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct TierConfigInput {
    pub(super) tier_definitions: Option<Vec<section::TierDefinition>>,
    #[serde(alias = "enable_non_reasoning_tier")]
    pub(super) non_reasoning_tier: bool,
    pub(super) tier_labels: BTreeMap<String, String>,
    #[serde(alias = "tiers")]
    pub(super) tier_models: Option<BTreeMap<String, section::TierPool>>,
    pub(super) classifier_type: ClassifierKind,
    pub(super) fallback_tier: Option<String>,
    // the section does not read this, but the same document configures the reference router
    pub(super) classifier_llm_config: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Reply {
    Text {
        content: String,
    },
    Padded {
        content: String,
        pad: String,
        pad_len: u64,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Fields {
    pub(super) prompt: Option<String>,
    pub(super) examples: Option<String>,
    pub(super) resolved: Option<String>,
    pub(super) response_format: Option<String>,
    pub(super) parse: Option<ParseOutcome>,
    pub(super) logging_flag: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ParseOutcome {
    Selected {
        label: String,
        tier: String,
    },
    Rejected {
        reason: RejectReason,
        withheld: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RejectReason {
    Schema,
    UnknownTier,
    TooLarge,
}
