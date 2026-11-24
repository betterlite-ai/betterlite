import asyncio
from collections.abc import AsyncIterator, Coroutine
from typing import (
    Final,
    Literal,
    NotRequired,
    Protocol,
    TypeAlias,
    TypedDict,
    Unpack,
    cast,
)

import litellm
from litellm.litellm_core_utils import exception_mapping_utils
from litellm.llms.anthropic.pass_through.responses_adapters.handler import (
    LiteLLMMessagesToResponsesAPIHandler as MessagesHandler,
)
from litellm.types.llms.openai import ResponsesAPIResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

ANSWER: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
RESPONSE: Final[TypeAdapter[ResponsesAPIResponse]] = TypeAdapter(ResponsesAPIResponse)


class Request(TypedDict):
    model: str
    messages: JsonValue
    max_tokens: int
    system: NotRequired[str]
    temperature: NotRequired[float]
    top_p: NotRequired[float]
    thinking: NotRequired[dict[str, JsonValue]]
    output_config: NotRequired[dict[str, JsonValue]]
    output_format: NotRequired[dict[str, JsonValue]]
    metadata: NotRequired[dict[str, JsonValue]]
    tool_choice: NotRequired[dict[str, JsonValue]]
    tools: NotRequired[JsonValue]
    context_management: NotRequired[dict[str, JsonValue]]


class Forwarded(TypedDict):
    custom_llm_provider: NotRequired[str]
    api_base: NotRequired[str]
    prompt_cache_key: NotRequired[str]
    include: NotRequired[JsonValue]


class Call(TypedDict):
    request: Request
    stream: NotRequired[bool]
    settings: NotRequired[dict[str, JsonValue]]
    forwarded: NotRequired[Forwarded]


class AnswerSpec(TypedDict):
    shape: Literal["response", "stream", "other"]
    body: NotRequired[JsonValue]
    events: NotRequired[list[dict[str, JsonValue]]]
    kind: NotRequired[str]


class Input(TypedDict):
    call: Call
    include_supported: bool
    provider: NotRequired[str | None]
    model_info: NotRequired[dict[str, JsonValue] | None]
    answer: AnswerSpec


class Frame(TypedDict):
    event: str
    data: JsonValue


class Output(TypedDict):
    request: JsonValue
    native: NotRequired[JsonValue]
    frames: NotRequired[list[Frame]]


HandlerResult: TypeAlias = AsyncIterator[bytes] | JsonValue | ResponsesAPIResponse
EMPTY_EVENTS: Final[list[dict[str, JsonValue]]] = []


class ScriptedStream(AsyncIterator[dict[str, JsonValue]]):
    def __init__(self, events: list[dict[str, JsonValue]]) -> None:
        self._events = events
        self._index = 0

    @override
    async def __anext__(self) -> dict[str, JsonValue]:
        if self._index >= len(self._events):
            raise StopAsyncIteration
        event = self._events[self._index]
        self._index += 1
        return event


class Scripted:
    def __init__(self, answer: AnswerSpec) -> None:
        self._answer = answer
        self.params: dict[str, JsonValue] | None = None

    async def __call__(
        self, **params: JsonValue
    ) -> ScriptedStream | JsonValue | ResponsesAPIResponse:
        self.params = params
        if self._answer["shape"] == "stream":
            events = self._answer.get("events")
            return ScriptedStream(EMPTY_EVENTS if events is None else events)
        body = self._answer.get("body")
        value = ANSWER.validate_python(None if body is None else body)
        if self._answer["shape"] == "other":
            return value
        return RESPONSE.validate_python(value)


class HandlerFn(Protocol):
    def __call__(self, **kwargs: JsonValue) -> Coroutine[None, None, HandlerResult]: ...


class ExceptionTypeArguments(TypedDict):
    model: str
    original_exception: Exception
    custom_llm_provider: str


class ExceptionTypeFn(Protocol):
    def __call__(self, **kwargs: Unpack[ExceptionTypeArguments]) -> BaseException: ...


# the handler's async entry, whose staticmethod type no public stub spells out
HANDLER: Final[HandlerFn] = cast(HandlerFn, MessagesHandler.async_anthropic_messages_handler)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/handler.py#L169
EXCEPTION_TYPE: Final[ExceptionTypeFn] = cast(ExceptionTypeFn, exception_mapping_utils.exception_type)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/exception_mapping_utils.py#L2358


def mapped_error(error: Exception, model: str, provider: str) -> BaseException:
    try:
        return EXCEPTION_TYPE(
            model=model, original_exception=error, custom_llm_provider=provider
        )
    except Exception as mapped:  # noqa: BLE001 - exception_type raises the mapped error  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/exception_mapping_utils.py#L2358
        return mapped


def provider_label(call: Call) -> str:
    forwarded = call.get("forwarded")
    if forwarded is None:
        return ""
    provider = forwarded.get("custom_llm_provider")
    return "" if provider is None else provider


def handler_kwargs(call: Call) -> dict[str, JsonValue]:
    request = call["request"]
    stream = call.get("stream")
    kwargs: dict[str, JsonValue] = {
        "max_tokens": request["max_tokens"],
        "messages": request["messages"],
        "model": request["model"],
        "stream": False if stream is None else stream,
        "system": request.get("system"),
        "temperature": request.get("temperature"),
        "top_p": request.get("top_p"),
        "thinking": request.get("thinking"),
        "tool_choice": request.get("tool_choice"),
        "tools": request.get("tools"),
        "output_config": request.get("output_config"),
        "output_format": request.get("output_format"),
        "context_management": request.get("context_management"),
        "metadata": request.get("metadata"),
    }
    forwarded = call.get("forwarded")
    if forwarded is not None:
        provider = forwarded.get("custom_llm_provider")
        if provider is not None:
            kwargs["custom_llm_provider"] = provider
        base = forwarded.get("api_base")
        if base is not None:
            kwargs["api_base"] = base
        include = forwarded.get("include")
        if include is not None:
            kwargs["include"] = include
        cache_key = forwarded.get("prompt_cache_key")
        if cache_key is not None:
            kwargs["prompt_cache_key"] = cache_key
    return kwargs


def frame_value(raw: bytes) -> Frame:
    text = raw.decode()
    if not text.endswith("\n\n"):
        raise ValueError(f"not an SSE frame: {raw!r}")
    event_line, data_line = text.removesuffix("\n\n").split("\n")
    event = event_line.removeprefix("event: ")
    data = data_line.removeprefix("data: ")
    if event == event_line or data == data_line:
        raise ValueError(f"not an SSE frame: {raw!r}")
    return {"event": event, "data": ANSWER.validate_json(data)}


async def drive(call: Call, scripted: Scripted) -> Output:
    litellm.aresponses = scripted  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L576
    try:
        raw = await HANDLER(**handler_kwargs(call))
    except Exception as error:  # mapped through exception_type  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/exception_mapping_utils.py#L2358
        raise mapped_error(error, call["request"]["model"], provider_label(call)) from error
    params = scripted.params
    if params is None:
        raise RuntimeError("the handler never reached litellm.aresponses")
    answer: Output = {"request": ANSWER.validate_python(params)}
    stream = call.get("stream")
    if stream is None or not stream:
        answer["native"] = ANSWER.validate_python(raw)
        return answer
    answer["frames"] = [
        frame_value(chunk)
        async for chunk in cast(AsyncIterator[bytes], raw)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L486
    ]
    return answer


def run(input_value: Input) -> Output:
    call = input_value["call"]
    scripted = Scripted(input_value["answer"])
    return asyncio.run(drive(call, scripted))
