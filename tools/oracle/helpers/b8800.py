from collections.abc import Callable
from functools import partial
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy.common_request_processing import (
    _collect_response_file_search_vector_store_ids,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L617
    _request_model,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L698
    _stream_usage_tracking_updates,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L503
)
from pydantic import JsonValue, TypeAdapter


class StreamUsageInput(TypedDict):
    function: Literal["stream_usage_updates"]
    data: dict[str, JsonValue]
    always_include: bool | None
    route_type: str
    supports: bool


class FileSearchInput(TypedDict):
    function: Literal["file_search_store_ids"]
    data: dict[str, JsonValue]


class RequestModelInput(TypedDict):
    function: Literal["request_model"]
    data: dict[str, JsonValue]


Input: TypeAlias = StreamUsageInput | FileSearchInput | RequestModelInput

_JSON: Final = TypeAdapter(JsonValue)
_OBJECT: Final = TypeAdapter(dict[str, JsonValue])


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "stream_usage_updates":
        settings = {"always_include_stream_usage": input_value["always_include"]}
        supports: Callable[[], bool] = partial(bool, input_value["supports"])
        updates = _stream_usage_tracking_updates(
            input_value["data"], settings, input_value["route_type"], supports
        )
        return _OBJECT.validate_python(updates)
    if input_value["function"] == "file_search_store_ids":
        store_ids = sorted(_collect_response_file_search_vector_store_ids(input_value["data"]))
        return _JSON.validate_python(store_ids)
    model = _request_model(input_value["data"])
    return _JSON.validate_python(model)
