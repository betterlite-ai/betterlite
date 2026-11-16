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
    pub const fn is_compression(&self) -> bool {
        matches!(
            self.spec,
            // the registry overwrites the spec of a headroom, compresr or typesafe callback
            CallbackSpec::Compression(_)
        )
    }

    // Prisma AIRS and Prompt Security callbacks also return false, but their specs are B8521's
    #[must_use]
    // ast-grep-ignore: no-long-names
    pub const fn supports_scan_only_tool_results(&self) -> bool {
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

impl From<BedrockArgs> for CallbackSpec {
    fn from(args: BedrockArgs) -> Self {
        Self::Bedrock(args)
    }
}

impl From<LakeraArgs> for CallbackSpec {
    fn from(args: LakeraArgs) -> Self {
        Self::Lakera(args)
    }
}

impl From<LakeraV2Args> for CallbackSpec {
    fn from(args: LakeraV2Args) -> Self {
        Self::LakeraV2(args)
    }
}

impl From<PresidioArgs> for CallbackSpec {
    fn from(args: PresidioArgs) -> Self {
        Self::Presidio(args)
    }
}

impl From<ToolPermissionArgs> for CallbackSpec {
    fn from(args: ToolPermissionArgs) -> Self {
        Self::ToolPermission(args)
    }
}
