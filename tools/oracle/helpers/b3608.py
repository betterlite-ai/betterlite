from collections.abc import Callable
from datetime import UTC, datetime
from typing import TypeAlias, cast

from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.proxy.pass_through_endpoints.streaming_handler import PassThroughStreamingHandler
from litellm.proxy.pass_through_endpoints.success_handler import PassThroughEndpointLogging
from litellm.types.passthrough_endpoints.pass_through_endpoints import EndpointType

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Operation: TypeAlias = Callable[[dict[str, Json]], Json]

_START_TIME = datetime(2024, 1, 1, tzinfo=UTC)
_MIN_BYTE = 0
_MAX_BYTE = 255

_ENDPOINTS: dict[str, EndpointType] = {
    "vertex-ai": EndpointType.VERTEX_AI,
    "gemini": EndpointType.GEMINI,
    "anthropic": EndpointType.ANTHROPIC,
    "openai": EndpointType.OPENAI,
    "tinyfish": EndpointType.TINYFISH,
    "generic": EndpointType.GENERIC,
    "decisions": EndpointType.DECISIONS,
}


class Recording:
    def __init__(self, details: dict[str, object]) -> None:
        self.model_call_details = details


def run(input_value: dict[str, Json]) -> Json:
    operation = _text(input_value.get("operation"), "operation")
    handler = _HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown b3608 operation: {operation}")
    return handler(input_value)


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _optional_text(value: Json) -> str | None:
    if isinstance(value, str):
        return value
    return None


def _byte(value: Json) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError("byte values must be integers")
    if not _MIN_BYTE <= value <= _MAX_BYTE:
        raise ValueError("byte values must be from 0 to 255")
    return value


def _chunk(value: Json) -> bytes:
    if not isinstance(value, list):
        raise TypeError("each chunk must be a list of byte values")
    return bytes(_byte(item) for item in value)


def _chunks(value: Json) -> list[bytes]:
    if not isinstance(value, list):
        raise TypeError("chunks must be a list")
    return [_chunk(item) for item in value]


def _optional_object(value: Json) -> dict[str, Json] | None:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise TypeError("request_body must be an object or null")
    return value


def _object_or_empty(value: Json) -> dict[str, Json]:
    body = _optional_object(value)
    return {} if body is None else body


def _endpoint(value: Json) -> EndpointType:
    name = _text(value, "endpoint_type")
    endpoint = _ENDPOINTS.get(name)
    if endpoint is None:
        raise ValueError(f"unknown endpoint type: {name}")
    return endpoint


def _lines(input_value: dict[str, Json]) -> Json:
    chunks = _chunks(input_value.get("chunks"))
    lines = PassThroughStreamingHandler._convert_raw_bytes_to_str_lines(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        chunks
    )
    return {"lines": cast(Json, lines)}


def _extract_model(input_value: dict[str, Json]) -> Json:
    details: dict[str, object] = {}
    model = _optional_text(input_value.get("model_call_details_model"))
    if model is not None:
        details["model"] = model
    logging_obj = cast(LiteLLMLoggingObj, Recording(details))
    result = PassThroughStreamingHandler._extract_model_for_cost_injection(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        _optional_object(input_value.get("request_body")),
        _text(input_value.get("url_route"), "url_route"),
        _endpoint(input_value.get("endpoint_type")),
        logging_obj,
    )
    return {"model": result}


def _build_result(input_value: dict[str, Json]) -> Json:
    logging_obj = cast(LiteLLMLoggingObj, Recording({}))
    result, kwargs = PassThroughStreamingHandler._build_passthrough_logging_result(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
        litellm_logging_obj=logging_obj,
        passthrough_success_handler_obj=PassThroughEndpointLogging(),
        url_route=_text(input_value.get("url_route"), "url_route"),
        request_body=_object_or_empty(input_value.get("request_body")),
        endpoint_type=_endpoint(input_value.get("endpoint_type")),
        start_time=_START_TIME,
        raw_bytes=_chunks(input_value.get("chunks")),
        end_time=_START_TIME,
        model=_optional_text(input_value.get("model")),
    )
    return {"result": cast(Json, result), "kwargs": cast(Json, kwargs)}


_HANDLERS: dict[str, Operation] = {
    "lines": _lines,
    "extract_model": _extract_model,
    "build_result": _build_result,
}
