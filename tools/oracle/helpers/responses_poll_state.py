import asyncio
import json
import logging
from datetime import UTC, datetime, tzinfo
from functools import partial
from typing import ClassVar, Literal, Self, TypedDict
from uuid import UUID

from litellm.caching.redis_cache import RedisCache
from litellm.proxy.response_polling import polling_handler
from litellm.proxy.response_polling.polling_handler import (
    ResponsePollingHandler,
    should_use_polling_for_request,
)
from litellm.types.llms.openai import ResponsesAPIStatus
from opentelemetry.trace import Span
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Seed(TypedDict):
    raw: str | None
    response: JsonValue


class Patch(TypedDict, total=False):
    status: ResponsesAPIStatus | None
    output: list[JsonValue] | None
    usage: dict[str, JsonValue] | None
    error: dict[str, JsonValue] | None
    incomplete_details: dict[str, JsonValue] | None
    reasoning: dict[str, JsonValue] | None
    tool_choice: JsonValue
    tools: list[JsonValue] | None
    model: str | None
    instructions: str | None
    temperature: float | None
    top_p: float | None
    max_output_tokens: int | None
    previous_response_id: str | None
    text: dict[str, JsonValue] | None
    truncation: Literal["auto", "disabled"] | None
    parallel_tool_calls: bool | None
    user: str | None
    store: bool | None


class Call(TypedDict):
    op: Literal["generate", "create", "update", "get", "cancel", "delete"]
    polling_id: str
    redis: bool
    ttl: int | None
    metadata: dict[str, JsonValue] | None
    patch: Patch
    seed: Seed | None


class Router(TypedDict):
    indices: list[int]
    deployments: list[dict[str, str | None]]


class Selection(TypedDict):
    background: bool
    selector: JsonValue
    redis: bool
    model: str
    native_background_mode: list[str]
    router: Router | None


class Input(TypedDict):
    now: int
    ttl: int
    uuid: str
    calls: list[Call]
    selections: list[Selection]


class FrozenDatetime(datetime):
    instant: ClassVar[datetime] = datetime.fromtimestamp(0, UTC)

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        stamp = cls.instant.timestamp()
        return cls.fromtimestamp(stamp) if tz is None else cls.fromtimestamp(stamp, tz)


# RedisCache.__init__ builds a real client, so skip it and answer every cache call from the script
class ScriptedCache(RedisCache):
    def __init__(self) -> None:
        object.__init__(self)
        self.entries: dict[str, str] = {}
        self.effects: list[JsonValue] = []

    @override
    async def async_set_cache(self, key: str, value: str, **kwargs: int) -> None:
        effect: JsonValue = {
            "op": "write",
            "key": key,
            "ttl": kwargs.get("ttl"),
            "value": JSON.validate_json(value),
        }
        self.effects.append(effect)
        self.entries[key] = value

    @override
    async def async_get_cache(
        self, key: str, parent_otel_span: Span | None = None, **kwargs: int
    ) -> str | None:
        del parent_otel_span, kwargs
        effect: JsonValue = {"op": "read", "key": key, "ttl": None, "value": None}
        self.effects.append(effect)
        return self.entries.get(key)

    @override
    async def async_delete_cache(self, key: str) -> int:
        effect: JsonValue = {"op": "delete", "key": key, "ttl": None, "value": None}
        self.effects.append(effect)
        removed = self.entries.pop(key, None)
        return int(removed is not None)


class Watch(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.lines: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelname in ("WARNING", "DEBUG"):
            self.lines.append({"level": record.levelname, "message": record.getMessage()})


class ScriptedRouter:
    def __init__(self, model: str, router: Router) -> None:
        self.model_name_to_deployment_indices: dict[str, list[int]] = {model: router["indices"]}
        self.model_list: list[dict[str, JsonValue]] = [
            {"litellm_params": deployment_params(deployment)}
            for deployment in router["deployments"]
        ]


def deployment_params(deployment: dict[str, str | None]) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {}
    provider = deployment["custom_llm_provider"]
    if provider is not None:
        params["custom_llm_provider"] = provider
    model = deployment["model"]
    if model is not None:
        params["model"] = model
    return params


async def execute(call: Call, handler: ResponsePollingHandler) -> JsonValue:
    op = call["op"]
    polling_id = call["polling_id"]
    if op == "generate":
        return ResponsePollingHandler.generate_polling_id()
    if op == "create":
        request_data: dict[str, JsonValue] = {}
        metadata = call["metadata"]
        if metadata is not None:
            request_data["metadata"] = metadata
        response = await handler.create_initial_state(polling_id, request_data)
        return JSON.validate_json(response.model_dump_json())
    if op == "update":
        await handler.update_state(polling_id=polling_id, **call["patch"])
        return None
    if op == "get":
        return JSON.validate_python(await handler.get_state(polling_id))
    if op == "cancel":
        return await handler.cancel_polling(polling_id)
    if op == "delete":
        return await handler.delete_polling(polling_id)
    raise ValueError(f"unknown op {op}")


async def state_result(
    call: Call, cache: ScriptedCache, watch: Watch, default_ttl: int
) -> JsonValue:
    seed = call["seed"]
    if seed is not None:
        key = ResponsePollingHandler.get_cache_key(call["polling_id"])
        raw = seed["raw"]
        cache.entries[key] = raw if raw is not None else json.dumps(seed["response"])
    effects_start = len(cache.effects)
    logs_start = len(watch.lines)
    redis: ScriptedCache | None = cache if call["redis"] else None
    handler = ResponsePollingHandler(
        redis_cache=redis, ttl=call["ttl"] if call["ttl"] is not None else default_ttl
    )
    returned: JsonValue = None
    error: JsonValue = None
    try:
        returned = await execute(call, handler)
    except json.JSONDecodeError:
        error = "decode"
    except (KeyError, TypeError) as failure:
        error = type(failure).__name__
    return {
        "returned": returned,
        "error": error,
        "effects": cache.effects[effects_start:],
        "logs": watch.lines[logs_start:],
    }


def selection_result(selection: Selection, cache: ScriptedCache, watch: Watch) -> JsonValue:
    start = len(watch.lines)
    llm_router: ScriptedRouter | None = None
    router = selection["router"]
    if router is not None:
        llm_router = ScriptedRouter(selection["model"], router)
    enabled = should_use_polling_for_request(
        background_mode=selection["background"],
        polling_via_cache_enabled=selection["selector"],
        redis_cache=cache if selection["redis"] else None,
        model=selection["model"],
        llm_router=llm_router,
        native_background_mode=selection["native_background_mode"],
    )
    return {"enabled": enabled, "logs": watch.lines[start:]}


def run(input_value: Input) -> JsonValue:
    FrozenDatetime.instant = datetime.fromtimestamp(input_value["now"], UTC)
    object.__setattr__(polling_handler, "datetime", FrozenDatetime)
    object.__setattr__(polling_handler, "uuid4", partial(UUID, input_value["uuid"]))
    watch = Watch()
    logger = logging.getLogger("LiteLLM Proxy")
    previous_handlers = logger.handlers[:]
    previous_level = logger.level
    logger.handlers = [watch]
    logger.setLevel(logging.DEBUG)
    cache = ScriptedCache()
    try:
        calls = [
            asyncio.run(state_result(call, cache, watch, input_value["ttl"]))
            for call in input_value["calls"]
        ]
        selections = [
            selection_result(selection, cache, watch) for selection in input_value["selections"]
        ]
        result: JsonValue = {"calls": calls, "selections": selections}
    finally:
        logger.handlers = previous_handlers
        logger.setLevel(previous_level)
    return result
