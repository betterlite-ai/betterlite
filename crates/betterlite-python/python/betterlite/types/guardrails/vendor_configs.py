from collections.abc import Mapping
from typing import ClassVar, Final, Literal, Self, TypeAlias

from pydantic import ConfigDict, Field, JsonValue, model_validator

from betterlite.types.utils.base import ResponseObject

# ast-grep-ignore: no-long-names
BEDROCK_APPLY_GUARDRAIL_CHUNK_BUDGET_CHARS: Final = 25_000

# ast-grep-ignore: no-long-names
BedrockChecksContentFilterCategory: TypeAlias = Literal[
    "VIOLENCE", "HATE", "SEXUAL", "MISCONDUCT", "INSULTS",
]
# ast-grep-ignore: no-long-names
BedrockChecksPromptAttackCategory: TypeAlias = Literal[
    "JAILBREAK", "PROMPT_INJECTION", "PROMPT_LEAKAGE",
]
# ast-grep-ignore: no-long-names
BedrockChecksSensitiveInformationEntity: TypeAlias = Literal[
    "ADDRESS", "AGE", "AWS_ACCESS_KEY", "AWS_SECRET_KEY", "CA_HEALTH_NUMBER",
    "CA_SOCIAL_INSURANCE_NUMBER", "CREDIT_DEBIT_CARD_CVV", "CREDIT_DEBIT_CARD_EXPIRY",
    "CREDIT_DEBIT_CARD_NUMBER", "DRIVER_ID", "EMAIL", "INTERNATIONAL_BANK_ACCOUNT_NUMBER",
    "IP_ADDRESS", "LICENSE_PLATE", "MAC_ADDRESS", "NAME", "PASSWORD", "PHONE", "PIN",
    "SWIFT_CODE", "UK_NATIONAL_HEALTH_SERVICE_NUMBER", "UK_NATIONAL_INSURANCE_NUMBER",
    "UK_UNIQUE_TAXPAYER_REFERENCE_NUMBER", "URL", "USERNAME", "US_BANK_ACCOUNT_NUMBER",
    "US_BANK_ROUTING_NUMBER", "US_INDIVIDUAL_TAX_IDENTIFICATION_NUMBER",
    "US_PASSPORT_NUMBER", "US_SOCIAL_SECURITY_NUMBER", "VEHICLE_IDENTIFICATION_NUMBER",
]


# ast-grep-ignore: no-long-names
class BedrockChecksContentFilterCategoryItem(ResponseObject):
    # ast-grep-ignore: no-long-names
    category: BedrockChecksContentFilterCategory


# ast-grep-ignore: no-long-names
class BedrockChecksContentFilterModel(ResponseObject):
    # ast-grep-ignore: no-long-names
    categories: list[BedrockChecksContentFilterCategoryItem]


# ast-grep-ignore: no-long-names
class BedrockChecksPromptAttackCategoryItem(ResponseObject):
    # ast-grep-ignore: no-long-names
    category: BedrockChecksPromptAttackCategory


class BedrockChecksPromptAttackModel(ResponseObject):
    # ast-grep-ignore: no-long-names
    categories: list[BedrockChecksPromptAttackCategoryItem]


# ast-grep-ignore: no-long-names
class BedrockChecksSensitiveInformationEntityItem(ResponseObject):
    # ast-grep-ignore: no-long-names
    type: BedrockChecksSensitiveInformationEntity


# ast-grep-ignore: no-long-names
class BedrockChecksSensitiveInformationModel(ResponseObject):
    # ast-grep-ignore: no-long-names
    entities: list[BedrockChecksSensitiveInformationEntityItem]


