import asyncio
import json
from typing import TypedDict

from litellm.proxy.guardrails.guardrail_endpoints import get_provider_specific_params
from pydantic import JsonValue, TypeAdapter
from pydantic_core import PydanticUndefinedType

ANSWERS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class UndefinedAsNull:
    def __call__(self, _value: PydanticUndefinedType) -> JsonValue:
        return None


class Input(TypedDict):
    provider: str


def run(input_value: Input) -> JsonValue:
    provider = input_value["provider"]
    answer = json.dumps(asyncio.run(get_provider_specific_params()), default=UndefinedAsNull())
    params = ANSWERS.validate_json(answer)
    if provider not in params:
        raise ValueError(f"no config model for {provider}")
    return params[provider]
