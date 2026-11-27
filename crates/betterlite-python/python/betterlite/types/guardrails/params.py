from datetime import datetime
from enum import Enum
from typing import ClassVar, Literal

from pydantic import ConfigDict, Field, JsonValue, field_validator, model_validator
from typing_extensions import ReadOnly, Required, Self, TypedDict

from betterlite.types.guardrails.content_filter import (
    MCP_SECURITY_ON_VIOLATION,
    ContentFilterConfigModel,
)
from betterlite.types.guardrails.core import (
    LakeraCategoryThresholds,
    SupportedGuardrailIntegrations,
)
from betterlite.types.guardrails.pii import PresidioConfigModel
from betterlite.types.guardrails.vendor_configs import (
    BedrockChecksConfigModel,
    BedrockGuardrailConfigModel,
    DeepKeepGuardrailConfigModel,
    JavelinGuardrailConfigModel,
    LakeraV2GuardrailConfigModel,
    LassoGuardrailConfigModel,
    NomaGuardrailConfigModel,
    PillarGuardrailConfigModel,
    ZscalerAIGuardConfigModel,
)
from betterlite.types.proxy.guardrails.guardrail_hooks.agent_365 import Agent365GuardrailConfigModel
from betterlite.types.proxy.guardrails.guardrail_hooks.akto import AktoConfigModel
from betterlite.types.proxy.guardrails.guardrail_hooks.block_code_execution import (
    BlockCodeExecutionGuardrailConfigModel,
)
from betterlite.types.proxy.guardrails.guardrail_hooks.cisco_ai_defense import (
    CiscoAIDefenseGuardrailConfigModel,
)
from betterlite.types.proxy.guardrails.guardrail_hooks.compresr import CompresrGuardrailConfigModel
from betterlite.types.proxy.guardrails.guardrail_hooks.decision_model import DecisionModelCheck
from betterlite.types.proxy.guardrails.guardrail_hooks.enkryptai import EnkryptAIGuardrailConfigs
from betterlite.types.proxy.guardrails.guardrail_hooks.grayswan import GraySwanGuardrailConfigModel
from betterlite.types.proxy.guardrails.guardrail_hooks.headroom import HeadroomGuardrailConfigModel
from betterlite.types.proxy.guardrails.guardrail_hooks.hiddenlayer import (
    HiddenlayerGuardrailConfigModel,
)
from betterlite.types.proxy.guardrails.guardrail_hooks.ibm.base import IBMGuardrailsBaseConfigModel
from betterlite.types.proxy.guardrails.guardrail_hooks.ovalix import OvalixGuardrailConfigModel
from betterlite.types.proxy.guardrails.guardrail_hooks.promptguard import PromptGuardConfigModel
from betterlite.types.proxy.guardrails.guardrail_hooks.qohash import QostodianNexusConfigModel
from betterlite.types.proxy.guardrails.guardrail_hooks.qualifire import (
    QualifireGuardrailConfigModel,
)
from betterlite.types.proxy.guardrails.guardrail_hooks.repelloai import (
    RepelloAIGuardrailConfigModel,
)
from betterlite.types.proxy.guardrails.guardrail_hooks.singulr import SingulrGuardrailConfigModel
from betterlite.types.proxy.guardrails.guardrail_hooks.tool_permission import (
    ToolPermissionGuardrailConfigModel,
)
from betterlite.types.proxy.guardrails.guardrail_hooks.typesafe import TypeSafeGuardrailConfigModel
from betterlite.types.proxy.guardrails.guardrail_hooks.vigil_guard import (
    VigilGuardGuardrailConfigModel,
)
from betterlite.types.proxy.guardrails.guardrail_hooks.xecguard import XecGuardConfigModel
from betterlite.types.utils.base import ResponseObject


class Mode(ResponseObject):
    tags: dict[str, str | list[str]]
    default: str | list[str] | None = None


