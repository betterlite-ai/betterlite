import asyncio
import time
from datetime import UTC, datetime
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm import DualCache
from litellm.exceptions import BudgetExceededError
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks import model_max_budget_limiter as limiter_module
from litellm.proxy.hooks.model_max_budget_limiter import (
    _PROXY_VirtualKeyModelMaxBudgetLimiter as ModelMaxBudgetLimiter,  # pyright: ignore[reportPrivateUsage]
)
from openai.types import Batch

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

SPEND_PREFIXES = (
    "virtual_key_spend:",
    "user_model_spend:",
    "end_user_model_spend:",
    "team_model_spend:",
)


def object_of(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return cast("Object", value)


def required_string(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_string(value: Json, name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def uptime_of(value: Json) -> float:
    if isinstance(value, bool) or not isinstance(value, int | float):
        raise TypeError("uptime_seconds must be a number")
    return float(value)


def budget_of(value: Json) -> Object | None:
    return None if value is None else object_of(value, "budget")


def mapping_of(value: Json) -> dict[str, object]:
    return cast("dict[str, object]", budget_of(value))


def fallbacks_of(value: Json) -> dict[str, list[str]] | None:
    if value is None:
        return None
    parsed: dict[str, list[str]] = {}
    for model, names in object_of(value, "fallbacks").items():
        if not isinstance(names, list):
            raise TypeError(f"fallbacks[{model}] must be a list")
        models: list[str] = []
        for name in cast("list[Json]", names):
            if not isinstance(name, str):
                raise TypeError(f"fallbacks[{model}] must hold strings")
            models.append(name)
        parsed[model] = models
    return parsed


def calls_of(value: Json) -> list[Object]:
    if not isinstance(value, list):
        raise TypeError("calls must be a list")
    return [object_of(call, "call") for call in cast("list[Json]", value)]


def json_number(value: object) -> Json:
    if isinstance(value, bool):
        return None
    if isinstance(value, int | float):
        return float(value)
    return None


def exceeded(error: BudgetExceededError) -> Json:
    return {
        "outcome": "exceeded",
        "current_cost": float(error.current_cost),
        "max_budget": float(error.max_budget),
        "entity_type": error.entity_type,
        "entity_id": error.entity_id,
    }


def key_auth(fields: Object) -> UserAPIKeyAuth:
    auth = UserAPIKeyAuth(
        token=required_string(fields, "entity_id"),
        key_alias=optional_string(fields.get("key_alias"), "key_alias"),
    )
    model_budget = budget_of(fields.get("budget"))
    if model_budget is not None:
        auth.model_max_budget = model_budget
    fallbacks = fallbacks_of(fields.get("fallbacks"))
    if fallbacks is not None:
        auth.budget_fallbacks = fallbacks
    return auth


async def key_check(limiter: ModelMaxBudgetLimiter, fields: Object) -> Json:
    try:
        await limiter.is_key_within_model_budget(  # pyright: ignore[reportUnknownMemberType]
            user_api_key_dict=key_auth(fields), model=required_string(fields, "model")
        )
    except BudgetExceededError as error:
        return exceeded(error)
    return {"outcome": "within"}


async def user_check(limiter: ModelMaxBudgetLimiter, fields: Object) -> Json:
    try:
        await limiter.is_user_within_model_budget(  # pyright: ignore[reportUnknownMemberType]
            user_id=required_string(fields, "entity_id"),
            user_model_max_budget=mapping_of(fields.get("budget")),
            model=required_string(fields, "model"),
        )
    except BudgetExceededError as error:
        return exceeded(error)
    return {"outcome": "within"}


async def end_user_check(limiter: ModelMaxBudgetLimiter, fields: Object) -> Json:
    try:
        await limiter.is_end_user_within_model_budget(  # pyright: ignore[reportUnknownMemberType]
            end_user_id=required_string(fields, "entity_id"),
            end_user_model_max_budget=mapping_of(fields.get("budget")),
            model=required_string(fields, "model"),
        )
    except BudgetExceededError as error:
        return exceeded(error)
    return {"outcome": "within"}


async def team_check(limiter: ModelMaxBudgetLimiter, fields: Object) -> Json:
    try:
        await limiter.is_team_within_model_budget(  # pyright: ignore[reportUnknownMemberType]
            team_id=required_string(fields, "team_id"),
            team_model_max_budget=mapping_of(fields.get("budget")),
            key_model_max_budget=budget_of(fields.get("key_budget")),
            model=required_string(fields, "model"),
        )
    except BudgetExceededError as error:
        return exceeded(error)
    return {"outcome": "within"}


async def fallback_check(limiter: ModelMaxBudgetLimiter, fields: Object) -> Json:
    chosen = await limiter.get_fallback_model_within_budget(  # pyright: ignore[reportUnknownMemberType]
        user_api_key_dict=key_auth(fields), model=required_string(fields, "model")
    )
    return {"outcome": "fallback", "model": chosen}


def batch_response(batch_id: str) -> Batch:
    return Batch(
        id=batch_id,
        completion_window="24h",
        created_at=0,
        endpoint="/v1/chat/completions",
        input_file_id="file-oracle",
        object="batch",
        status="completed",
    )


def charge_kwargs(fields: Object) -> dict[str, Json]:
    metadata: dict[str, Json] = {}
    for name, field in (
        ("user_api_key_model_max_budget", "key_budget"),
        ("user_api_key_team_model_max_budget", "team_budget"),
        ("user_api_key_user_model_max_budget", "user_budget"),
        ("user_api_key_end_user_model_max_budget", "end_user_budget"),
    ):
        value = fields.get(field)
        if value is not None:
            metadata[name] = value
    payload_metadata: dict[str, Json] = {}
    for name, field in (
        ("user_api_key_hash", "key_hash"),
        ("user_api_key_team_id", "team_id"),
        ("user_api_key_user_id", "user_id"),
        ("user_api_key_end_user_id", "end_user_id"),
    ):
        value = fields.get(field)
        if value is not None:
            payload_metadata[name] = value
    payload: dict[str, Json] = {
        "metadata": payload_metadata,
        "response_cost": fields.get("response_cost", 0),
    }
    for name in ("model_group", "model", "end_user"):
        value = fields.get(name)
        if value is not None:
            payload[name] = value
    kwargs: dict[str, Json] = {
        "standard_logging_object": payload,
        "litellm_params": {"metadata": metadata},
    }
    call_type = optional_string(fields.get("call_type"), "call_type")
    if call_type is not None:
        kwargs["call_type"] = call_type
    return kwargs


async def charge(limiter: ModelMaxBudgetLimiter, fields: Object) -> Json:
    batch_id = optional_string(fields.get("batch_id"), "batch_id")
    response_obj: object = {} if batch_id is None else batch_response(batch_id)
    now = datetime.now(UTC)
    try:
        await limiter.async_log_success_event(  # pyright: ignore[reportUnknownMemberType]
            kwargs=charge_kwargs(fields),
            response_obj=response_obj,
            start_time=now,
            end_time=now,
        )
    except Exception:  # noqa: BLE001 - any callback failure is one comparable outcome
        return {"outcome": "raised"}
    return {"outcome": "charged"}


async def run_call(limiter: ModelMaxBudgetLimiter, call: Object) -> Json:
    kind = required_string(call, "kind")
    match kind:
        case "key":
            return await key_check(limiter, call)
        case "user":
            return await user_check(limiter, call)
        case "end_user":
            return await end_user_check(limiter, call)
        case "team":
            return await team_check(limiter, call)
        case "fallback":
            return await fallback_check(limiter, call)
        case "charge":
            return await charge(limiter, call)
        case _:
            raise ValueError(f"unsupported call kind {kind}")


def cache_state(cache: DualCache) -> tuple[Json, Json]:
    contents = cast("dict[str, object]", cache.in_memory_cache.cache_dict)  # pyright: ignore[reportUnknownMemberType]
    spend: dict[str, Json] = {}
    markers: dict[str, Json] = {}
    for key, value in contents.items():
        if ":batch:" in key:
            markers[key] = json_number(value)
            continue
        if key.startswith(SPEND_PREFIXES):
            spend[key] = json_number(value)
    return spend, markers


async def execute(memory: Object, calls: list[Object]) -> Json:
    cache = DualCache()
    for key, value in memory.items():
        await cache.async_set_cache(key=key, value=value)  # pyright: ignore[reportUnknownMemberType]
    limiter = ModelMaxBudgetLimiter(dual_cache=cache)
    outcomes = [await run_call(limiter, call) for call in calls]
    spend, markers = cache_state(cache)
    return {"calls": outcomes, "spend": spend, "markers": markers}


def run(input_value: Object) -> Json:
    fields = object_of(input_value, "input")
    uptime = uptime_of(fields.get("uptime_seconds"))
    memory = object_of(fields.get("memory", {}), "memory")
    calls = calls_of(fields.get("calls"))
    # only the offset between process start and now is observed, never the clock's value
    started = time.monotonic() - uptime
    with patch.object(limiter_module, "_PROCESS_STARTED_AT", started):
        return asyncio.run(execute(memory, calls))
