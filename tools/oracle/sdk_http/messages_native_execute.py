import asyncio
import json
from abc import ABC, abstractmethod
from collections.abc import AsyncIterator, Callable
from typing import Final, Literal, NotRequired, TypedDict, cast

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
from litellm.llms.anthropic.pass_through.messages import fake_stream_iterator
from litellm.llms.anthropic.pass_through.messages import handler as entry_handler
from pydantic import BaseModel, ConfigDict
from pydantic import JsonValue as Json
from typing_extensions import override

API_BASE: Final[str] = "<api_base>"
AMBIENT: Final[frozenset[str]] = frozenset({"litellm_call_id", "litellm_logging_obj"})
NAMED: Final[frozenset[str]] = frozenset(
    {
        "_litellm_messages_presanitized", "api_base", "api_key", "client", "custom_llm_provider",
        "max_tokens", "messages", "metadata", "model", "stop_sequences", "stream", "system",
        "temperature", "thinking", "tool_choice", "tools", "top_k", "top_p",
    }
)


class EntryRequest(TypedDict):
    max_tokens: int
    messages: list[dict[str, Json]]
    model: str
    metadata: dict[str, Json] | None
    stop_sequences: list[str] | None
    stream: bool | None
    system: str | list[Json] | None
    temperature: float | None
    thinking: dict[str, Json] | None
    tool_choice: dict[str, Json] | None
    tools: list[dict[str, Json]] | None
    top_k: int | None
    top_p: float | None
    api_key: str | None
    custom_llm_provider: str | None
    kwargs: dict[str, Json]


class HookScript(TypedDict):
    effect: Literal["pass", "mutate", "replace"]
    values: NotRequired[dict[str, Json]]


class SearchScript(TypedDict):
    answer: NotRequired[dict[str, Json] | None]


class InterceptorScript(TypedDict):
    can_handle: bool
    answer: Json


class NativeExecuteCall(TypedDict):
    request: EntryRequest
    provider: str | None
    hooks: list[HookScript]
    searches: list[SearchScript]
    interceptors: list[InterceptorScript]


class Recordings:
    def __init__(self) -> None:
        self.cache: dict[str, Json] = {}
        self.hooks: list[Json] = []
        self.dispatch: dict[str, Json] | None = None


class Dispatcher:
    def __init__(  # type: ignore[explicit-any]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/handler.py#L254-L256
        self,
        recordings: Recordings,
        handler: Callable[..., Json],
    ) -> None:
        self.recordings = recordings
        self.handler = handler  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/handler.py#L254-L256

    def __call__(self, **call: Json) -> Json:
        record: dict[str, Json] = {
            "api_base": API_BASE if call.get("api_base") is not None else None,
            "client": call.get("client"),
            "_litellm_messages_presanitized": call["_litellm_messages_presanitized"],
        }
        for name, value in call.items():
            if name in NAMED or name in AMBIENT:
                continue
            record[name] = value
        self.recordings.dispatch = record
        object.__setattr__(litellm, "get_llm_provider", get_llm_provider)
        return self.handler(**call)


class CacheRecorder:
    def __init__(self, recordings: Recordings) -> None:
        self.recordings = recordings

    def __call__(
        self, messages: list[Json], system: Json, kwargs: dict[str, Json], **call: Json
    ) -> tuple[list[Json], Json]:
        if not self.recordings.cache:
            self.recordings.cache = {
                "messages": messages, "system": system, "model": call.get("model"),
                "custom_llm_provider": call.get("custom_llm_provider"),
                "tools": call.get("tools"),
                "api_base": API_BASE if call.get("api_base") is not None else None,
                "kwargs": {name: value for name, value in kwargs.items() if name not in AMBIENT},
            }
        return messages, system


class ScriptedHook:
    def __init__(self, script: HookScript, recordings: Recordings) -> None:
        self.script = script
        self.recordings = recordings

    async def pre_request(
        self, model: str, messages: list[Json], kwargs: dict[str, Json]
    ) -> dict[str, Json] | None:
        self.recordings.hooks.append(
            {
                "model": model, "messages": messages,
                "kwargs": {name: value for name, value in kwargs.items() if name not in AMBIENT},
            }
        )
        effect = self.script["effect"]
        if effect == "pass":
            return None
        values = self.script.get("values")
        if values is None:
            values = {}
        if effect == "replace":
            return dict(values)
        kwargs.update(values)
        return None


class SearchDouble:
    async def pre_request(
        self, _model: str, _messages: list[Json], _kwargs: dict[str, Json]
    ) -> None: ...

    async def short_circuit(self, **_call: Json) -> dict[str, Json] | None: ...


class ScriptedSearch(SearchDouble):
    def __init__(self, script: SearchScript) -> None:
        self.script = script

    @override
    async def pre_request(
        self, _model: str, _messages: list[Json], _kwargs: dict[str, Json]
    ) -> None:
        pass

    @override
    async def short_circuit(self, **_call: Json) -> dict[str, Json] | None:
        return self.script.get("answer")


