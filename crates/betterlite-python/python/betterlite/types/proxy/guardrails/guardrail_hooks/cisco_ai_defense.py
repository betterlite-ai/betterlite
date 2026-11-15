from typing import ClassVar, Literal, TypeAlias

from pydantic import ConfigDict, Field
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel
from betterlite.types.utils.base import ResponseObject

CISCO_AI_DEFENSE_RULE_NAMES: TypeAlias = Literal[
    "Code Detection",
    "Harassment",
    "Hate Speech",
    "PCI",
    "PHI",
    "PII",
    "Prompt Injection",
    "Profanity",
    "Sexual Content & Exploitation",
    "Social Division & Polarization",
    "Violence & Public Safety Threats",
]

# ast-grep-ignore: no-long-names
CISCO_AI_DEFENSE_INSPECTION_TYPE: TypeAlias = Literal["chat", "mcp"]


class CiscoAIDefenseRule(ResponseObject):
    rule_name: CISCO_AI_DEFENSE_RULE_NAMES
    entity_types: list[str] | None = None


# ast-grep-ignore: no-long-names
class CiscoAIDefenseGuardrailConfigModelOptionalParams(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")

    # ast-grep-ignore: no-long-names
    inspection_type: CISCO_AI_DEFENSE_INSPECTION_TYPE = "chat"
    inspect_path: str | None = None
    enabled_rules: list[CiscoAIDefenseRule] | None = None
    integration_profile_id: str | None = None
    integration_profile_version: str | None = None
    integration_tenant_id: str | None = None
    integration_type: str | None = None
    on_flagged_action: str | None = "block"
    fallback_on_error: Literal["allow", "block"] | None = "block"
    timeout: float | None = Field(default=10.0, ge=1.0, le=60.0)


# ast-grep-ignore: no-long-names
class CiscoAIDefenseGuardrailConfigModel(
    # ast-grep-ignore: no-long-names
    GuardrailConfigModel[CiscoAIDefenseGuardrailConfigModelOptionalParams],
):
    api_key: str | None = None
    api_base: str | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Cisco AI Defense"
