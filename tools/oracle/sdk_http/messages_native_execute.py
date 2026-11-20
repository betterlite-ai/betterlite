import asyncio
import json
from collections.abc import Callable, Mapping
from typing import Final, Literal, NotRequired, Protocol, TypedDict, cast

import litellm
import openai
from litellm.integrations.anthropic_cache_control_hook import (
    AnthropicCacheControlHook,
)
from litellm.integrations.custom_logger import CustomLogger
from litellm.integrations.websearch_interception.handler import (
    WebSearchInterceptionLogger,
)
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.llms.anthropic.pass_through.messages import handler as entry_handler
from litellm.llms.anthropic.pass_through.messages.fake_stream_iterator import (
    FakeAnthropicMessagesStreamIterator,
)
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

API_BASE: Final[str] = "<api_base>"
AMBIENT: Final[frozenset[str]] = frozenset({"litellm_call_id", "litellm_logging_obj"})
NAMED: Final[frozenset[str]] = frozenset(
    {
        "_litellm_messages_presanitized",
        "api_base",
        "api_key",
        "client",
        "custom_llm_provider",
        "max_tokens",
        "messages",
        "metadata",
        "model",
        "stop_sequences",
        "stream",
        "system",
        "temperature",
        "thinking",
        "tool_choice",
        "tools",
        "top_k",
        "top_p",
    }
)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
MESSAGES: Final[TypeAdapter[list[JsonValue]]] = TypeAdapter(list[JsonValue])
RESULTS: Final[TypeAdapter[dict[str, object]]] = TypeAdapter(dict[str, object])


# the entry's named arguments as the case scripts them; the harness owns api_base
class EntryRequest(TypedDict):
    max_tokens: int
    messages: list[dict[str, JsonValue]]
    model: str
    metadata: dict[str, JsonValue] | None
    stop_sequences: list[str] | None
    stream: bool | None
    system: str | list[JsonValue] | None
    temperature: float | None
    thinking: dict[str, JsonValue] | None
    tool_choice: dict[str, JsonValue] | None
    tools: list[dict[str, JsonValue]] | None
    top_k: int | None
    top_p: float | None
    api_key: str | None
    custom_llm_provider: str | None
    kwargs: dict[str, JsonValue]


# One CustomLogger the case installs, and what async_pre_request_hook does to the hook map
class HookScript(TypedDict):
    effect: Literal["pass", "mutate", "replace"]
    values: NotRequired[dict[str, JsonValue]]


# One WebSearchInterceptionLogger; a null answer lets the request continue
class SearchScript(TypedDict):
    answer: NotRequired[JsonValue]


# One MessagesInterceptor; a null answer is the entry's None result
class InterceptorScript(TypedDict):
    can_handle: bool
    answer: JsonValue


class NativeExecuteCall(TypedDict):
    request: EntryRequest
    provider: str | None
    hooks: list[HookScript]
    searches: list[SearchScript]
    interceptors: list[InterceptorScript]


# What the cache hook, the pre-hooks and the prepared dispatch observed; api_base is masked
class OutcomeValue(TypedDict):
    cache: dict[str, JsonValue]
    hooks: list[dict[str, JsonValue]]
    dispatch: dict[str, JsonValue] | None
    response: JsonValue
    stream: list[str]
    failure: dict[str, JsonValue] | None


class ChunkStream(Protocol):
    async def __anext__(self) -> bytes: ...


class ProviderResolutionError(LookupError):
    pass


class Recordings:
    def __init__(self) -> None:
        self.cache: dict[str, JsonValue] | None = None
        self.hooks: list[dict[str, JsonValue]] = []
        self.dispatch: dict[str, JsonValue] | None = None


# The decorator adds its own keys to every call; they are not the case's extras
def recorded(kwargs: Mapping[str, JsonValue]) -> dict[str, JsonValue]:
    record: dict[str, JsonValue] = {}
    for name, value in kwargs.items():
        if name in AMBIENT:
            continue
        record[name] = value
    return record