class BaseLitellmParams(ContentFilterConfigModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", protected_namespaces=())

    api_key: str | None = None
    api_base: str | None = None
    # ast-grep-ignore: no-long-names
    experimental_use_latest_role_message_only: bool | None = False
    only_scan_new_messages: bool | None = False
    # ast-grep-ignore: no-long-names
    skip_system_message_in_guardrail: bool | None = None
    skip_tool_message_in_guardrail: bool | None = None
    scan_only_tool_results: bool | None = None
    inspect_embeddings: bool | None = None
    category_thresholds: LakeraCategoryThresholds | None = None
    detect_secrets_config: dict[str, JsonValue] | None = None
    guard_name: str | None = None
    default_on: bool | None = None
    mask_request_content: bool | None = None
    mask_response_content: bool | None = None
    pangea_input_recipe: str | None = None
    pangea_output_recipe: str | None = None
    model: str | None = None
    violation_message_template: str | None = None
    end_session_after_n_fails: int | None = None
    on_violation: Literal["warn", "end_session", "block", "alert"] | None = None
    realtime_violation_message: str | None = None
    template_id: str | None = None
    location: str | None = None
    credentials: str | None = None
    api_endpoint: str | None = None
    fail_on_error: bool | None = True
    skip_unscannable_attachments: bool | None = False
    sanitize_error_detail: bool | None = True
    # ast-grep-ignore: no-long-names
    additional_provider_specific_params: dict[str, JsonValue] | None = None
    unreachable_fallback: Literal["fail_closed", "fail_open"] = "fail_closed"
    max_input_chars: int | None = Field(default=None, gt=0)
    max_concurrent_decision_calls: int | None = Field(default=None, gt=0)
    extra_headers: list[str] | None = None
    custom_code: str | None = None
    timeout: float | None = None
    on_sensitive_data: Literal["block", "route"] | None = None
    sensitive_data_route_to_model: str | None = None
    sticky_session_routing: bool | None = True
    run_in_parallel: bool | None = None
    scan_raw_request: bool | None = None

    @field_validator(
        "mode",
        "default_action",
        "on_disallowed_action",
        "unreachable_fallback",
        "on_sensitive_data",
        mode="before",
        check_fields=False,
    )
    @classmethod
    def normalize_lowercase(cls, value: JsonValue | Mode) -> JsonValue | Mode:
        match value:
            case str():
                return value.lower()
            case list():
                lowered: list[JsonValue] = []
                for item in value:
                    match item:
                        case str():
                            lowered.append(item.lower())
                        case list() | dict() | bool() | int() | float() | None:
                            lowered.append(item)
                return lowered
            case dict() | bool() | int() | float() | Mode() | None:
                return value

    @model_validator(mode="after")
    # ast-grep-ignore: no-long-names
    def validate_sensitive_data_routing(self) -> Self:
        if self.on_sensitive_data == "route" and not self.sensitive_data_route_to_model:
            raise ValueError(
                "sensitive_data_route_to_model must be set when on_sensitive_data='route'"
            )
        return self


class LitellmParams(  # pyright: ignore[reportGeneralTypeIssues, reportIncompatibleVariableOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L1192
    CiscoAIDefenseGuardrailConfigModel,  # ast-grep-ignore: no-long-names
    PresidioConfigModel,
    BedrockGuardrailConfigModel,
    LakeraV2GuardrailConfigModel,
    HeadroomGuardrailConfigModel,
    CompresrGuardrailConfigModel,
    TypeSafeGuardrailConfigModel,
    RepelloAIGuardrailConfigModel,
    LassoGuardrailConfigModel,
    DeepKeepGuardrailConfigModel,
    PillarGuardrailConfigModel,
    GraySwanGuardrailConfigModel,
    NomaGuardrailConfigModel,
    PromptGuardConfigModel,
    XecGuardConfigModel,
    # ast-grep-ignore: no-long-names
    ToolPermissionGuardrailConfigModel,
    ZscalerAIGuardConfigModel,
    AktoConfigModel,
    JavelinGuardrailConfigModel,
    BaseLitellmParams,
    EnkryptAIGuardrailConfigs,
    IBMGuardrailsBaseConfigModel,
    OvalixGuardrailConfigModel,
    QualifireGuardrailConfigModel,
    # ast-grep-ignore: no-long-names
    BlockCodeExecutionGuardrailConfigModel,
    # ast-grep-ignore: no-long-names
    HiddenlayerGuardrailConfigModel,
    QostodianNexusConfigModel,
    VigilGuardGuardrailConfigModel,
    SingulrGuardrailConfigModel,
    Agent365GuardrailConfigModel,
):
    guardrail: str
    decision_model: str | None = None
    checks: BedrockChecksConfigModel | tuple[DecisionModelCheck, ...] | None = None  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L1229
    mode: str | list[str] | Mode

    @field_validator("timeout", mode="before", check_fields=False)
    @classmethod
    def coerce_timeout(cls, value: JsonValue) -> float | None:
        if value is None:
            return None
        match value:
            case str():
                if value == "":
                    return None
                try:
                    return float(value)
                except ValueError as error:
                    raise ValueError(f"timeout must be numeric, got {value!r}") from error
            case bool() | int() | float():
                return float(value)
            case list() | dict():
                raise ValueError(f"timeout must be numeric, got {value!r}")

    @model_validator(mode="after")
    # ast-grep-ignore: no-long-names
    def validate_on_violation_for_guardrail(self) -> Self:
        if self.on_violation not in MCP_SECURITY_ON_VIOLATION:
            return self
        if self.guardrail == SupportedGuardrailIntegrations.MCP_SECURITY.value:
            return self
        raise ValueError(
            f"on_violation={self.on_violation!r} is only supported by guardrail='mcp_security'"
        )

    @model_validator(mode="after")
    # ast-grep-ignore: no-long-names
    def validate_checks_list_for_guardrail(self) -> Self:
        match self.checks:
            case tuple() if self.guardrail != SupportedGuardrailIntegrations.DECISION_MODEL.value:
                raise ValueError(
                    "checks as a list is only supported by guardrail='decision_model', "
                    f"got guardrail={self.guardrail!r}"
                )
            case tuple() | BedrockChecksConfigModel() | None:
                return self

    @model_validator(mode="before")
    @classmethod
    def default_on_false(cls, data: JsonValue | ResponseObject) -> JsonValue | ResponseObject:
        match data:
            case dict() if data.get("default_on") is None:
                return {**data, "default_on": False}
            case dict() | list() | str() | bool() | int() | float() | ResponseObject() | None:
                return data


class Guardrail(TypedDict, total=False):
    guardrail_id: str | None
    guardrail_name: Required[str]
    litellm_params: Required[LitellmParams]
    guardrail_info: dict[str, JsonValue] | None
    policy_template: str | None
    created_at: datetime | None
    updated_at: datetime | None


class guardrailConfig(TypedDict):  # noqa: N801  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L1301
    guardrails: list[Guardrail]


class GuardrailEventHooks(str, Enum):
    pre_call = "pre_call"
    post_call = "post_call"
    during_call = "during_call"
    logging_only = "logging_only"
    pre_mcp_call = "pre_mcp_call"
    during_mcp_call = "during_mcp_call"
    post_mcp_call = "post_mcp_call"
    realtime_input_transcription = "realtime_input_transcription"


class DynamicGuardrailParams(TypedDict):
    extra_body: ReadOnly[dict[str, JsonValue]]


class GUARDRAIL_DEFINITION_LOCATION(str, Enum):  # noqa: N801  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L1320
    DB = "db"
    CONFIG = "config"
