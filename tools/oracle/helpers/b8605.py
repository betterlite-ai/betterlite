import asyncio
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy.common_request_processing import (
    _assembled_model_came_from_a_later_chunk,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L356
    _assembled_model_is_the_name_the_client_asked_for,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L378
    _record_streaming_client_disconnect_if_needed,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L293
)
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]

_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class ModelCheck(TypedDict):
    chunks: list[JsonObject]
    assembled: JsonValue
    request_data: JsonObject


class RecordInput(TypedDict):
    function: Literal["record_disconnect"]
    requests: list[JsonObject]


class ChecksInput(TypedDict):
    function: Literal["model_checks"]
    checks: list[ModelCheck]


Input: TypeAlias = RecordInput | ChecksInput


async def record_answers(requests: list[JsonObject]) -> list[JsonObject]:
    for request in requests:
        await _record_streaming_client_disconnect_if_needed(
            request=None,
            request_data=request,
            client_disconnected=True,
        )
    return requests


def model_answer(check: ModelCheck) -> JsonObject:
    return {
        "later_chunk": _assembled_model_came_from_a_later_chunk(
            check["chunks"], check["assembled"]
        ),
        "client_named": _assembled_model_is_the_name_the_client_asked_for(
            check["request_data"], check["assembled"]
        ),
    }


def model_answers(checks: list[ModelCheck]) -> list[JsonObject]:
    return [model_answer(check) for check in checks]


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "record_disconnect":
        return _JSON.validate_python(asyncio.run(record_answers(input_value["requests"])))
    return _JSON.validate_python(model_answers(input_value["checks"]))
