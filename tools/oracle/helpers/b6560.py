import json
from typing import Literal, TypedDict, TypeVar

from litellm.proxy.guardrails.guardrail_endpoints import _get_fields_from_model  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_endpoints.py#L1902
from litellm.types.guardrails import (
    BedrockGuardrailConfigModel as BedrockModel,
)
from litellm.types.guardrails import (
    BedrockGuardrailStreamingParams as BedrockStreaming,
)
from litellm.types.guardrails import (
    LakeraV2GuardrailConfigModel as LakeraModel,
)
from litellm.types.guardrails import (
    PresidioPresidioConfigModelUserInterface as PresidioUI,
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

ANSWERS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
T = TypeVar("T", bound=BaseModel)
ModelClass = type[GuardrailConfigModel[BaseModel]]


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


def as_answer(model_class: type[BaseModel]) -> dict[str, JsonValue]:
    text = json.dumps(_get_fields_from_model(model_class), default=UndefinedAsNull())
    return ANSWERS.validate_json(text)


def named_answer(model_class: ModelClass) -> dict[str, JsonValue]:
    fields = as_answer(model_class)
    fields["ui_friendly_name"] = model_class.ui_friendly_name()
    return fields


def provider_fields() -> dict[str, dict[str, JsonValue]]:
    models: tuple[ModelClass, ...] = (ToolPermissionModel, HideSecretsModel, LakeraModel)  # type: ignore[misc, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/proxy/guardrails/guardrail_hooks/base.py#L12
    tool_permission, hide_secrets, lakera = models
    bedrock: dict[str, JsonValue] = {
        **as_answer(BedrockModel),
        **as_answer(BedrockStreaming),
    }
    return {
        "bedrock": bedrock,
        "presidio": as_answer(PresidioUI),
        "lakera_v2": as_answer(lakera),
        "tool_permission": named_answer(tool_permission),
        "hide-secrets": named_answer(hide_secrets),
        "synthetic_literal_list": named_answer(SyntheticLiteralList),
        "synthetic_nested": named_answer(SyntheticNested),
    }


def run(input_value: Input) -> JsonValue:
    if input_value["model"] == "synthetic_generic":
        answers: dict[str, JsonValue] = {
            "unfilled": named_answer(SyntheticGenericUnfilled),
            "filled": named_answer(SyntheticGenericFilled),
        }
        return answers
    return provider_fields()[input_value["model"]]
