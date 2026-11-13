import asyncio
import json
from typing import Literal, TypedDict, TypeVar

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.proxy.guardrails.guardrail_endpoints import get_provider_specific_params
from litellm.proxy.guardrails.guardrail_registry import guardrail_class_registry
from litellm.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel
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


class SyntheticLiteralListGuardrail(CustomGuardrail):
    @staticmethod
    @override
    def get_config_model() -> type[GuardrailConfigModel[BaseModel]] | None:
        return SyntheticLiteralList


class SyntheticNestedGuardrail(CustomGuardrail):
    @staticmethod
    @override
    def get_config_model() -> type[GuardrailConfigModel[BaseModel]] | None:
        return SyntheticNested


class SyntheticGenericUnfilledGuardrail(CustomGuardrail):
    @staticmethod
    @override
    def get_config_model() -> type[GuardrailConfigModel[BaseModel]] | None:
        return SyntheticGenericUnfilled


class SyntheticGenericFilledGuardrail(CustomGuardrail):
    @staticmethod
    @override
    def get_config_model() -> type[GuardrailConfigModel[BaseModel]] | None:
        return SyntheticGenericFilled


SYNTHETIC_GUARDRAILS: dict[str, type[CustomGuardrail]] = {
    "synthetic_literal_list": SyntheticLiteralListGuardrail,
    "synthetic_nested": SyntheticNestedGuardrail,
    "synthetic_generic_unfilled": SyntheticGenericUnfilledGuardrail,
    "synthetic_generic_filled": SyntheticGenericFilledGuardrail,
}


class UndefinedAsNull:
    def __call__(self, _value: PydanticUndefinedType) -> JsonValue:
        return None


class Input(TypedDict):
    model: str


def route_answer() -> dict[str, JsonValue]:
    saved = dict(guardrail_class_registry)
    guardrail_class_registry.clear()
    guardrail_class_registry.update(SYNTHETIC_GUARDRAILS)
    try:
        provider_params = asyncio.run(get_provider_specific_params())
    finally:
        guardrail_class_registry.clear()
        guardrail_class_registry.update(saved)
    return ANSWERS.validate_json(json.dumps(provider_params, default=UndefinedAsNull()))


def run(input_value: Input) -> JsonValue:
    model = input_value["model"]
    answers = route_answer()
    if model == "synthetic_generic":
        return {
            "unfilled": answers["synthetic_generic_unfilled"],
            "filled": answers["synthetic_generic_filled"],
        }
    return answers[model]
