from collections.abc import Awaitable, Mapping
from typing import Final, NotRequired, TypedDict

from pydantic import JsonValue, TypeAdapter

from betterlite._completion import CustomStreamWrapper
from betterlite._completion.completion import completion_from_arguments, settle_outcome
from betterlite._completion.stream import CompletionOutcome, usage_from_wire
from betterlite._completion.wire import WireUsage
from betterlite._endpoints import (
    CallPayload,
    CallValue,
    acall_native,
    call_native,
    model_from_arguments,
    read_settings,
    to_call_payload,
)
from betterlite._native import NativeTextError, NativeTextSession
from betterlite._text.stream import ChunkList, TextCompletionStreamWrapper
from betterlite.types.utils import ModelResponse, TextChoices, TextCompletionResponse
from betterlite.types.utils.text_completion import HiddenParams

SETTINGS: Final[tuple[str, ...]] = ("api_base", "api_key", "openai_key")
# text_completion's own parameters, in the order atext_completion's positional arguments take them
NAMED_ARGUMENTS: Final[tuple[str, ...]] = (
    "prompt", "model", "best_of", "echo", "frequency_penalty", "logit_bias", "logprobs",
    "max_tokens", "n", "presence_penalty", "stop", "stream", "stream_options", "suffix",
    "temperature", "top_p", "user", "api_base", "api_version", "api_key", "model_list",
    "custom_llm_provider",
)
# names the payload or the chat fallback owns: they never travel as provider parameters
SKIPPED_KWARGS: Final[tuple[str, ...]] = (
    "acompletion",
    "client",
    "litellm_logging_obj",
    "logger_fn",
    "model_list",
    "prompt",
    "shared_session",
    "text_completion",
)
UNKNOWN_ANSWER: Final = "the native text answer names no known kind"
CHAT_AWAITABLE: Final = "the chat fallback answered an awaitable to a synchronous call"

TEXT: Final[TypeAdapter[str | None]] = TypeAdapter(str | None)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
MESSAGES: Final[TypeAdapter[list[JsonValue]]] = TypeAdapter(list[JsonValue])
CHUNKS: Final[TypeAdapter[list[str]]] = TypeAdapter(list[str])
OPTIONS: Final[TypeAdapter[dict[str, JsonValue] | None]] = TypeAdapter(dict[str, JsonValue] | None)


class TextChoiceWire(TypedDict):
    index: int
    text: NotRequired[str | None]
    finish_reason: NotRequired[str | None]
    logprobs: NotRequired[JsonValue]


class TextResponseWire(TypedDict):
    id: str
    created: int
    model: NotRequired[str | None]
    choices: list[TextChoiceWire]
    usage: NotRequired[WireUsage | None]


RESPONSE: Final[TypeAdapter[TextResponseWire]] = TypeAdapter(TextResponseWire)


def text_completion(
    prompt: JsonValue,
    model: str | None = None,
    best_of: int | None = None,
    echo: bool | None = None,
    frequency_penalty: float | None = None,
    logit_bias: JsonValue = None,
    logprobs: int | None = None,
    max_tokens: int | None = None,
    n: int | None = None,
    presence_penalty: float | None = None,
    stop: JsonValue = None,
    stream: bool | None = None,
    stream_options: JsonValue = None,
    suffix: str | None = None,
    temperature: float | None = None,
    top_p: float | None = None,
    user: str | None = None,
    api_base: str | None = None,
    api_version: str | None = None,
    api_key: str | None = None,
    model_list: JsonValue = None,
    custom_llm_provider: str | None = None,
    *args: JsonValue,
    **kwargs: CallValue,
) -> TextCompletionResponse | TextCompletionStreamWrapper:
    # main.py reads engine as the model when the caller passed no model
    match kwargs.pop("engine", None):
        case str() as engine if model is None:
            model = engine
        case object():
            pass
    named: dict[str, CallValue] = {
        "prompt": prompt, "model": model, "best_of": best_of, "echo": echo,
        "frequency_penalty": frequency_penalty, "logit_bias": logit_bias,
        "logprobs": logprobs, "max_tokens": max_tokens, "n": n,
        "presence_penalty": presence_penalty, "stop": stop, "stream": stream,
        "stream_options": stream_options, "suffix": suffix, "temperature": temperature,
        "top_p": top_p, "user": user, "api_base": api_base, "api_version": api_version,
        "api_key": api_key, "model_list": model_list,
        "custom_llm_provider": custom_llm_provider,
    }
    # main.py hands its own *args to completion; betterlite has none left for them
    _ = args
    return text_from_values({**kwargs, **named})


async def atext_completion(
    *args: JsonValue, **kwargs: CallValue
) -> TextCompletionResponse | TextCompletionStreamWrapper:
    # main.py reads the model before it awaits, so a call with no model raises at the call
    _ = model_from_arguments(args, kwargs)
    positional = dict(zip(NAMED_ARGUMENTS, args, strict=False))
    return await text_from_values_async({**kwargs, **positional})


def text_from_values(
    values: Mapping[str, CallValue],
) -> TextCompletionResponse | TextCompletionStreamWrapper:
    answer = call_native(
        NativeTextSession("{}").text_completion, text_payload(values), NativeTextError
    )
    return sync_answer(answer, values)