def dispatch_record(call: dict[str, JsonValue]) -> dict[str, JsonValue]:
    record: dict[str, JsonValue] = {
        "api_base": API_BASE if call.get("api_base") is not None else None,
        "client": call.get("client"),
        "_litellm_messages_presanitized": call["_litellm_messages_presanitized"],
    }
    for name, value in call.items():
        if name in NAMED or name in AMBIENT:
            continue
        record[name] = value
    return record


def failure_record(error: openai.APIStatusError) -> dict[str, JsonValue]:
    return {
        "class": type(error).__name__,
        "status": error.status_code,
        "type": error.type,
        "param": error.param,
        "code": error.code,
    }


def empty_outcome() -> OutcomeValue:
    return {
        "cache": {},
        "hooks": [],
        "dispatch": None,
        "response": None,
        "stream": [],
        "failure": None,
    }


async def collect_stream(stream: ChunkStream) -> list[str]:
    events: list[str] = []
    while True:
        try:
            chunk = await stream.__anext__()
        except StopAsyncIteration:
            return events
        line = chunk.decode().split("\n", maxsplit=1)[0]
        if line.startswith("event: "):
            events.append(line.removeprefix("event: "))


class CacheRecorder:
    def __init__(self, recordings: Recordings) -> None:
        self.recordings = recordings

    def __call__(
        self,
        messages: list[JsonValue],
        system: JsonValue,
        kwargs: dict[str, JsonValue],
        **call: JsonValue,
    ) -> tuple[list[JsonValue], JsonValue]:
        recorded_messages = MESSAGES.validate_python(messages)
        recorded_system = JSON.validate_python(system)
        if self.recordings.cache is None:
            self.recordings.cache = {
                "messages": recorded_messages,
                "system": recorded_system,
                "model": call.get("model"),
                "custom_llm_provider": call.get("custom_llm_provider"),
                "tools": call.get("tools"),
                "api_base": API_BASE if call.get("api_base") is not None else None,
                "kwargs": recorded(kwargs),
            }
        return recorded_messages, recorded_system


class ScriptedHook:
    def __init__(self, script: HookScript, recordings: Recordings) -> None:
        self.script = script
        self.recordings = recordings

    async def pre_request(
        self,
        model: str,
        messages: list[JsonValue],
        kwargs: dict[str, JsonValue],
    ) -> dict[str, JsonValue] | None:
        self.recordings.hooks.append(
            {
                "model": model,
                "messages": MESSAGES.validate_python(messages),
                "kwargs": recorded(kwargs),
            }
        )
        effect = self.script["effect"]
        if effect == "pass":
            return None
        values = self.script["values"]
        if effect == "replace":
            return dict(values)
        for name, value in values.items():
            kwargs[name] = value
        return None


class ScriptedSearch:
    def __init__(self, script: SearchScript) -> None:
        self.script = script

    async def pre_request(
        self, _model: str, _messages: list[JsonValue], _kwargs: dict[str, JsonValue]
    ) -> None:
        pass

    async def short_circuit(self, **_call: JsonValue) -> dict[str, object] | None:
        answer = self.script.get("answer")
        if answer is None:
            return None
        return RESULTS.validate_python(answer)


class ScriptedInterceptor:
    def __init__(self, script: InterceptorScript) -> None:
        self.script = script

    def can_handle(self, _tools: list[JsonValue] | None, _provider: str | None) -> bool:
        return self.script["can_handle"]

    async def handle(self, **_call: JsonValue) -> JsonValue:
        return self.script["answer"]


class InterceptorRegistry:
    def __init__(self, scripts: list[InterceptorScript]) -> None:
        self.scripts = scripts

    def __call__(self) -> list[ScriptedInterceptor]:
        return [ScriptedInterceptor(script) for script in self.scripts]


class ScriptedResolver:
    def __init__(self, provider: str | None) -> None:
        self.provider = provider

    def __call__(
        self, model: str, **_call: JsonValue
    ) -> tuple[str, str, str | None, str | None]:
        provider = self.provider
        if provider is None:
            raise ProviderResolutionError
        return model, provider, None, None