class BedrockChecksConfigModel(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", serialize_by_alias=True)

    # ast-grep-ignore: no-long-names
    content_filter: BedrockChecksContentFilterModel | None = Field(
        default=None,
        alias="contentFilter",
    )
    prompt_attack: BedrockChecksPromptAttackModel | None = Field(
        default=None,
        alias="promptAttack",
    )
    # ast-grep-ignore: no-long-names
    sensitive_information: BedrockChecksSensitiveInformationModel | None = Field(
        default=None,
        alias="sensitiveInformation",
    )

    @model_validator(mode="after")
    def _require_at_least_one_check(self) -> Self:
        if (
            self.content_filter is None
            and self.prompt_attack is None
            and self.sensitive_information is None
        ):
            raise ValueError(
                "Bedrock 'checks' must enable at least one of: "
                "contentFilter, promptAttack, sensitiveInformation.",
            )
        return self


class BedrockGuardrailConfigModel(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", serialize_by_alias=True)

    guardrail_identifier: str | None = Field(default=None, alias="guardrailIdentifier")
    guardrail_version: str | None = Field(default=None, alias="guardrailVersion")
    disable_exception_on_block: bool | None = False
    aws_region_name: str | None = None
    aws_access_key_id: str | None = None
    aws_secret_access_key: str | None = None
    aws_session_token: str | None = None
    aws_session_name: str | None = None
    aws_profile_name: str | None = None
    aws_role_name: str | None = None
    aws_web_identity_token: str | None = None
    aws_sts_endpoint: str | None = None
    aws_external_id: str | None = None
    aws_bedrock_runtime_endpoint: str | None = None
    checks: BedrockChecksConfigModel | None = None
    content_filter_threshold: float | None = Field(default=0.5, ge=0.0, le=1.0)
    prompt_attack_threshold: float | None = Field(default=0.5, ge=0.0, le=1.0)
    pii_confidence_threshold: float | None = Field(default=0.5, ge=0.0, le=1.0)
    # ast-grep-ignore: no-long-names
    chunk_budget_chars: int = Field(default=BEDROCK_APPLY_GUARDRAIL_CHUNK_BUDGET_CHARS, gt=0)
    # ast-grep-ignore: no-long-names
    contextual_grounding_from_messages: bool = False


# ast-grep-ignore: no-long-names
class BedrockGuardrailStreamingParams(ResponseObject):
    # ast-grep-ignore: no-long-names
    streaming_buffer_until_moderated: bool = True
    streaming_sampling_rate: int = Field(default=5, ge=1)
    streaming_end_of_stream_only: bool = False
    # ast-grep-ignore: no-long-names
    streaming_buffer_release_on_scan: bool = False

    @classmethod
    def from_extras(cls, extras: Mapping[str, JsonValue] | None) -> Self:
        if not extras:
            return cls()
        known: dict[str, JsonValue] = {
            name: extras[name]
            for name in cls.model_fields
            if extras.get(name) is not None
        }
        return cls.model_validate(known)


class LakeraV2GuardrailConfigModel(ResponseObject):
    api_key: str | None = None
    api_base: str | None = None
    project_id: str | None = None
    payload: bool | None = True
    breakdown: bool | None = True
    metadata: dict[str, JsonValue] | None = None
    dev_info: bool | None = True
    on_flagged: Literal["block", "monitor", "inject_system_message"] | None = "block"
    advisory_system_message: str | None = None


class LassoGuardrailConfigModel(ResponseObject):
    lasso_user_id: str | None = None
    lasso_conversation_id: str | None = None
    mask: bool | None = False


class DeepKeepGuardrailConfigModel(ResponseObject):
    deepkeep_firewall_id: str | None = None


class PillarGuardrailConfigModel(ResponseObject):
    on_flagged_action: str | None = "monitor"
    async_mode: bool | None = None
    persist_session: bool | None = None
    include_scanners: bool | None = True
    include_evidence: bool | None = True


class NomaGuardrailConfigModel(ResponseObject):
    use_v2: bool | None = False
    application_id: str | None = None
    gateway_name: str | None = None
    monitor_mode: bool | None = None
    block_failures: bool | None = None
    anonymize_input: bool | None = None


class ZscalerAIGuardConfigModel(ResponseObject):
    policy_id: int | None = None
    send_user_api_key_alias: bool | None = False
    send_user_api_key_user_id: bool | None = False
    send_user_api_key_team_id: bool | None = False


class JavelinGuardrailConfigModel(ResponseObject):
    guard_name: str | None = None
    api_version: str | None = None
    metadata: dict[str, JsonValue] | None = None
    application: str | None = None
    config: dict[str, JsonValue] | None = None
