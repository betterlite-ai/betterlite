import asyncio
import json
from typing import Literal, TypedDict, TypeVar
from unittest.mock import patch

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


class SyntheticRegistration:
    def __init__(self, config_model: type[GuardrailConfigModel[BaseModel]]) -> None:
        self.config_model = config_model

    def get_config_model(self) -> type[GuardrailConfigModel[BaseModel]]:
        return self.config_model


SYNTHETIC_REGISTRATIONS: dict[str, SyntheticRegistration] = {
    "synthetic_literal_list": SyntheticRegistration(SyntheticLiteralList),
    "synthetic_nested": SyntheticRegistration(SyntheticNested),
    "synthetic_generic_unfilled": SyntheticRegistration(SyntheticGenericUnfilled),
    "synthetic_generic_filled": SyntheticRegistration(SyntheticGenericFilled),
}


class UndefinedAsNull:
    def __call__(self, _value: PydanticUndefinedType) -> JsonValue:
        return None


class Input(TypedDict):
    model: str


def route_answer() -> dict[str, JsonValue]:
    with patch.dict(guardrail_class_registry, SYNTHETIC_REGISTRATIONS, clear=True):
        provider_params: object = asyncio.run(get_provider_specific_params())  # type: ignore[misc, no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_endpoints.py#L1915
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
