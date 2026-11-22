import json
import logging
from collections.abc import Awaitable, Mapping
from typing import Final

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.arguments import (
    JSON,
    MESSAGES,
    CallValue,
    SessionLike,
    TimeoutValues,
    messages_value,
    response_format_value,
    settings_snapshot,
    timeout_value,
)
from betterlite._completion.errors import UnsupportedCallArgument, raise_native_error
from betterlite._completion.mock import (
    araise_mock_timeout,
    mock_response_value,
    provider_name,
    raise_mock_timeout,
)
from betterlite._completion.stream import (
    CompletionOutcome,
    CustomStreamWrapper,
    response_from_wire,
)
from betterlite._completion.wire import RESPONSE
from betterlite._native import NativeChatStream, NativeError, NativeSession
from betterlite.llms.base_llm.base_utils import ResponseFormatModel
from betterlite.types.utils import Message, ModelResponse

LOGGER: Final = logging.getLogger("LiteLLM")
# LiteLLM's own bookkeeping, never a provider argument: the flag, the Python clients and the loggers
SKIPPED_KWARGS: Final[tuple[str, ...]] = (
    "acompletion", "client", "litellm_logging_obj", "logger_fn", "mock_response",
)
MODEL: Final[TypeAdapter[str | None]] = TypeAdapter(str | None)


async def settle_outcome(
    outcome: CompletionOutcome | Awaitable[CompletionOutcome],
) -> CompletionOutcome:
    # LiteLLM keeps a result it cannot await, so a sync call still answers the async retry
    match outcome:
        case ModelResponse() | CustomStreamWrapper():
            return outcome
        case Awaitable():
            return await outcome


def outcome_from_native(answer: str | NativeChatStream) -> CompletionOutcome:
    match answer:
        case str():
            return response_from_wire(RESPONSE.validate_json(answer))
        case NativeChatStream():
            return CustomStreamWrapper(answer)


def call_sync(text: str) -> CompletionOutcome:
    answer: str | NativeChatStream
    try:
        answer = NativeSession("{}").completion(text)
    except NativeError as error:
        raise_native_error(error)
    return outcome_from_native(answer)


async def call_async(text: str) -> CompletionOutcome:
    answer: str | NativeChatStream
    try:
        answer = await NativeSession("{}").acompletion(text)
    except NativeError as error:
        raise_native_error(error)
    return outcome_from_native(answer)


def call_arguments_from(
    args: tuple[JsonValue, ...], kwargs: dict[str, CallValue]
) -> dict[str, CallValue]:
    # litellm.utils.acreate and the retry loop hand positional arguments to completion by name
    positional = dict(zip(("model", "messages", "timeout"), args, strict=False))
    return {**kwargs, **positional}


def reject_unsupported_arguments(values: dict[str, CallValue]) -> None:
    # the completion runs in Rust, so LiteLLM's Python client and session cannot be passed on
    for name in ("shared_session", "client"):
        if values.pop(name, None) is not None:
            raise UnsupportedCallArgument(name)


def completion_from_arguments(
    arguments: Mapping[str, CallValue],
) -> CompletionOutcome | Awaitable[CompletionOutcome]:
    values = dict(arguments)
    model = MODEL.validate_python(values.pop("model", None))
    if model is None:
        raise ValueError("model param not passed in.")
    reject_unsupported_arguments(values)
    logger_fn = values.pop("logger_fn", None)
    if logger_fn is not None:
        LOGGER.warning("logger_fn is not called: the completion runs in Rust")
    provider = provider_name(values.get("custom_llm_provider"))
    timeout = values.pop("timeout", None)
    forwarded_timeout = None if timeout is None else timeout_value(timeout)
    acompletion_flag = values.get("acompletion")
    if values.get("mock_timeout") is True and forwarded_timeout is not None:
        if acompletion_flag is True:
            return araise_mock_timeout(forwarded_timeout, model)
        raise_mock_timeout(forwarded_timeout, model)
    forwarded: dict[str, JsonValue] = {
        "model": model,
        "messages": MESSAGES.validate_python(values.pop("messages", [])),
    }
    response_format = values.pop("response_format", None)
    if response_format is not None:
        forwarded["response_format"] = response_format_value(response_format)
    if forwarded_timeout is not None:
        forwarded["timeout"] = forwarded_timeout
    mock_response = values.pop("mock_response", None)
    if mock_response is not None:
        forwarded["mock_response"] = mock_response_value(mock_response, model, provider)
    forwarded |= {
        name: JSON.validate_python(value)
        for name, value in values.items()
        if value is not None and name not in SKIPPED_KWARGS
    }
    payload: dict[str, JsonValue] = {"arguments": forwarded, "settings": settings_snapshot()}
    text = json.dumps(payload)
    if acompletion_flag is True:
        return call_async(text)
    return call_sync(text)


def completion(
    model: str | None,
    messages: list[dict[str, JsonValue] | Message] = [],
    timeout: float | str | TimeoutValues | None = None,
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
    values: list[tuple[str, CallValue]] = [
        ("model", model), ("messages", messages_value(messages)), ("timeout", timeout),
        ("temperature", temperature), ("top_p", top_p), ("n", n),
        ("stream", stream), ("stream_options", stream_options), ("stop", stop),
        ("max_completion_tokens", max_completion_tokens), ("max_tokens", max_tokens),
        ("modalities", modalities), ("prediction", prediction), ("audio", audio),
        ("presence_penalty", presence_penalty), ("frequency_penalty", frequency_penalty),
        ("logit_bias", logit_bias), ("user", user), ("reasoning_effort", reasoning_effort),
        ("verbosity", verbosity), ("seed", seed), ("tools", tools), ("tool_choice", tool_choice),
        ("logprobs", logprobs), ("top_logprobs", top_logprobs),
        ("parallel_tool_calls", parallel_tool_calls), ("web_search_options", web_search_options),
        # ast-grep-ignore: no-long-names
        ("include_server_side_tool_invocations", include_server_side_tool_invocations),
        ("deployment_id", deployment_id), ("extra_headers", extra_headers),
        ("safety_identifier", safety_identifier), ("service_tier", service_tier),
        ("store", store), ("prompt_cache_key", prompt_cache_key), ("functions", functions),
        ("function_call", function_call), ("base_url", base_url),
        ("api_version", api_version), ("api_key", api_key),
        ("model_list", model_list), ("thinking", thinking), ("shared_session", shared_session),
        ("enable_json_schema_validation", enable_json_schema_validation),
    ]
    arguments: dict[str, CallValue] = dict(values)
    if response_format is not None:
        arguments["response_format"] = response_format_value(response_format)
    arguments.update(kwargs)
    return completion_from_arguments(arguments)
