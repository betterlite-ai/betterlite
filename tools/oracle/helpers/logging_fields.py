import datetime as dt
from collections.abc import Callable
from typing import TypeAlias, cast, get_type_hints
from unittest.mock import patch

from litellm.litellm_core_utils import litellm_logging
from litellm.types.utils import (
    StandardLoggingHiddenParams,
    StandardLoggingMetadata,
    StandardLoggingPayload,
    StandardLoggingPayloadErrorInformation,
    StandardLoggingPayloadStatus,
    Usage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


_HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {}


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    handler = _HANDLERS.get(cast(str, function))
    if handler is None:
        raise ValueError(f"unknown logging_fields function {function}")
    return handler(input_value)


def _moment(value: Json) -> object:
    if isinstance(value, str):
        return dt.datetime.fromisoformat(value)
    return value


def _cleanup_timestamps(input_value: dict[str, Json]) -> Json:
    start = cast("dt.datetime | float", _moment(input_value.get("start")))
    end = cast("dt.datetime | float", _moment(input_value.get("end")))
    completion = cast("dt.datetime | float", _moment(input_value.get("completion_start")))
    values = litellm_logging.StandardLoggingPayloadSetup.cleanup_timestamps(start, end, completion)
    return {
        "start_time": values[0],
        "end_time": values[1],
        "completion_start_time": values[2],
    }


def _system_messages(input_value: dict[str, Json]) -> Json:
    kwargs: dict[str, object] | None = None
    if "system" in input_value:
        kwargs = {"system": input_value.get("system")}
    messages = input_value.get("messages")
    result = litellm_logging.StandardLoggingPayloadSetup.append_system_prompt_messages(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        kwargs, messages
    )
    return cast(Json, result)


def _status_fields(input_value: dict[str, Json]) -> Json:
    status = cast("StandardLoggingPayloadStatus", input_value.get("status"))
    guardrails = cast("list[dict[str, object]] | None", input_value.get("guardrail_information"))
    result = litellm_logging._get_status_fields(status, guardrails, None)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    return cast(Json, result)


def _usage_source(input_value: dict[str, Json]) -> tuple[dict[str, object] | None, Usage | None]:
    raw = input_value.get("usage")
    response_obj: dict[str, object] | None = None
    if "usage" in input_value and raw is not None:
        response_obj = {"usage": raw}
    combined_raw = input_value.get("combined")
    combined = None if combined_raw is None else Usage.model_validate(combined_raw)
    return response_obj, combined


def _usage_as_dict(input_value: dict[str, Json]) -> Json:
    response_obj, combined = _usage_source(input_value)
    result = litellm_logging.StandardLoggingPayloadSetup.get_usage_as_dict(response_obj, combined)  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
    return cast(Json, result)


def _init_response(value: Json) -> object:
    if isinstance(value, dict) and "mapping" in value:
        return _object(value.get("mapping"))
    return value


def _extract_response(input_value: dict[str, Json]) -> Json:
    init = _init_response(input_value.get("init"))
    headers = input_value.get("additional_headers")
    exception = None if headers is None else ValueError("boom")
    with (
        patch.object(
            litellm_logging.StandardLoggingPayloadSetup,
            "get_additional_headers",
            return_value=headers,
        ),
        patch(
            "litellm.litellm_core_utils.litellm_logging._get_response_headers",
            return_value=None if headers is None else {},
        ),
    ):
        response_obj, hidden = litellm_logging._extract_response_obj_and_hidden_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            init, exception
        )
    return {"response_obj": cast(Json, response_obj), "hidden_params": cast(Json, hidden)}


def _null_record(template: type[object]) -> dict[str, Json]:
    return cast("dict[str, Json]", dict.fromkeys(get_type_hints(template)))


def _object(value: Json) -> dict[str, Json]:
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise ValueError("expected an object")
    return cast("dict[str, Json]", value)


def _payload_record(input_value: dict[str, Json]) -> Json:
    raw = _object(input_value.get("payload"))
    metadata = _null_record(StandardLoggingMetadata)
    metadata.update(_object(raw.get("metadata")))
    hidden = _null_record(StandardLoggingHiddenParams)
    hidden.update(_object(raw.get("hidden_params")))
    payload: dict[str, Json] = {}
    for key in StandardLoggingPayload.__required_keys__:
        payload[key] = raw.get(key)
    for key in StandardLoggingPayload.__optional_keys__:
        if key in raw:
            payload[key] = raw.get(key)
    payload["metadata"] = metadata
    payload["hidden_params"] = hidden
    if raw.get("error_information") is not None:
        information = _null_record(StandardLoggingPayloadErrorInformation)
        information.update(_object(raw.get("error_information")))
        payload["error_information"] = information
    return payload


_HANDLERS.update(
    {
        "cleanup_timestamps": _cleanup_timestamps,
        "system_messages": _system_messages,
        "status_fields": _status_fields,
        "usage_as_dict": _usage_as_dict,
        "extract_response": _extract_response,
        "payload_record": _payload_record,
    }
)
