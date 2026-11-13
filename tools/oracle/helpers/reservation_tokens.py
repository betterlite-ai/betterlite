import asyncio
import os
from collections.abc import Coroutine, Mapping
from typing import Protocol, TypeAlias, cast

# LiteLLM reads LITELLM_RUST per call; false pins the Python path the oracle compares against.
os.environ["LITELLM_RUST"] = "false"

from litellm.proxy.spend_tracking import input_tokens as input_tokens_module

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class CountForModel(Protocol):
    def __call__(self, request_body: dict[str, Json], model: str) -> int | None: ...


class CountInputTokens(Protocol):
    def __call__(
        self, request_body: dict[str, Json], raw_body: bytes | None, models: list[str]
    ) -> Coroutine[object, object, Mapping[str, int]]: ...


class ApproximateInputSize(Protocol):
    def __call__(self, request_body: dict[str, Json]) -> int: ...


count_for_model = cast(
    CountForModel, input_tokens_module.__dict__["count_input_tokens_for_model"]
)
count_input_tokens = cast(CountInputTokens, input_tokens_module.__dict__["count_input_tokens"])
approximate_input_size = cast(
    ApproximateInputSize, input_tokens_module.__dict__["_approximate_input_size"]
)


def _request(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError("request must be an object")
    return value


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _models(value: Json) -> list[str]:
    if not isinstance(value, list):
        raise ValueError("models must be a list")
    return [_string(item, "each model") for item in value]


def _calls(input_value: dict[str, Json]) -> list[dict[str, Json]]:
    value = input_value.get("calls")
    if not isinstance(value, list):
        raise ValueError("calls must be a list")
    calls: list[dict[str, Json]] = []
    for call in value:
        if not isinstance(call, dict):
            raise ValueError("each call must be an object")
        calls.append(call)
    return calls


def _call(call: dict[str, Json]) -> Json:
    function = call.get("function")
    match function:
        case "count_for_model":
            return count_for_model(
                _request(call.get("request")), _string(call.get("model"), "model")
            )
        case "aggregate":
            counts = asyncio.run(
                count_input_tokens(_request(call.get("request")), None, _models(call.get("models")))
            )
            return cast(Json, dict(counts))
        case "size":
            return approximate_input_size(_request(call.get("request")))
        case other:
            raise ValueError(f"unknown reservation_tokens function: {other!r}")


def run(input_value: dict[str, Json]) -> Json:
    return [_call(call) for call in _calls(input_value)]
