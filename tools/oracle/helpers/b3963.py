from collections.abc import Callable
from datetime import datetime, timedelta, timezone
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch
from uuid import UUID

import httpx
import litellm
from litellm.integrations.galileo import GalileoObserve
from litellm.types.llms.openai import (
    HttpxBinaryResponseContent,
    ResponseOutputItem,
    ResponsesAPIResponse,
)

if TYPE_CHECKING:
    from litellm.types.rerank import RerankResponseResult
    from litellm.types.utils import ImageObject

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    if operation == "message_normalization":
        return _message_normalization(input_value)
    if operation == "timestamp_formats":
        return _timestamp_formats(input_value)
    if operation == "token_metrics":
        return _token_metrics(input_value)
    if operation == "trace_envelope":
        return _trace_envelope(input_value)
    if operation == "content_selection":
        return _content_selection(input_value)
    message = f"unknown operation: {operation}"
    raise ValueError(message)


def _message_normalization(input_value: dict[str, Json]) -> Json:
    messages = input_value.get("messages")
    input_text = cast(str, input_value["input_text"])
    logger = object.__new__(GalileoObserve)
    normalize = cast(
        Callable[[Json, str], list[dict[str, str]]],
        logger._galileo_input_messages,  # pyright: ignore[reportPrivateUsage]
    )
    return cast(Json, normalize(messages, input_text))


def _timestamp_formats(input_value: dict[str, Json]) -> Json:
    naive = datetime.fromisoformat(cast(str, input_value["naive"]))
    offset_minutes = cast(int, input_value["offset_minutes"])
    aware = datetime.fromisoformat(cast(str, input_value["aware"]))
    normalize = cast(list[str], input_value["normalize"])
    logger = object.__new__(GalileoObserve)
    format_created_at = cast(
        Callable[[object], str],
        logger._format_created_at,  # pyright: ignore[reportPrivateUsage]
    )
    normalize_created_at = cast(
        Callable[[str], str],
        logger._normalize_created_at,  # pyright: ignore[reportPrivateUsage]
    )
    local_offset = timezone(timedelta(minutes=offset_minutes))
    with patch.object(GalileoObserve, "_local_timezone", staticmethod(lambda: local_offset)):
        naive_formatted = format_created_at(naive)
    return {
        "naive_formatted": naive_formatted,
        "aware_formatted": format_created_at(aware),
        "normalized": [normalize_created_at(value) for value in normalize],
    }


def _token_metrics(input_value: dict[str, Json]) -> Json:
    record = cast(dict[str, Json], input_value["record"])
    logger = object.__new__(GalileoObserve)
    metrics = cast(
        Callable[[dict[str, Json]], dict[str, Json]],
        logger._token_metrics_from_record,  # pyright: ignore[reportPrivateUsage]
    )
    return metrics(record)


def _trace_envelope(input_value: dict[str, Json]) -> Json:
    record = cast(dict[str, Json], input_value["record"])
    trace_id = cast(str, input_value["trace_id"])
    span_id = cast(str, input_value["span_id"])
    log_stream_id = cast("str | None", input_value.get("log_stream_id"))
    logger = object.__new__(GalileoObserve)
    logger.log_stream_id = log_stream_id
    build = cast(
        Callable[[list[dict[str, Json]]], dict[str, Json]],
        logger._build_traces_payload,  # pyright: ignore[reportPrivateUsage]
    )
    uuid_target = "litellm.integrations.galileo.uuid.uuid4"
    with patch(uuid_target, side_effect=[UUID(trace_id), UUID(span_id)]):
        return build([record])


def _kwargs_mapping(raw: dict[str, Json]) -> dict[str, Json]:
    optional_params: dict[str, Json] = {}
    if raw.get("functions") is not None:
        optional_params["functions"] = raw["functions"]
    if raw.get("tools") is not None:
        optional_params["tools"] = raw["tools"]
    return {
        "call_type": raw.get("call_type"),
        "messages": raw.get("messages"),
        "optional_params": optional_params,
        "input": raw.get("input"),
    }


def _selection_result(input_text: str, output_text: Json, messages_for_span: Json) -> Json:
    return {
        "input_text": input_text,
        "output_text": output_text,
        "messages_for_span": messages_for_span,
    }


def _content_selection(input_value: dict[str, Json]) -> Json:
    kwargs = _kwargs_mapping(cast("dict[str, Json]", input_value.get("kwargs") or {}))
    response = cast("dict[str, Json] | None", input_value.get("response"))
    level = "ERROR" if input_value.get("level") == "error" else "DEFAULT"
    status_message = cast("str | None", input_value.get("status_message"))

    if response is None:
        return _invoke_full(kwargs, None, level, status_message)

    family = cast(str, response["family"])
    response_obj = _dispatchable_response(family, response)
    return _invoke_full(kwargs, response_obj, level, status_message)


def _speech_response(_response: dict[str, Json]) -> object:
    return HttpxBinaryResponseContent(httpx.Response(200, content=b"speech-bytes"))


def _chat_response(response: dict[str, Json]) -> object:
    return litellm.ModelResponse(choices=[{"message": response.get("message")}])


def _text_completion_response(response: dict[str, Json]) -> object:
    return litellm.TextCompletionResponse(choices=[{"text": response.get("text")}])


def _image_response(response: dict[str, Json]) -> object:
    return litellm.ImageResponse(data=cast("list[ImageObject] | None", response.get("data")))


def _transcription_response(response: dict[str, Json]) -> object:
    return litellm.TranscriptionResponse(text=cast("str | None", response.get("text")))


def _rerank_response(response: dict[str, Json]) -> object:
    return litellm.RerankResponse(
        results=cast("list[RerankResponseResult] | None", response.get("results"))
    )


def _responses_api_response(response: dict[str, Json]) -> object:
    return ResponsesAPIResponse(
        id="resp_1",
        created_at=0,
        output=cast("list[ResponseOutputItem | dict[str, Json]]", response.get("output") or []),
    )


def _dict_response(response: dict[str, Json]) -> object:
    return cast("dict[str, Json]", response.get("value") or {})


def _realtime_list_response(response: dict[str, Json]) -> object:
    return cast("list[Json]", response.get("items") or [])


_RESPONSE_BUILDERS: dict[str, Callable[[dict[str, Json]], object]] = {
    "embedding": lambda _response: object(),
    "speech": _speech_response,
    "chat": _chat_response,
    "text_completion": _text_completion_response,
    "image": _image_response,
    "transcription": _transcription_response,
    "rerank": _rerank_response,
    "responses_api": _responses_api_response,
    "pass_through_dict": _dict_response,
    "generic_dict": _dict_response,
    "realtime_list": _realtime_list_response,
}


def _dispatchable_response(family: str, response: dict[str, Json]) -> object:
    builder = _RESPONSE_BUILDERS.get(family)
    if builder is None:
        message = f"unsupported dispatchable family: {family}"
        raise ValueError(message)
    return builder(response)


def _invoke_full(
    kwargs: dict[str, Json], response_obj: object, level: str, status_message: str | None
) -> Json:
    logger = object.__new__(GalileoObserve)
    invoke = cast(
        Callable[[dict[str, Json], object, str, "str | None"], tuple[str, str, Json]],
        logger._get_galileo_input_output_content,  # pyright: ignore[reportPrivateUsage]
    )
    input_text, output_text, messages_for_span = invoke(kwargs, response_obj, level, status_message)
    return _selection_result(input_text, output_text, messages_for_span)
