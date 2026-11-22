import asyncio
import json
import logging
from datetime import UTC, datetime, tzinfo
from typing import ClassVar, Final, Literal, NotRequired, Self, TypedDict
from uuid import UUID

from litellm.caching.redis_cache import RedisCache
from litellm.proxy.response_polling import polling_handler
from litellm.proxy.response_polling.polling_handler import ResponsePollingHandler
from litellm.types.llms.openai import ResponsesAPIStatus
from opentelemetry.trace import Span
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

LOGGER_NAME: Final = "LiteLLM Proxy"
DEFAULT_NOW: Final = 1_700_000_000
DEFAULT_TTL: Final = 3600
DEFAULT_UUID: Final = "00000000-0000-4000-8000-000000000001"
JSON: Final = TypeAdapter(JsonValue)


class Seed(TypedDict):
    raw: NotRequired[str]
    response: NotRequired[JsonValue]


class Patch(TypedDict):
    status: NotRequired[ResponsesAPIStatus | None]
    output: NotRequired[list[JsonValue] | None]
    usage: NotRequired[dict[str, JsonValue] | None]
    error: NotRequired[dict[str, JsonValue] | None]
    incomplete_details: NotRequired[dict[str, JsonValue] | None]
    reasoning: NotRequired[dict[str, JsonValue] | None]
    tool_choice: NotRequired[JsonValue]
    tools: NotRequired[list[JsonValue] | None]
    model: NotRequired[str | None]
    instructions: NotRequired[str | None]
    temperature: NotRequired[float | None]
    top_p: NotRequired[float | None]
    max_output_tokens: NotRequired[int | None]
    previous_response_id: NotRequired[str | None]
    text: NotRequired[dict[str, JsonValue] | None]
    truncation: NotRequired[str | None]
    parallel_tool_calls: NotRequired[bool | None]
    user: NotRequired[str | None]
    store: NotRequired[bool | None]


EMPTY_PATCH: Final[Patch] = {}


class Call(TypedDict):
    op: Literal["generate", "create", "update", "get", "cancel", "delete"]
    polling_id: NotRequired[str]
    redis: NotRequired[bool]
    ttl: NotRequired[int]
    metadata: NotRequired[dict[str, JsonValue]]
    patch: NotRequired[Patch]
    seed: NotRequired[Seed]


class Deployment(TypedDict):
    custom_llm_provider: NotRequired[str]
    model: NotRequired[str]


class Router(TypedDict):
    indices: list[int]
    deployments: list[Deployment]


class Selection(TypedDict):
    background: bool
    selector: JsonValue
    redis: bool
    model: str
    native_background_mode: NotRequired[list[str]]
    router: NotRequired[Router]


class Input(TypedDict):
    now: NotRequired[int]
    ttl: NotRequired[int]
    uuid: NotRequired[str]
    calls: NotRequired[list[Call]]
    selections: NotRequired[list[Selection]]


class FrozenDatetime(datetime):
    instant: ClassVar[datetime] = datetime.fromtimestamp(0, UTC)

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        if tz is None:
            return cls.fromtimestamp(cls.instant.timestamp())
        return cls.fromtimestamp(cls.instant.timestamp(), tz)


class FixedUuid:
    def __init__(self, value: UUID) -> None:
        self.value = value

    def __call__(self) -> UUID:
        return self.value


# RedisCache.__init__ builds a real client, so skip it and answer every cache call from the script
class ScriptedCache(RedisCache):
    def __init__(self) -> None:
        object.__init__(self)
        self.entries: dict[str, str] = {}
        self.effects: list[JsonValue] = []

    def seed(self, key: str, value: str) -> None:
        self.entries[key] = value

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
    async def async_delete_cache(self, key: str) -> None:
        effect: JsonValue = {"op": "delete", "key": key, "ttl": None, "value": None}
        self.effects.append(effect)
        self.entries.pop(key, None)


