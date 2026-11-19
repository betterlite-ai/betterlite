import asyncio
from typing import Final, Literal, NotRequired, Protocol, TypedDict

from litellm.caching.in_memory_cache import InMemoryCache
from litellm.models.access_group import LiteLLM_AccessGroupTable
from litellm.models.team import LiteLLM_TeamTableCachedObj
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.auth.auth_checks import (
    _cache_access_object,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3319-L3331
    _cache_key_object,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3022-L3044
    _cache_management_object,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2771-L2789
    _cache_team_object,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2792-L2846
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.utils import ProxyLogging
from pydantic import JsonValue, TypeAdapter

CLOCK_NOW: Final[float] = 1000.0

JSON_VALUES: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
OBJECTS: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
FLOATS: Final[TypeAdapter[float]] = TypeAdapter(float)
TEAMS: Final[TypeAdapter[LiteLLM_TeamTableCachedObj]] = TypeAdapter(LiteLLM_TeamTableCachedObj)
ACCESS_GROUPS: Final[TypeAdapter[LiteLLM_AccessGroupTable]] = TypeAdapter(LiteLLM_AccessGroupTable)
API_KEYS: Final[TypeAdapter[UserAPIKeyAuth]] = TypeAdapter(UserAPIKeyAuth)


class KeyWire(TypedDict):
    request_route: NotRequired[JsonValue]
    last_refreshed_at: NotRequired[JsonValue]


KEY_WIRES: Final[TypeAdapter[KeyWire]] = TypeAdapter(KeyWire)


class CacheView(Protocol):
    cache_dict: dict[str, JsonValue]
    ttl_dict: dict[str, JsonValue]


class FixedClock:
    def __call__(self) -> float:
        return CLOCK_NOW


class Input(TypedDict):
    scenario: Literal["management", "team", "key", "access"]
    management_ttl: float | None
    access_group_ttl: float
    team_id: str
    team: dict[str, JsonValue]
    stale_alias: JsonValue
    hashed_token: str
    rehash_candidate: str
    auth: dict[str, JsonValue]
    access_group_id: str
    table: dict[str, JsonValue]


def build_cache(call: Input) -> UserApiKeyCache:
    clock = FixedClock()
    return UserApiKeyCache(
        in_memory_cache=InMemoryCache(clock=clock),
        redis_cache=None,
        default_in_memory_ttl=call["management_ttl"],
        default_redis_ttl=None,
        key_object_in_memory_cache=InMemoryCache(clock=clock),
    )


def entry(memory: CacheView, key: str) -> tuple[dict[str, JsonValue], float]:
    payload = OBJECTS.validate_python(memory.cache_dict.get(key))
    ttl = FLOATS.validate_python(memory.ttl_dict.get(key)) - CLOCK_NOW
    return payload, ttl


async def management_write(
    cache: UserApiKeyCache,
    team: LiteLLM_TeamTableCachedObj,
    logging: ProxyLogging | None,
) -> tuple[dict[str, JsonValue], float]:
    key = f"team_id:{team.team_id}"
    await _cache_management_object(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2771-L2789
        key=key,
        value=team,
        user_api_key_cache=cache,
        proxy_logging_obj=logging,
        model_type=LiteLLM_TeamTableCachedObj,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2771-L2789
    )
    memory: CacheView = cache.in_memory_cache_for(key)
    return entry(memory, key)


async def management_report(call: Input) -> JsonValue:
    cache = build_cache(call)
    team = TEAMS.validate_python(call["team"])
    first, ttl = await management_write(cache, team, None)
    second, _ = await management_write(
        cache, team, ProxyLogging(user_api_key_cache=cache)
    )
    key = f"team_id:{team.team_id}"
    return {
        "management_model_type_and_ttl": {
            "key": key,
            "ttl": ttl,
            "team_id": first.get("team_id"),
            "team_alias": first.get("team_alias"),
        },
        "management_logging_argument_unused": first == second,
    }


async def team_refresh_report(call: Input) -> JsonValue:
    cache = build_cache(call)
    team = TEAMS.validate_python(call["team"])
    alias = team.team_alias
    alias_key: str | None = None
    alias_present = False
    if alias:
        alias_key = f"team_alias:{alias}"
        cache.set_cache(alias_key, call["stale_alias"])  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L150-L160
    was_none = team.last_refreshed_at is None
    await _cache_team_object(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2792-L2846
        team_id=call["team_id"],
        team_table=team,
        user_api_key_cache=cache,
        proxy_logging_obj=None,
    )
    key = f"team_id:{call['team_id']}"
    memory: CacheView = cache.in_memory_cache_for(key)
    stored, ttl = entry(memory, key)
    if alias_key is not None:
        alias_memory: CacheView = cache.in_memory_cache_for(alias_key)
        alias_present = alias_key in alias_memory.cache_dict
    return {
        "team_id_and_alias_keys": {
            "team_key": key,
            "team_key_ttl": ttl,
            "alias_key": alias_key,
            "alias_present": alias_present,
        },
        "team_timestamp_mutates_input": {
            "input_was_none": was_none,
            "input_now_is_none": team.last_refreshed_at is None,
            "stored_has_timestamp": stored.get("last_refreshed_at") is not None,
            "stored_equals_input": stored.get("last_refreshed_at")
            == team.last_refreshed_at,
        },
    }


async def key_refresh_report(call: Input) -> JsonValue:
    cache = build_cache(call)
    key = API_KEYS.validate_python(call["auth"])
    hashed = call["hashed_token"]
    await _cache_key_object(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3022-L3044
        hashed_token=hashed,
        user_api_key_obj=key,
        user_api_key_cache=cache,
        proxy_logging_obj=None,
    )
    memory: CacheView = cache.in_memory_cache_for(hashed)
    stored, ttl = entry(memory, hashed)
    wire = KEY_WIRES.validate_json(key.model_dump_json())
    candidate = call["rehash_candidate"]
    candidate_memory: CacheView = cache.in_memory_cache_for(candidate)
    return {
        "key_timestamp_and_request_field_copy": {
            "stored_ttl": ttl,
            "stored_request_route_is_null": stored.get("request_route") is None,
            "stored_has_timestamp": stored.get("last_refreshed_at") is not None,
        },
        "key_hash_not_recomputed": {
            "supplied_key_present": hashed in memory.cache_dict,
            "rehash_key_present": candidate in candidate_memory.cache_dict,
        },
        "key_copy_retains_original_request_fields": {
            "original_request_route": wire.get("request_route"),
            "original_last_refreshed_at_matches_stored": wire.get("last_refreshed_at")
            == stored.get("last_refreshed_at"),
        },
    }


async def access_group_report(call: Input) -> JsonValue:
    cache = build_cache(call)
    table = ACCESS_GROUPS.validate_python(call["table"])
    key = f"access_group_id:{call['access_group_id']}"
    await _cache_access_object(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3319-L3331
        access_group_id=call["access_group_id"],
        access_group_table=table,
        user_api_key_cache=cache,
        proxy_logging_obj=None,
    )
    memory: CacheView = cache.in_memory_cache_for(key)
    stored, ttl = entry(memory, key)
    return {
        "access_group_key_and_independent_ttl": {
            "key": key,
            "ttl": ttl,
            "access_group_id": stored.get("access_group_id"),
            "access_group_name": stored.get("access_group_name"),
        },
    }


async def dispatch(call: Input) -> JsonValue:
    scenario = call["scenario"]
    if scenario == "management":
        return await management_report(call)
    if scenario == "team":
        return await team_refresh_report(call)
    if scenario == "key":
        return await key_refresh_report(call)
    return await access_group_report(call)


def run(input_value: Input) -> JsonValue:
    result = asyncio.run(dispatch(input_value))
    return JSON_VALUES.validate_python(result)
