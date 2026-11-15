from typing import Final, Literal

from pydantic import BaseModel, Field
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel

# ast-grep-ignore: no-long-names
XECGUARD_DEFAULT_POLICY_OPTIONS: Final = [
    "Default_Policy_SystemPromptEnforcement",
    "Default_Policy_GeneralPromptAttackProtection",
    "Default_Policy_ContentBiasProtection",
    "Default_Policy_HarmfulContentProtection",
    "Default_Policy_SkillsProtection",
    "Default_Policy_PIISensitiveDataProtection",
]


class XecGuardConfigModel(GuardrailConfigModel[BaseModel]):
    api_key: str | None = None
    api_base: str | None = None
    xecguard_model: str | None = None
    policy_names: list[str] | None = Field(
        default=None,
        json_schema_extra={
            "ui_type": "multiselect",
            # ast-grep-ignore: no-long-names
            "options": list(XECGUARD_DEFAULT_POLICY_OPTIONS),
        },
    )
    block_on_error: bool | None = None
    grounding_strictness: Literal["BALANCED", "STRICT"] | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "XecGuard"