class Watch(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.lines: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelname not in ("WARNING", "DEBUG"):
            return
        self.lines.append({"level": record.levelname, "message": record.getMessage()})


class ScriptedRouter:
    def __init__(self, model: str, router: Router) -> None:
        self.model_name_to_deployment_indices: dict[str, list[int]] = {model: router["indices"]}
        self.model_list: list[dict[str, JsonValue]] = [
            {"litellm_params": deployment_params(deployment)} for deployment in router["deployments"]
        ]


def deployment_params(deployment: Deployment) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {}
    provider = deployment.get("custom_llm_provider")
    if provider is not None:
        params["custom_llm_provider"] = provider
    if "model" in deployment:
        params["model"] = deployment["model"]
    return params


def failure_label(error: BaseException) -> str:
    if isinstance(error, json.JSONDecodeError):
        return "decode"
    return type(error).__name__


def apply_seed(call: Call, cache: ScriptedCache) -> None:
    seed = call.get("seed")
    if seed is None:
        return
    key = polling_handler.ResponsePollingHandler.get_cache_key(call.get("polling_id", ""))
    raw = seed.get("raw")
    if raw is not None:
        cache.seed(key, raw)
        return
    if "response" in seed:
        cache.seed(key, json.dumps(seed["response"]))


def build_handler(
    call: Call, cache: ScriptedCache, default_ttl: int
) -> ResponsePollingHandler:
    redis: ScriptedCache | None = cache if call.get("redis", True) else None
    return ResponsePollingHandler(redis_cache=redis, ttl=call.get("ttl", default_ttl))


async def apply_update(handler: ResponsePollingHandler, polling_id: str, patch: Patch) -> None:
    await handler.update_state(
        polling_id=polling_id,
        status=patch.get("status"),
        usage=patch.get("usage"),
        error=patch.get("error"),
        incomplete_details=patch.get("incomplete_details"),
        reasoning=patch.get("reasoning"),
        tool_choice=patch.get("tool_choice"),
        tools=patch.get("tools"),
        output=patch.get("output"),
        model=patch.get("model"),
        instructions=patch.get("instructions"),
        temperature=patch.get("temperature"),
        top_p=patch.get("top_p"),
        max_output_tokens=patch.get("max_output_tokens"),
        previous_response_id=patch.get("previous_response_id"),
        text=patch.get("text"),
        truncation=patch.get("truncation"),
        parallel_tool_calls=patch.get("parallel_tool_calls"),
        user=patch.get("user"),
        store=patch.get("store"),
    )


async def execute(call: Call, handler: ResponsePollingHandler) -> JsonValue:
    op = call["op"]
    polling_id = call.get("polling_id", "")
    if op == "generate":
        return ResponsePollingHandler.generate_polling_id()
    if op == "create":
        request_data: dict[str, JsonValue] = {}
        metadata = call.get("metadata")
        if metadata is not None:
            request_data["metadata"] = metadata
        response = await handler.create_initial_state(polling_id, request_data)
        return JSON.validate_json(response.model_dump_json())
    if op == "update":
        patch = call.get("patch")
        await apply_update(handler, polling_id, EMPTY_PATCH if patch is None else patch)
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
    apply_seed(call, cache)
    effects_start = len(cache.effects)
    logs_start = len(watch.lines)
    handler = build_handler(call, cache, default_ttl)
    returned: JsonValue = None
    error: JsonValue = None
    try:
        returned = await execute(call, handler)
    except (ValueError, KeyError, TypeError) as failure:
        error = failure_label(failure)
    result: JsonValue = {
        "returned": returned,
        "error": error,
        "effects": cache.effects[effects_start:],
        "logs": watch.lines[logs_start:],
    }
    return result


def selection_result(selection: Selection, cache: ScriptedCache, watch: Watch) -> JsonValue:
    start = len(watch.lines)
    router = selection.get("router")
    llm_router: ScriptedRouter | None = None
    if router is not None:
        llm_router = ScriptedRouter(selection["model"], router)
    enabled = polling_handler.should_use_polling_for_request(
        background_mode=selection["background"],
        polling_via_cache_enabled=selection["selector"],
        redis_cache=cache if selection["redis"] else None,
        model=selection["model"],
        llm_router=llm_router,
        native_background_mode=selection.get("native_background_mode", []),
    )
    result: JsonValue = {"enabled": enabled, "logs": watch.lines[start:]}
    return result


async def run_calls(input_value: Input, cache: ScriptedCache, watch: Watch) -> JsonValue:
    default_ttl = input_value.get("ttl", DEFAULT_TTL)
    results: list[JsonValue] = []
    for call in input_value.get("calls", []):
        results.append(await state_result(call, cache, watch, default_ttl))
    return results


def run_selections(input_value: Input, cache: ScriptedCache, watch: Watch) -> JsonValue:
    results: list[JsonValue] = []
    for selection in input_value.get("selections", []):
        results.append(selection_result(selection, cache, watch))
    return results


def run(input_value: Input) -> JsonValue:
    now = input_value.get("now", DEFAULT_NOW)
    FrozenDatetime.instant = datetime.fromtimestamp(now, UTC)
    object.__setattr__(polling_handler, "datetime", FrozenDatetime)
    fixed = FixedUuid(UUID(input_value.get("uuid", DEFAULT_UUID)))
    object.__setattr__(polling_handler, "uuid4", fixed)
    watch = Watch()
    logger = logging.getLogger(LOGGER_NAME)
    previous_handlers = list(logger.handlers)
    previous_level = logger.level
    for handler in previous_handlers:
        logger.removeHandler(handler)
    logger.addHandler(watch)
    logger.setLevel(logging.DEBUG)
    cache = ScriptedCache()
    try:
        calls = asyncio.run(run_calls(input_value, cache, watch))
        selections = run_selections(input_value, cache, watch)
        result: JsonValue = {"calls": calls, "selections": selections}
    finally:
        logger.removeHandler(watch)
        for handler in previous_handlers:
            logger.addHandler(handler)
        logger.setLevel(previous_level)
    return result
