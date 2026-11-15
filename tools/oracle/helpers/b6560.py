import json
from typing import Literal, TypedDict, TypeVar

from litellm.proxy.guardrails.guardrail_endpoints import _get_fields_from_model
from litellm.types.guardrails import (
    BedrockGuardrailConfigModel,
    BedrockGuardrailStreamingParams,
    LakeraV2GuardrailConfigModel,
    PresidioPresidioConfigModelUserInterface,
    ToolPermissionGuardrailConfigModel,
)
from litellm.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel
from litellm.types.proxy.guardrails.guardrail_hooks.hide_secrets import (
    HideSecretsGuardrailConfigModel,
)
from pydantic import BaseModel, Field, JsonValue, TypeAdapter
from pydantic_core import PydanticUndefinedType
from typing_extensions import override

ANSWERS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
T = TypeVar("T", bound=BaseModel)


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


class UndefinedAsNull:
    def __call__(self, _value: PydanticUndefinedType) -> JsonValue:
        return None


class Input(TypedDict):
    model: str


def as_answer(fields: dict[str, object]) -> JsonValue:
    return ANSWERS.validate_json(json.dumps(fields, default=UndefinedAsNull()))


def synthetic_fields(model_class: type[GuardrailConfigModel[BaseModel]]) -> dict[str, object]:
    fields = _get_fields_from_model(model_class)
    fields["ui_friendly_name"] = model_class.ui_friendly_name()
    return fields


def provider_answers() -> dict[str, dict[str, object]]:
    bedrock_fields: dict[str, object] = {
        **_get_fields_from_model(BedrockGuardrailConfigModel),
        **_get_fields_from_model(BedrockGuardrailStreamingParams),
    }
    tool_permission_fields = _get_fields_from_model(ToolPermissionGuardrailConfigModel)
    tool_permission_fields["ui_friendly_name"] = (
        ToolPermissionGuardrailConfigModel.ui_friendly_name()
    )
    hide_secrets_fields = _get_fields_from_model(HideSecretsGuardrailConfigModel)
    hide_secrets_fields["ui_friendly_name"] = HideSecretsGuardrailConfigModel.ui_friendly_name()
    return {
        "bedrock": bedrock_fields,
        "presidio": _get_fields_from_model(PresidioPresidioConfigModelUserInterface),
        "lakera_v2": _get_fields_from_model(LakeraV2GuardrailConfigModel),
        "tool_permission": tool_permission_fields,
        "hide-secrets": hide_secrets_fields,
        "synthetic_literal_list": synthetic_fields(SyntheticLiteralList),
        "synthetic_nested": synthetic_fields(SyntheticNested),
    }


def run(input_value: Input) -> JsonValue:
    if input_value["model"] == "synthetic_generic":
        return {
            "unfilled": as_answer(synthetic_fields(SyntheticGenericUnfilled)),
            "filled": as_answer(synthetic_fields(SyntheticGenericFilled)),
        }
    return as_answer(provider_answers()[input_value["model"]])