def callback_list(call: NativeExecuteCall, recordings: Recordings) -> list[CustomLogger]:
    callbacks: list[CustomLogger] = []
    for script in call["hooks"]:
        hook = CustomLogger()
        object.__setattr__(
            hook, "async_pre_request_hook", ScriptedHook(script, recordings).pre_request
        )
        callbacks.append(hook)
    for script in call["searches"]:
        search = WebSearchInterceptionLogger()
        scripted = ScriptedSearch(script)
        object.__setattr__(search, "async_pre_request_hook", scripted.pre_request)
        object.__setattr__(search, "try_short_circuit_search", scripted.short_circuit)
        callbacks.append(search)
    return callbacks


class Dispatcher:
    def __init__(
        self,
        recordings: Recordings,
        handler: Callable[..., object],
        provider: Callable[..., object],
    ) -> None:
        self.recordings = recordings
        self.handler = handler
        self.provider = provider

    def __call__(self, **call: JsonValue) -> object:
        self.recordings.dispatch = dispatch_record(call)
        object.__setattr__(litellm, "get_llm_provider", self.provider)
        return self.handler(**call)


def install(call: NativeExecuteCall, recordings: Recordings) -> None:
    object.__setattr__(litellm, "callbacks", callback_list(call, recordings))
    type.__setattr__(
        AnthropicCacheControlHook,
        "maybe_inject_cache_control",
        staticmethod(CacheRecorder(recordings)),
    )
    object.__setattr__(litellm, "get_llm_provider", ScriptedResolver(call["provider"]))
    object.__setattr__(
        entry_handler, "get_messages_interceptors", InterceptorRegistry(call["interceptors"])
    )
    object.__setattr__(
        entry_handler,
        "anthropic_messages_handler",
        Dispatcher(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1674-L1681
            recordings, entry_handler.anthropic_messages_handler, get_llm_provider
        ),
    )


async def execute(call: NativeExecuteCall, api_base: str) -> OutcomeValue:
    recordings = Recordings()
    request = call["request"]
    install(call, recordings)
    outcome = empty_outcome()
    try:
        answer = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/handler.py#L254-L256
            "dict[str, JsonValue] | FakeAnthropicMessagesStreamIterator | None",
            await entry_handler.anthropic_messages(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1674-L1681

                max_tokens=request["max_tokens"],
                messages=request["messages"],
                model=request["model"],
                metadata=request["metadata"],
                stop_sequences=request["stop_sequences"],
                stream=request["stream"] is True,
                system=request["system"],
                temperature=request["temperature"],
                thinking=request["thinking"],
                tool_choice=request["tool_choice"],
                tools=request["tools"],
                top_k=request["top_k"],
                top_p=request["top_p"],
                api_key=request["api_key"],
                api_base=api_base.removesuffix("/v1"),
                client=None,
                custom_llm_provider=request["custom_llm_provider"],
                **request["kwargs"],
            ),
        )  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/handler.py#L254-L256
    except openai.APIStatusError as error:
        outcome["failure"] = failure_record(error)
        return finish(outcome, recordings)
    if isinstance(answer, FakeAnthropicMessagesStreamIterator):
        stream = cast("ChunkStream", answer)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/fake_stream_iterator.py#L194-L203
        outcome["stream"] = await collect_stream(stream)
    elif answer is not None:
        outcome["response"] = JSON.validate_python(answer)
    return finish(outcome, recordings)


def finish(outcome: OutcomeValue, recordings: Recordings) -> OutcomeValue:
    cache = recordings.cache
    outcome["cache"] = {} if cache is None else cache
    outcome["hooks"] = recordings.hooks
    outcome["dispatch"] = recordings.dispatch
    return outcome


def run(call: NativeExecuteCall, api_base: str) -> str:
    outcome = asyncio.run(execute(call, api_base))
    return json.dumps({"messages_native_execute": {"value": outcome}})


class MessagesNativeExecuteRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_native_execute: NativeExecuteCall

    def serialize(self, api_base: str) -> str:
        return run(self.messages_native_execute, api_base)
