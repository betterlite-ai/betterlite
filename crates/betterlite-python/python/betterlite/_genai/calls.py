from collections.abc import Awaitable, Mapping
from typing import Final

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.arguments import TimeoutValues
from betterlite._completion.errors import raise_native_error
from betterlite._endpoints.payload import CallValue, to_call_payload
from betterlite._endpoints.runner import acall_native, call_native
from betterlite._genai.arguments import ASYNC_FLAGS, call_values, session_settings
from betterlite._genai.stream import GenaiStream
from betterlite._native import NativeGenaiError, NativeGenaiSession

JSON_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


# a non-stream answer is the response body LiteLLM hands back as a provider object's model_dump
def content_result(values: Mapping[str, CallValue]) -> dict[str, JsonValue]:
    session = NativeGenaiSession(session_settings())
    answer = call_native(session.generate_content, to_call_payload(values), NativeGenaiError)
    return JSON_OBJECT.validate_python(answer)


async def content_result_async(values: Mapping[str, CallValue]) -> dict[str, JsonValue]:
    session = NativeGenaiSession(session_settings())
    answer = await acall_native(
        session.agenerate_content, to_call_payload(values), NativeGenaiError
    )
    return JSON_OBJECT.validate_python(answer)


# a stream's chunks are not JSON, so its native errors are mapped here instead of by the runner
async def stream_result_async(values: Mapping[str, CallValue]) -> GenaiStream:
    session = NativeGenaiSession(session_settings())
    payload = to_call_payload(values)
    try:
        native = await session.agenerate_content_stream(payload.text, payload.blobs)
    except NativeGenaiError as failure:
        raise_native_error(failure)
    return GenaiStream(native)


def generate_content(
    model: str, contents: CallValue, config: CallValue = None, tools: CallValue = None,
    extra_headers: Mapping[str, str] | None = None, extra_query: CallValue = None,
    extra_body: CallValue = None, timeout: TimeoutValues | float | str | None = None,
    custom_llm_provider: str | None = None, **kwargs: CallValue,
) -> dict[str, JsonValue] | Awaitable[dict[str, JsonValue]]:
    asynchronous = kwargs.pop(ASYNC_FLAGS[0], False) is True
    values = call_values(
        model, contents, config, tools, extra_headers, extra_query, extra_body, timeout,
        custom_llm_provider, kwargs,
    )
    if asynchronous:
        return content_result_async(values)
    return content_result(values)


def generate_content_stream(
    model: str, contents: CallValue, config: CallValue = None, tools: CallValue = None,
    extra_headers: Mapping[str, str] | None = None, extra_query: CallValue = None,
    extra_body: CallValue = None, timeout: TimeoutValues | float | str | None = None,
    custom_llm_provider: str | None = None, **kwargs: CallValue,
) -> GenaiStream | Awaitable[GenaiStream]:
    asynchronous = kwargs.pop(ASYNC_FLAGS[1], False) is True
    values = call_values(
        model, contents, config, tools, extra_headers, extra_query, extra_body, timeout,
        custom_llm_provider, kwargs,
    )
    if asynchronous:
        return stream_result_async(values)
    session = NativeGenaiSession(session_settings())
    payload = to_call_payload(values)
    try:
        native = session.generate_content_stream(payload.text, payload.blobs)
    except NativeGenaiError as failure:
        raise_native_error(failure)
    return GenaiStream(native)


async def agenerate_content(
    model: str, contents: CallValue, config: CallValue = None, tools: CallValue = None,
    extra_headers: Mapping[str, str] | None = None, extra_query: CallValue = None,
    extra_body: CallValue = None, timeout: TimeoutValues | float | str | None = None,
    custom_llm_provider: str | None = None, **kwargs: CallValue,
) -> dict[str, JsonValue]:
    values = call_values(
        model, contents, config, tools, extra_headers, extra_query, extra_body, timeout,
        custom_llm_provider, kwargs,
    )
    session = NativeGenaiSession(session_settings())
    answer = await acall_native(
        session.agenerate_content, to_call_payload(values), NativeGenaiError
    )
    return JSON_OBJECT.validate_python(answer)


async def agenerate_content_stream(
    model: str, contents: CallValue, config: CallValue = None, tools: CallValue = None,
    extra_headers: Mapping[str, str] | None = None, extra_query: CallValue = None,
    extra_body: CallValue = None, timeout: TimeoutValues | float | str | None = None,
    custom_llm_provider: str | None = None, **kwargs: CallValue,
) -> GenaiStream:
    values = call_values(
        model, contents, config, tools, extra_headers, extra_query, extra_body, timeout,
        custom_llm_provider, kwargs,
    )
    return await stream_result_async(values)
