import json
from collections.abc import Callable
from datetime import datetime, timedelta, timezone
from types import SimpleNamespace
from typing import TypeAlias, cast
from unittest.mock import patch
from uuid import UUID

import httpx
from litellm.integrations.galileo import GalileoObserve
from litellm.types.llms.openai import HttpxBinaryResponseContent

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _AttrDict:
    """A minimal stand-in for a litellm response object that supports both
    attribute access (`.choices`) and subscript access (`["message"]`), the
    way `_get_chat_content_for_galileo` reads its argument."""

    def __init__(self, **fields: object) -> None:
        self.__dict__.update(fields)

    def __getitem__(self, key: str) -> object:
        return cast(object, getattr(self, key))


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
    named = {
        "chat": _chat_selection,
        "text_completion": _text_completion_selection,
        "image": _image_selection,
        "transcription": _transcription_selection,
        "rerank": _rerank_selection,
        "responses_api": _responses_api_selection,
    }
    if family in named:
        return named[family](response, kwargs)

    response_obj = _dispatchable_response(family, response)
    return _invoke_full(kwargs, response_obj, level, status_message)


def _dispatchable_response(family: str, response: dict[str, Json]) -> object:
    if family == "embedding":
        return object()
    if family == "speech":
        return HttpxBinaryResponseContent(httpx.Response(200, content=b"speech-bytes"))
    if family in ("pass_through_dict", "generic_dict"):
        return cast("dict[str, Json]", response.get("value") or {})
    if family == "realtime_list":
        return cast("list[Json]", response.get("items") or [])
    message = f"unsupported dispatchable family: {family}"
    raise ValueError(message)


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


def _build_prompt(kwargs: dict[str, Json]) -> dict[str, Json]:
    logger = object.__new__(GalileoObserve)
    build = cast(
        Callable[[dict[str, Json]], dict[str, Json]],
        logger._build_prompt,  # pyright: ignore[reportPrivateUsage]
    )
    return build(kwargs)


def _prompt_to_input_text(prompt: dict[str, Json]) -> str:
    logger = object.__new__(GalileoObserve)
    to_text = cast(
        Callable[[dict[str, Json]], str],
        logger._prompt_to_input_text,  # pyright: ignore[reportPrivateUsage]
    )
    return to_text(prompt)


def _serialize(value: Json) -> str:
    logger = object.__new__(GalileoObserve)
    serialize = cast(
        Callable[[Json], str],
        logger._serialize_galileo_output,  # pyright: ignore[reportPrivateUsage]
    )
    return serialize(value)


def _chat_selection(response: dict[str, Json], kwargs: dict[str, Json]) -> Json:
    prompt = _build_prompt(kwargs)
    choice = _AttrDict(message=response.get("message"))
    response_obj = _AttrDict(choices=[choice])
    logger = object.__new__(GalileoObserve)
    extract = cast(
        Callable[[object], Json],
        logger._get_chat_content_for_galileo,  # pyright: ignore[reportPrivateUsage]
    )
    output = _serialize(extract(response_obj))
    return _selection_result(_prompt_to_input_text(prompt), output, kwargs.get("messages") or [])


def _text_completion_selection(response: dict[str, Json], kwargs: dict[str, Json]) -> Json:
    prompt = _build_prompt(kwargs)
    stub = SimpleNamespace(choices=[SimpleNamespace(text=response.get("text"))])
    logger = object.__new__(GalileoObserve)
    extract = cast(
        Callable[[object], "str | None"],
        logger._get_text_completion_content_for_galileo,  # pyright: ignore[reportPrivateUsage]
    )
    output = _serialize(extract(stub))
    return _selection_result(_prompt_to_input_text(prompt), output, kwargs.get("messages") or [])


def _image_selection(response: dict[str, Json], kwargs: dict[str, Json]) -> Json:
    prompt = _build_prompt(kwargs)
    output = _serialize(response.get("data"))
    return _selection_result(_prompt_to_input_text(prompt), output, prompt)


def _transcription_selection(response: dict[str, Json], kwargs: dict[str, Json]) -> Json:
    prompt = _build_prompt(kwargs)
    output = _serialize(response.get("text"))
    return _selection_result(_prompt_to_input_text(prompt), output, prompt)


def _rerank_selection(response: dict[str, Json], kwargs: dict[str, Json]) -> Json:
    logger = object.__new__(GalileoObserve)
    rerank_prompt_fn = cast(
        Callable[[dict[str, Json]], dict[str, Json]],
        logger._langfuse_style_rerank_prompt,  # pyright: ignore[reportPrivateUsage]
    )
    rerank_prompt = rerank_prompt_fn(kwargs)
    output = _serialize(response.get("results"))
    input_text = json.dumps(rerank_prompt, default=str)
    return _selection_result(input_text, output, rerank_prompt)


def _responses_api_selection(response: dict[str, Json], kwargs: dict[str, Json]) -> Json:
    prompt = _build_prompt(kwargs)
    stub = SimpleNamespace(output=response.get("output"))
    logger = object.__new__(GalileoObserve)
    extract = cast(
        Callable[[object], Json],
        logger._get_responses_api_content_for_galileo,  # pyright: ignore[reportPrivateUsage]
    )
    output = _serialize(extract(stub))
    return _selection_result(_prompt_to_input_text(prompt), output, kwargs.get("messages") or [])