async def text_from_values_async(
    values: Mapping[str, CallValue],
) -> TextCompletionResponse | TextCompletionStreamWrapper:
    answer = await acall_native(
        NativeTextSession("{}").atext_completion, text_payload(values), NativeTextError
    )
    return await async_answer(answer, values)


def text_payload(values: Mapping[str, CallValue]) -> CallPayload:
    arguments = passing_arguments(values)
    # a missing prompt travels as null, the value main.py's dispatch refuses
    arguments["prompt"] = values.get("prompt")
    payload: dict[str, CallValue] = {"arguments": arguments, "settings": read_settings(SETTINGS)}
    # the top-level names are the payload's own: it drops the logger, refuses a client and warns
    payload["client"] = values.get("client")
    payload["logger_fn"] = values.get("logger_fn")
    payload["shared_session"] = values.get("shared_session")
    return to_call_payload(payload)


def passing_arguments(values: Mapping[str, CallValue]) -> dict[str, CallValue]:
    return {
        name: value
        for name, value in values.items()
        if value is not None and name not in SKIPPED_KWARGS
    }


# main.py builds the chat call's messages without the prompt or the parameters it owns
def chat_arguments(
    values: Mapping[str, CallValue], messages: list[JsonValue]
) -> dict[str, CallValue]:
    arguments = passing_arguments(values)
    arguments["messages"] = messages
    return arguments


def sync_answer(
    answer: JsonValue, values: Mapping[str, CallValue]
) -> TextCompletionResponse | TextCompletionStreamWrapper:
    match JSON.validate_python(answer):
        case {"kind": "chat", "messages": messages}:
            outcome = completion_from_arguments(
                chat_arguments(values, MESSAGES.validate_python(messages))
            )
            return sync_outcome(outcome, values)
        case {"kind": "text", "response": response}:
            return buffered_text(RESPONSE.validate_python(response))
        case {"kind": "chunks", "chunks": chunks}:
            return text_stream(
                CustomStreamWrapper(ChunkList(CHUNKS.validate_python(chunks))), values
            )
        case object():
            raise ValueError(UNKNOWN_ANSWER)


async def async_answer(
    answer: JsonValue, values: Mapping[str, CallValue]
) -> TextCompletionResponse | TextCompletionStreamWrapper:
    match JSON.validate_python(answer):
        case {"kind": "chat", "messages": messages}:
            arguments = chat_arguments(values, MESSAGES.validate_python(messages))
            arguments["acompletion"] = True
            settled = await settle_outcome(completion_from_arguments(arguments))
            return settled_answer(settled, values)
        case {"kind": "text", "response": response}:
            return buffered_text(RESPONSE.validate_python(response))
        case {"kind": "chunks", "chunks": chunks}:
            return text_stream(
                CustomStreamWrapper(ChunkList(CHUNKS.validate_python(chunks))), values
            )
        case object():
            raise ValueError(UNKNOWN_ANSWER)


def sync_outcome(
    outcome: CompletionOutcome | Awaitable[CompletionOutcome],
    values: Mapping[str, CallValue],
) -> TextCompletionResponse | TextCompletionStreamWrapper:
    match outcome:
        case ModelResponse():
            return text_from_chat(outcome)
        case CustomStreamWrapper():
            return text_stream(outcome, values)
        case Awaitable():
            raise ValueError(CHAT_AWAITABLE)


def settled_answer(
    outcome: CompletionOutcome, values: Mapping[str, CallValue]
) -> TextCompletionResponse | TextCompletionStreamWrapper:
    match outcome:
        case ModelResponse():
            return text_from_chat(outcome)
        case CustomStreamWrapper():
            return text_stream(outcome, values)


def text_stream(
    stream: CustomStreamWrapper, values: Mapping[str, CallValue]
) -> TextCompletionStreamWrapper:
    return TextCompletionStreamWrapper(
        completion_stream=stream,
        model=TEXT.validate_python(values.get("model")),
        stream_options=OPTIONS.validate_python(values.get("stream_options")),
        custom_llm_provider=TEXT.validate_python(values.get("custom_llm_provider")),
    )


def buffered_text(wire: TextResponseWire) -> TextCompletionResponse:
    choices = [
        TextChoices(
            index=choice["index"],
            text=choice.get("text"),
            finish_reason=choice.get("finish_reason"),
            logprobs=choice.get("logprobs"),
        )
        for choice in wire["choices"]
    ]
    usage = wire.get("usage")
    return TextCompletionResponse(
        id=wire["id"],
        created=wire["created"],
        model=wire.get("model"),
        choices=choices,
        usage=None if usage is None else usage_from_wire(usage),
    )


# convert_dict_to_response.py's convert_chat_to_text_completion, field by field
def text_from_chat(chat: ModelResponse) -> TextCompletionResponse:
    choices = [
        TextChoices(
            index=choice.index,
            text=choice.message.content,
            finish_reason=choice.finish_reason,
            logprobs=None,
        )
        for choice in chat.choices
    ]
    response = TextCompletionResponse(choices=choices)
    response["id"] = chat["id"]
    response["created"] = chat["created"]
    response["model"] = chat["model"]
    response["usage"] = chat.get("usage")
    response["_hidden_params"] = HiddenParams.model_validate(chat.get("_hidden_params"))
    return response
