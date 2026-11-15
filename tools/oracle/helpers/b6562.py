import asyncio
import json
from collections.abc import Callable, Coroutine
from typing import Final, TypedDict

from litellm.proxy.guardrails.guardrail_endpoints import get_provider_specific_params
from pydantic import JsonValue, TypeAdapter
from pydantic_core import PydanticUndefinedType

ANSWERS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])

ENDPOINT: Final[Callable[[], Coroutine[None, None, dict[str, dict[str, JsonValue]]]]] = (
    get_provider_specific_params
)


class UndefinedAsNull:
    def __call__(self, _value: PydanticUndefinedType) -> JsonValue:
        return None


class Input(TypedDict):
    provider: str


def run(input_value: Input) -> JsonValue:
    params = asyncio.run(ENDPOINT())
    text = json.dumps(params[input_value["provider"]], default=UndefinedAsNull())
    return ANSWERS.validate_json(text)
