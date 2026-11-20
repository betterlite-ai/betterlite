use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::wire;

const EMPTY: &[GuardrailEventHooks] = &[];

const PRE_ONLY: &[GuardrailEventHooks] = &[GuardrailEventHooks::PreCall];

const PRE_MCP_ONLY: &[GuardrailEventHooks] = &[GuardrailEventHooks::PreMcpCall];

const PRE_POST: &[GuardrailEventHooks] =
    &[GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall];

const PRE_DURING: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
];

const DURING_POST: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
];

const PRE_DURING_POST: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
];

const PRE_POST_DURING: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::DuringCall,
];

const PRE_POST_LOGGING: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::LoggingOnly,
];

const GUARDRAILS_AI_HOOKS: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::LoggingOnly,
];

const CISCO_HOOKS: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::LoggingOnly,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::DuringMcpCall,
];

const FULL_LIFECYCLE: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::DuringMcpCall,
    GuardrailEventHooks::LoggingOnly,
];

const CONTENT_FILTER_HOOKS: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::RealtimeInputTranscription,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::PostMcpCall,
];

const NOMA_HOOKS: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::PreMcpCall,
];

const MCP_LIFECYCLE: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::DuringMcpCall,
];

const PANW_HOOKS: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::LoggingOnly,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::DuringMcpCall,
    GuardrailEventHooks::PostMcpCall,
];

const SINGULR_HOOKS: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::LoggingOnly,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::PostMcpCall,
];

const XECGUARD_HOOKS: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::LoggingOnly,
];

// Each row is the class's own get_supported_event_hooks, in LiteLLM's order
const SUPPORTED: &[(&str, &[GuardrailEventHooks])] = &[
    (wire::AGENT_365, PRE_MCP_ONLY),
    (wire::AIM, PRE_DURING_POST),
    (wire::AKTO, PRE_POST),
    (wire::ALICE, PRE_DURING_POST),
    (wire::APORIA, DURING_POST),
    (wire::AZURE_PROMPT_SHIELD, PRE_DURING),
    (wire::AZURE_TEXT_MODERATIONS, PRE_POST),
    (wire::BLOCK_CODE_EXECUTION, PRE_POST_DURING),
    (wire::CATO_NETWORKS, PRE_DURING_POST),
    (wire::CISCO_AI_DEFENSE, CISCO_HOOKS),
    (wire::COMPRESR, EMPTY),
    (wire::CONDUCT, EMPTY),
    (wire::CROWDSTRIKE_AIDR, PRE_POST),
    (wire::CUSTOM_CODE, FULL_LIFECYCLE),
    (wire::DEEP_KEEP, PRE_POST_DURING),
    (wire::DYNAMOAI, PRE_POST_DURING),
    (wire::ENKRYPTAI, PRE_POST_DURING),
    (wire::GENERIC_GUARDRAIL_API, PRE_POST_DURING),
    (wire::GRAYSWAN, PRE_DURING_POST),
    (wire::GUARDRAILS_AI, GUARDRAILS_AI_HOOKS),
    (wire::HEADROOM, PRE_POST),
    (wire::HIDDENLAYER, PRE_POST),
    (wire::IBM_GUARDRAILS, PRE_POST_DURING),
    (wire::JAVELIN, PRE_ONLY),
    (wire::LASSO, PRE_DURING_POST),
    (wire::LITELLM_CONTENT_FILTER, CONTENT_FILTER_HOOKS),
    (wire::LLM_AS_A_JUDGE, PRE_DURING_POST),
    (wire::MCP_JWT_SIGNER, PRE_MCP_ONLY),
    (wire::MICROSOFT_PURVIEW, PRE_POST_LOGGING),
    (wire::MODEL_ARMOR, FULL_LIFECYCLE),
    (wire::NOMA, NOMA_HOOKS),
    (wire::NOMA_V2, MCP_LIFECYCLE),
    (wire::ONYX, PRE_DURING_POST),
    (wire::OPENAI_MODERATION, PRE_DURING_POST),
    (wire::OVALIX, PRE_POST),
    (wire::PANGEA, PRE_POST),
    (wire::PANW_PRISMA_AIRS, PANW_HOOKS),
    (wire::PILLAR, MCP_LIFECYCLE),
    (wire::PROMPTGUARD, PRE_POST),
    (wire::QOSTODIAN_NEXUS, PRE_POST_DURING),
    (wire::QUALIFIRE, PRE_DURING_POST),
    (wire::REPELLOAI, PRE_POST),
    (wire::RUBRIK, PRE_POST),
    (wire::SEMANTIC_GUARD, PRE_POST),
    (wire::SINGULR, SINGULR_HOOKS),
    (wire::STRAIKER, PRE_POST),
    (wire::TYPESAFE, EMPTY),
    (wire::VIGIL_GUARD, PRE_POST),
    (wire::XECGUARD, XECGUARD_HOOKS),
    (wire::ZSCALER_AI_GUARD, PRE_POST),
];

#[must_use]
pub fn supported_event_hooks(kind: &str) -> &'static [GuardrailEventHooks] {
    SUPPORTED
        .iter()
        .find_map(|(name, hooks)| (*name == kind).then_some(*hooks))
        .unwrap_or_default()
}
