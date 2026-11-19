from datetime import UTC, datetime
from typing import cast

import httpx
from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import JsonValue
from typing_extensions import override

from .fixture import ScriptedError
from .types import (
    CHUNK,
    CREATED_FLOOR,
    JSON,
    MODEL,
    PRE_ARGS,
    AsyncStream,
    ChunkView,
    ErrorView,
    SyncStream,
    WrapperView,
)


def pre_view(params: dict[str, JsonValue]) -> JsonValue:
    additional = PRE_ARGS.validate_python(params.get("additional_args"))
    return {
        "input": params.get("input"),
        "api_key": params.get("api_key"),
        "headers": additional.headers,
        "api_base": additional.api_base,
        "acompletion": additional.acompletion,
        "complete_input_dict": additional.complete_input_dict,
    }


class Recorder(Logging):
    def __init__(self) -> None:
        super().__init__(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L576
            model=MODEL,
            messages=None,
            stream=True,
            call_type="completion",
            start_time=datetime(2024, 1, 2, tzinfo=UTC),
            litellm_call_id="call",
            function_id="function",
        )
        self.pre: list[JsonValue] = []

    @override
    def pre_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args
        self.pre.append(pre_view(params))


def stored_headers(recorder: Recorder) -> JsonValue:
    details: dict[str, JsonValue] = recorder.model_call_details
    return JSON.validate_python(details.get("response_headers"))


def wrapper_view(result: object) -> JsonValue:
    view = cast(WrapperView, result)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L208-L299
    options = view.stream_options
    if options is None:
        stream_options: JsonValue = None
    else:
        stream_options = {"include_usage": options.get("include_usage") is True}
    return {
        "model": view.model,
        "provider": view.custom_llm_provider,
        "stream_options": stream_options,
        "response_headers": JSON.validate_python(view._response_headers),
        "additional_headers": JSON.validate_python(view._hidden_params.get("additional_headers")),
    }


def chunk_view(view: ChunkView) -> JsonValue:
    choice = view["choices"][0]
    return {
        "content": choice["delta"].get("content"),
        "role": choice["delta"].get("role"),
        "finish_reason": choice["finish_reason"],
        "index": choice["index"],
        "model": view["model"],
        "created_is_current": view["created"] > CREATED_FLOOR,
    }


async def collect(result: object, *, asynchronous: bool) -> list[JsonValue]:
    if asynchronous:
        stream = cast(AsyncStream, result)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L1978-L1990
        return [
            chunk_view(CHUNK.validate_json(chunk.model_dump_json())) async for chunk in stream
        ]
    buffered = cast(SyncStream, result)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L1768-L1775
    return [chunk_view(CHUNK.validate_json(chunk.model_dump_json())) for chunk in buffered]


def header_map(headers: httpx.Headers | dict[str, str] | None) -> JsonValue:
    if headers is None:
        return None
    return dict(headers.items())


def azure_error_view(error: AzureOpenAIError) -> JsonValue:
    view = cast(ErrorView, cast(object, error))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46-L63
    return {
        "class": type(error).__name__,
        "status_present": True,
        "status": view.status_code,
        "message_present": True,
        "message": view.message,
        "headers": header_map(view.headers),
        "body": JSON.validate_python(view.body),
    }


def scripted_status(error: ScriptedError) -> tuple[JsonValue, bool]:
    try:
        value: JsonValue = error.status_code
    except AttributeError:
        return None, False
    else:
        return value, True


def scripted_message(error: ScriptedError) -> tuple[JsonValue, bool]:
    try:
        value: JsonValue = error.message
    except AttributeError:
        return None, False
    else:
        return value, True


def scripted_headers(error: ScriptedError) -> JsonValue:
    try:
        value = header_map(error.headers)
    except AttributeError:
        return None
    else:
        return value


def scripted_body(error: ScriptedError) -> JsonValue:
    try:
        value = JSON.validate_python(error.body)
    except AttributeError:
        return None
    else:
        return value


def scripted_error_view(error: ScriptedError) -> JsonValue:
    status, status_present = scripted_status(error)
    message, message_present = scripted_message(error)
    return {
        "class": type(error).__name__,
        "status_present": status_present,
        "status": status,
        "message_present": message_present,
        "message": message,
        "headers": scripted_headers(error),
        "body": scripted_body(error),
    }
