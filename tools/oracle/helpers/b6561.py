import json
from typing import TypedDict

from litellm.proxy.guardrails.guardrail_endpoints import _get_fields_from_model
from litellm.proxy.guardrails.guardrail_registry import guardrail_class_registry
from litellm.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel
from pydantic import BaseModel, JsonValue, TypeAdapter
from pydantic_core import PydanticUndefinedType

ANSWERS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class UndefinedAsNull:
    def __call__(self, _value: PydanticUndefinedType) -> JsonValue:
        return None


class Input(TypedDict):
    provider: str


def project(model_class: type[BaseModel]) -> dict[str, JsonValue]:
    text = json.dumps(_get_fields_from_model(model_class), default=UndefinedAsNull())
    return ANSWERS.validate_json(text)


def run(input_value: Input) -> JsonValue:
    provider = input_value["provider"]
    model_class: type[GuardrailConfigModel[BaseModel]] | None = guardrail_class_registry[provider].get_config_model()
    if model_class is None:
        raise ValueError(f"no config model for {provider}")
    fields = project(model_class)
    fields["ui_friendly_name"] = model_class.ui_friendly_name()
    return fields
