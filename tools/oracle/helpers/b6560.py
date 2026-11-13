import json
from typing import Literal, TypedDict, TypeVar, cast

from litellm.proxy.guardrails.guardrail_endpoints import _get_fields_from_model  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_endpoints.py#L1902
from litellm.types.guardrails import (
    BedrockGuardrailConfigModel as BedrockModel,
)
from litellm.types.guardrails import (
    BedrockGuardrailStreamingParams as BedrockStreaming,
)
from litellm.types.guardrails import (
    LakeraV2GuardrailConfigModel as LakeraV2Model,
)
from litellm.types.guardrails import (
    PresidioPresidioConfigModelUserInterface as PresidioUi,
)
from litellm.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel
from litellm.types.proxy.guardrails.guardrail_hooks.hide_secrets import (
    HideSecretsGuardrailConfigModel as HideSecretsModel,
)
from litellm.types.proxy.guardrails.guardrail_hooks.tool_permission import (
    ToolPermissionGuardrailConfigModel as ToolPermissionModel,
)
from pydantic import BaseModel, Field, JsonValue, TypeAdapter
from pydantic_core import PydanticUndefinedType
from typing_extensions import override

_JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
T = TypeVar("T", bound=BaseModel)


class FormModel(BaseModel):
    @staticmethod
    def ui_friendly_name() -> str:
        return ""


class SyntheticEntity(BaseModel):
    label: str = Field(description="Entity label shown in the UI")
    score: float | None = Field(default=None, description="Entity score")


class SyntheticLiteralList(GuardrailConfigModel[BaseModel]):
    kinds: list[Literal["alpha", "beta"]] | None = Field(default=None, description="Kinds to scan")
    weights: dict[Literal["left", "right"], float] | None = Field(
        default=None, description="Weight per side"
    )
    mode: Literal["fast", "slow"] | None = Field(default=None, description="Processing mode")
    ratio: float | None = Field(
        default=None,
        description="Share of traffic scanned",
        json_schema_extra={"ui_type": "percentage", "min": 0.0, "max": 1.0, "step": 0.05},
    )
    tags: list[Literal["red"]] | None = Field(
        default=None,
        description="Tags attached to a match",
        json_schema_extra={"options": ["blue", "green"]},
    )
    counts: dict[str, int] | None = Field(default=None, description="Matches per label")
    enabled: bool | None = Field(default=False, description="Whether the check runs")

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Synthetic Literal List"


class SyntheticNested(GuardrailConfigModel[BaseModel]):
    entity: SyntheticEntity | None = Field(default=None, description="Primary entity")
    entities: list[SyntheticEntity] | None = Field(default=None, description="Entities scanned")

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Synthetic Nested"


class SyntheticGenericUnfilled(GuardrailConfigModel[T]):
    enabled: bool | None = Field(default=None, description="Whether the check runs")

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Synthetic Generic"


class SyntheticGenericFilled(GuardrailConfigModel[BaseModel]):
    enabled: bool | None = Field(default=None, description="Whether the check runs")

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Synthetic Generic"


LAKERA_V2: type[FormModel] = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L709
    "type[FormModel]",
    LakeraV2Model,
)
TOOL_PERMISSION: type[FormModel] = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/proxy/guardrails/guardrail_hooks/tool_permission.py#L113
    "type[FormModel]",
    ToolPermissionModel,
)
HIDE_SECRETS: type[FormModel] = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/proxy/guardrails/guardrail_hooks/hide_secrets.py#L8
    "type[FormModel]",
    HideSecretsModel,
)
class UndefinedAsNull:
    def __call__(self, _value: PydanticUndefinedType) -> JsonValue:
        return None


class Input(TypedDict):
    model: str


def answer(model: type[BaseModel], friendly_name: str | None) -> JsonValue:
    fields = _get_fields_from_model(model)
    if friendly_name is not None:
        fields["ui_friendly_name"] = friendly_name
    return _JSON.validate_json(json.dumps(fields, default=UndefinedAsNull()))


def bedrock_answer() -> JsonValue:
    fields = _get_fields_from_model(BedrockModel)
    fields.update(_get_fields_from_model(BedrockStreaming))
    return _JSON.validate_json(json.dumps(fields, default=UndefinedAsNull()))


def generic_answer() -> JsonValue:
    unfilled = _JSON.validate_json(
        json.dumps(_get_fields_from_model(SyntheticGenericUnfilled), default=UndefinedAsNull())
    )
    filled = _JSON.validate_json(
        json.dumps(_get_fields_from_model(SyntheticGenericFilled), default=UndefinedAsNull())
    )
    return {"unfilled": unfilled, "filled": filled}


def table_answer(model: str) -> JsonValue:
    if model == "bedrock":
        return bedrock_answer()
    if model == "presidio":
        return answer(PresidioUi, None)
    if model == "lakera_v2":
        return answer(LAKERA_V2, None)
    if model == "tool_permission":
        return answer(TOOL_PERMISSION, TOOL_PERMISSION.ui_friendly_name())
    if model == "hide-secrets":
        return answer(HIDE_SECRETS, HIDE_SECRETS.ui_friendly_name())
    return None


def synthetic_answer(model: str) -> JsonValue:
    if model == "synthetic_literal_list":
        return answer(SyntheticLiteralList, None)
    if model == "synthetic_nested":
        return answer(SyntheticNested, None)
    if model == "synthetic_generic":
        return generic_answer()
    return None


def run(input_value: Input) -> JsonValue:
    model = input_value["model"]
    table = table_answer(model)
    if table is not None:
        return table
    synthetic = synthetic_answer(model)
    if synthetic is not None:
        return synthetic
    raise ValueError(f"unknown form model {model}")
