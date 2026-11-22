import json
import logging
from collections.abc import Awaitable
from typing import Final

from pydantic import JsonValue

from betterlite import _native
from betterlite._completion.arguments import (
    JSON, CallValue, SessionLike, TimeoutValues, messages_value, response_format_value,
    settings_snapshot, timeout_value,
)
from betterlite._completion.errors import UnsupportedCallArgument, raise_native_error
from betterlite._completion.mock import (
    araise_mock_timeout, mock_response_value, provider_name, raise_mock_timeout,
)
from betterlite._completion.stream import (
    CompletionOutcome, CustomStreamWrapper, response_from_text,
)
from betterlite.llms.base_llm.base_utils import ResponseFormatModel
from betterlite.types.utils import Message

LOGGER: Final = logging.getLogger("LiteLLM")
# LiteLLM's own bookkeeping, never a provider argument: the flag, the Python clients and the loggers
SKIPPED_KWARGS: Final[tuple[str, ...]] = (
    "acompletion", "client", "litellm_logging_obj", "logger_fn", "mock_response",
)


def request_text(arguments: dict[str, JsonValue], settings: dict[str, JsonValue]) -> str:
    wire: dict[str, JsonValue] = {"arguments": arguments, "settings": settings}
    return json.dumps(wire)


async def settle_outcome(
    outcome: CompletionOutcome | Awaitable[CompletionOutcome],
) -> CompletionOutcome:
    # LiteLLM keeps a result it cannot await, so a sync call still answers the async retry
    if isinstance(outcome, Awaitable):
        return await outcome
    return outcome


def outcome_from_native(answer: str | _native.NativeChatStream) -> CompletionOutcome:
    match answer:
        case str():
            return response_from_text(answer)
        case _native.NativeChatStream():
            return CustomStreamWrapper(answer)


def call_sync(text: str) -> CompletionOutcome:
    session = _native.NativeSession("{}")
    answer: str | _native.NativeChatStream
    try:
        answer = session.completion(text)
    except _native.NativeError as error:
        raise_native_error(error)
    return outcome_from_native(answer)


async def call_async(text: str) -> CompletionOutcome:
    session = _native.NativeSession("{}")
    answer: str | _native.NativeChatStream
    try:
        answer = await session.acompletion(text)
    except _native.NativeError as error:
        raise_native_error(error)
    return outcome_from_native(answer)


def call_arguments(
    model: str,
    messages: JsonValue,
    timeout: float | int | str | TimeoutValues | None,
    response_format: dict[str, JsonValue] | type[ResponseFormatModel] | None,
    values: list[tuple[str, JsonValue]],
) -> dict[str, JsonValue]:
    arguments: dict[str, JsonValue] = {"model": model, "messages": messages}
    if timeout is not None:
        arguments["timeout"] = timeout_value(timeout)
    if response_format is not None:
        arguments["response_format"] = response_format_value(response_format)
    for name, value in values:
        if value is not None:
            arguments[name] = value
    return arguments


def completion(
    model: str | None,
    messages: list[dict[str, JsonValue] | Message] = [],
    timeout: float | int | str | TimeoutValues | None = None,
    temperature: JsonValue = None,
    top_p: JsonValue = None,
    n: JsonValue = None,
    stream: JsonValue = None,
    stream_options: JsonValue = None,
    stop: JsonValue = None,
    max_completion_tokens: JsonValue = None,
    max_tokens: JsonValue = None,
    modalities: JsonValue = None,
    prediction: JsonValue = None,
    audio: JsonValue = None,
    presence_penalty: JsonValue = None,
    frequency_penalty: JsonValue = None,
    logit_bias: JsonValue = None,
    user: JsonValue = None,
    reasoning_effort: JsonValue = None,
    verbosity: JsonValue = None,
    response_format: dict[str, JsonValue] | type[ResponseFormatModel] | None = None,
    seed: JsonValue = None,
    tools: JsonValue = None,
    tool_choice: JsonValue = None,
    logprobs: JsonValue = None,
    top_logprobs: JsonValue = None,
    parallel_tool_calls: JsonValue = None,
    web_search_options: JsonValue = None,
    include_server_side_tool_invocations: JsonValue = None,  # ast-grep-ignore: no-long-names
    deployment_id: JsonValue = None,
    extra_headers: JsonValue = None,
    safety_identifier: JsonValue = None,
    service_tier: JsonValue = None,
    store: JsonValue = None,
    prompt_cache_key: JsonValue = None,
    functions: JsonValue = None,
    function_call: JsonValue = None,
    base_url: JsonValue = None,
    api_version: JsonValue = None,
    api_key: JsonValue = None,
    model_list: JsonValue = None,
    thinking: JsonValue = None,
    shared_session: SessionLike | None = None,
    enable_json_schema_validation: JsonValue = None,
    **kwargs: CallValue,
) -> CompletionOutcome | Awaitable[CompletionOutcome]:
    if model is None:
        raise ValueError("model param not passed in.")
    if shared_session is not None:
        raise UnsupportedCallArgument("shared_session")
    if kwargs.get("client") is not None:
        raise UnsupportedCallArgument("client")
    if kwargs.get("logger_fn") is not None:
        LOGGER.warning("logger_fn is not called: the completion runs in Rust")
    provider = provider_name(kwargs.get("custom_llm_provider"))
    if kwargs.get("mock_timeout") is True and timeout is not None:
        if kwargs.get("acompletion") is True:
            return araise_mock_timeout(timeout, model)
        raise_mock_timeout(timeout, model)
    values: list[tuple[str, JsonValue]] = [
        ("temperature", temperature), ("top_p", top_p), ("n", n),
        ("stream", stream), ("stream_options", stream_options), ("stop", stop),
        ("max_completion_tokens", max_completion_tokens), ("max_tokens", max_tokens),
        ("modalities", modalities), ("prediction", prediction), ("audio", audio),
        ("presence_penalty", presence_penalty), ("frequency_penalty", frequency_penalty),
        ("logit_bias", logit_bias), ("user", user),
        ("reasoning_effort", reasoning_effort), ("verbosity", verbosity),
        ("seed", seed), ("tools", tools), ("tool_choice", tool_choice),
        ("logprobs", logprobs), ("top_logprobs", top_logprobs),
        ("parallel_tool_calls", parallel_tool_calls),
        ("web_search_options", web_search_options),
        # ast-grep-ignore: no-long-names
        ("include_server_side_tool_invocations", include_server_side_tool_invocations),
        ("deployment_id", deployment_id), ("extra_headers", extra_headers),
        ("safety_identifier", safety_identifier), ("service_tier", service_tier),
        ("store", store), ("prompt_cache_key", prompt_cache_key),
        ("functions", functions), ("function_call", function_call),
        ("base_url", base_url), ("api_version", api_version), ("api_key", api_key),
        ("model_list", model_list), ("thinking", thinking),
        ("enable_json_schema_validation", enable_json_schema_validation),
    ]
    arguments = call_arguments(
        model, messages_value(messages), timeout, response_format, values
    )
    mock_response = kwargs.get("mock_response")
    if mock_response is not None:
        arguments["mock_response"] = mock_response_value(mock_response, model, provider)
    for name, value in kwargs.items():
        if name in SKIPPED_KWARGS:
            continue
        arguments[name] = JSON.validate_python(value)
    text = request_text(arguments, settings_snapshot())
    if kwargs.get("acompletion") is True:
        return call_async(text)
    return call_sync(text)