class InterceptorDouble(ABC):
    @abstractmethod
    def can_handle(self, _tools: list[Json] | None, _provider: str | None) -> bool: ...

    @abstractmethod
    async def handle(self, **_call: Json) -> Json: ...


class ScriptedInterceptor(InterceptorDouble):
    def __init__(self, script: InterceptorScript) -> None:
        self.script = script

    @override
    def can_handle(self, _tools: list[Json] | None, _provider: str | None) -> bool:
        return self.script["can_handle"]

    @override
    async def handle(self, **_call: Json) -> Json:
        return self.script["answer"]


class InterceptorRegistry:
    def __init__(self, scripts: list[InterceptorScript]) -> None:
        self.scripts = scripts

    def __call__(self) -> list[ScriptedInterceptor]:
        return [ScriptedInterceptor(script) for script in self.scripts]


class ScriptedResolver:
    def __init__(self, provider: str | None) -> None:
        self.provider = provider

    def __call__(self, model: str, **_call: Json) -> tuple[str, str, str | None, str | None]:
        provider = self.provider
        if provider is None:
            raise LookupError
        return model, provider, None, None


def install(call: NativeExecuteCall, recordings: Recordings) -> None:
    callbacks: list[CustomLogger] = []
    for script in call["hooks"]:
        hook = CustomLogger()
        scripted_hook = ScriptedHook(script, recordings)
        object.__setattr__(hook, "async_pre_request_hook", scripted_hook.pre_request)
        callbacks.append(hook)
    for search_script in call["searches"]:
        search = WebSearchInterceptionLogger()
        scripted = ScriptedSearch(search_script)
        object.__setattr__(search, "async_pre_request_hook", scripted.pre_request)
        object.__setattr__(search, "try_short_circuit_search", scripted.short_circuit)
        callbacks.append(search)
    object.__setattr__(litellm, "callbacks", callbacks)
    cache_hook = staticmethod(CacheRecorder(recordings))
    type.__setattr__(AnthropicCacheControlHook, "maybe_inject_cache_control", cache_hook)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/anthropic_cache_control_hook.py
    object.__setattr__(litellm, "get_llm_provider", ScriptedResolver(call["provider"]))
    registry = InterceptorRegistry(call["interceptors"])
    object.__setattr__(entry_handler, "get_messages_interceptors", registry)
    dispatcher = Dispatcher(recordings, entry_handler.anthropic_messages_handler)  # type: ignore[misc, arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/handler.py#L254-L256
    object.__setattr__(entry_handler, "anthropic_messages_handler", dispatcher)


async def collect_stream(stream: AsyncIterator[bytes]) -> list[Json]:
    events: list[Json] = []
    async for chunk in stream:
        line = chunk.decode().split("\n", maxsplit=1)[0]
        if line.startswith("event: "):
            events.append(line.removeprefix("event: "))
    return events


async def execute(call: NativeExecuteCall, api_base: str) -> dict[str, Json]:
    recordings = Recordings()
    request = call["request"]
    install(call, recordings)
    outcome: dict[str, Json] = {"response": None, "stream": [], "failure": None}
    try:
        answer = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/handler.py#L254-L256
            "dict[str, Json] | fake_stream_iterator.FakeAnthropicMessagesStreamIterator | None",
            await entry_handler.anthropic_messages(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1674-L1681
                max_tokens=request["max_tokens"], messages=request["messages"],
                model=request["model"], metadata=request["metadata"],
                stop_sequences=request["stop_sequences"], stream=request["stream"] is True,
                system=request["system"], temperature=request["temperature"],
                thinking=request["thinking"], tool_choice=request["tool_choice"],
                tools=request["tools"], top_k=request["top_k"],
                top_p=request["top_p"], api_key=request["api_key"],
                api_base=api_base.removesuffix("/v1"), client=None,
                custom_llm_provider=request["custom_llm_provider"], **request["kwargs"],
            ),
        )
    except openai.APIStatusError as error:
        outcome["failure"] = {
            "class": type(error).__name__, "status": error.status_code,
            "type": error.type, "param": error.param, "code": error.code,
        }
    else:
        match answer:
            # ast-grep-ignore: no-long-names
            case fake_stream_iterator.FakeAnthropicMessagesStreamIterator():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/fake_stream_iterator.py#L194-L203
                stream = cast("AsyncIterator[bytes]", answer)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/fake_stream_iterator.py#L194-L203
                outcome["stream"] = await collect_stream(stream)
            case dict() | None:
                outcome["response"] = answer
    outcome["cache"] = recordings.cache
    outcome["hooks"] = recordings.hooks
    outcome["dispatch"] = recordings.dispatch
    return outcome


def run(call: NativeExecuteCall, api_base: str) -> str:
    outcome = asyncio.run(execute(call, api_base))
    payload: Json = {"messages_native_execute": {"value": outcome}}
    return json.dumps(payload)


class MessagesNativeExecuteRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_native_execute: NativeExecuteCall

    def serialize(self, api_base: str) -> str:
        return run(self.messages_native_execute, api_base)
