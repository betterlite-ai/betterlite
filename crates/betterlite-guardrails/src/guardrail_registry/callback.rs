use crate::guardrail_base::GuardrailBase;
use crate::guardrail_initializers::bedrock::BedrockArgs;
use crate::guardrail_initializers::lakera::LakeraArgs;
use crate::guardrail_initializers::lakera_v2::LakeraV2Args;
use crate::guardrail_initializers::presidio::PresidioArgs;
use crate::guardrail_initializers::tool_permission::ToolPermissionArgs;
use crate::tool_policy::ToolPolicyGuardrail;

use super::integrations::CompressionKind;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CallbackScoping {
    pub skip_system_message: Option<bool>,
    pub skip_tool_message: Option<bool>,
    pub scan_only_tool_results: Option<bool>,
}

pub struct GuardrailCallback {
    pub base: GuardrailBase,
    pub spec: CallbackSpec,
    pub scoping: CallbackScoping,
}

pub enum CallbackSpec {
    Bedrock(BedrockArgs),
    Lakera(LakeraArgs),
    LakeraV2(LakeraV2Args),
    Presidio(PresidioArgs),
    ToolPermission(ToolPermissionArgs),
    ToolPolicy(ToolPolicyGuardrail),
    Compression(CompressionKind),
}

impl GuardrailCallback {
    #[must_use]
    pub fn is_compression(&self) -> bool {
        matches!(
            self.spec,
            // the registry overwrites the spec of a headroom, compresr or typesafe callback
            CallbackSpec::Compression(_)
        )
    }

    // Prisma AIRS and Prompt Security callbacks also return false, but their specs are B8521's
    #[must_use]
    pub fn supports_tool_result_scan(&self) -> bool {
        match &self.spec {
            CallbackSpec::Bedrock(args) => !args.latest_role_message_only,
            CallbackSpec::Lakera(_)
            | CallbackSpec::LakeraV2(_)
            | CallbackSpec::Presidio(_)
            | CallbackSpec::ToolPermission(_)
            | CallbackSpec::ToolPolicy(_)
            | CallbackSpec::Compression(_) => true,
        }
    }
}
