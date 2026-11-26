import asyncio
import json
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from functools import partial
from typing import Final, Literal, NotRequired, Protocol, TypedDict
from unittest import mock

import httpx
from litellm.models.object_permission import LiteLLM_ObjectPermissionTable
from litellm.proxy._types import (
    LiteLLM_TeamMembership,
    LiteLLM_TeamTableCachedObj,
    LiteLLM_UserTable,
    LiteLLM_VerificationTokenView,
    ProxyErrorTypes,
    ProxyException,
    UserAPIKeyAuth,
)
from litellm.proxy.auth import auth_checks, user_api_key_auth
from litellm.proxy.auth.auth_checks import TeamNotFoundError
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.types.proxy.auth.auth_checks import UserNotFoundError
from pydantic import BaseModel, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
Row = dict[str, JsonValue]

Aspect = Literal["cache_key", "mapping", "key_object", "session"]
Outcome = Literal["missing", "gone", "outage"]


class Call(TypedDict):
    aspect: Aspect
    claim_name: NotRequired[str]
    claim_value: NotRequired[str]
    issuer: NotRequired[str | None]
    rows: NotRequired[list[Row]]
    lookup: NotRequired[Row]
    hashed_token: NotRequired[str]
    cached: NotRequired[Row | None]
    reconnect: NotRequired[Row]
    permissions: NotRequired[Row | None]
    check_db_only: NotRequired[bool]
    check_cache_only: NotRequired[bool | None]
    auth: NotRequired[Row]
    user: NotRequired[Row | None]
    user_outcome: NotRequired[Outcome]
    team: NotRequired[Row | None]
    membership: NotRequired[Row | None]


def object_row(value: JsonValue, name: str) -> Row:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def optional_row(value: JsonValue, name: str) -> Row | None:
    return None if value is None else object_row(value, name)


def object_rows(value: JsonValue, name: str) -> list[Row]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return [object_row(item, name) for item in value]


def string_value(value: JsonValue, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def string_list(value: JsonValue, name: str) -> list[str]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return [string_value(item, name) for item in value]


def optional_number(value: JsonValue, name: str) -> float | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int | float):
        raise ValueError(f"{name} must be a number")
    return float(value)


class MappingRow:
    def __init__(self, fields: Row) -> None:
        self.token = string_value(fields.get("token"), "token")
        self.jwt_claim_name = string_value(fields.get("jwt_claim_name"), "jwt_claim_name")
        self.jwt_claim_value = string_value(fields.get("jwt_claim_value"), "jwt_claim_value")
        issuer = fields.get("jwt_issuer", "")
        self.jwt_issuer = "" if issuer is None else str(issuer)


@dataclass
class MappingScript:
    rows: list[MappingRow]
    found: MappingRow | None
    where: list[JsonValue] = field(default_factory=list)


class MappingTable:
    def __init__(self, script: MappingScript) -> None:
        self.script = script

    async def find_many(
        self,
        *,
        where: Mapping[str, object],
        include: Mapping[str, object] | None = None,
        take: int | None = None,
    ) -> Sequence[MappingRow]:
        del include, take
        self.script.where.append(JSON.validate_json(json.dumps(where)))
        return self.script.rows

    async def find_first(
        self,
        *,
        where: Mapping[str, object],
        include: Mapping[str, object] | None = None,
    ) -> MappingRow | None:
        del include
        self.script.where.append(JSON.validate_json(json.dumps(where)))
        return self.script.found


class MappingDb:
    def __init__(self, script: MappingScript) -> None:
        self.litellm_jwtkeymapping = MappingTable(script)


class MappingClient(PrismaClient):
    def __init__(self, script: MappingScript) -> None:
        self.db = MappingDb(script)


@dataclass
class ReadScript:
    steps: list[Row]
    timeout: float | None = None
    lock: float | None = None
    reconnect_answer: bool = False
    use_writer: list[bool] = field(default_factory=list)
    reconnects: list[JsonValue] = field(default_factory=list)


async def scripted_get_data(script: ReadScript, **options: object) -> BaseModel | None:
    script.use_writer.append(bool(options.get("use_writer")))
    step = script.steps.pop(0) if script.steps else None
    if step is None:
        return None
    if step.get("kind") == "transport":
        raise httpx.ConnectError("connection closed")
    fields = optional_row(step.get("fields"), "fields")
    if fields is None:
        raise ValueError("a row step needs fields")
    return LiteLLM_VerificationTokenView.model_validate(fields)


class StaleEngine(Protocol):
    def is_still_live(self, current: PrismaWrapper) -> bool: ...


class ScriptedClient(PrismaClient):
    def __init__(self, script: ReadScript) -> None:
        self.script = script
        setattr(self, "get_data", partial(scripted_get_data, script))
        if script.timeout is not None:
            self._db_auth_reconnect_timeout_seconds = script.timeout
        if script.lock is not None:
            self._db_auth_reconnect_lock_timeout_seconds = script.lock

    @override
    async def attempt_db_reconnect(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L6435
        self,
        reason: str,
        force: bool = False,
        timeout_seconds: float | None = None,
        lock_timeout_seconds: float | None = None,
        force_recreate: bool = False,
        stale_read_engine: StaleEngine | None = None,
    ) -> bool:
        del force, force_recreate, stale_read_engine
        self.script.reconnects.append(
            JSON.validate_python({
                "reason": reason,
                "timeout": timeout_seconds,
                "lock": lock_timeout_seconds,
            })
        )
        return self.script.reconnect_answer


@dataclass
class CacheScript:
    cached: UserAPIKeyAuth | None
    writes: list[UserAPIKeyAuth] = field(default_factory=list)


async def scripted_cache_get(script: CacheScript, **_kwargs: object) -> UserAPIKeyAuth | None:
    return None if script.cached is None else script.cached.model_copy(deep=True)


async def scripted_cache_put(script: CacheScript, **_kwargs: object) -> None:
    value = _kwargs.get("value")
    if isinstance(value, UserAPIKeyAuth):
        script.writes.append(value)


def cache_with(script: CacheScript) -> UserApiKeyCache:
    cache = UserApiKeyCache()
    setattr(cache, "async_get_cache", partial(scripted_cache_get, script))
    setattr(cache, "async_set_cache", partial(scripted_cache_put, script))
    return cache


class PermissionReadError(Exception):
    pass


@dataclass
class PermissionScript:
    table: LiteLLM_ObjectPermissionTable | None
    fail: bool
    calls: list[bool] = field(default_factory=list)


async def scripted_permission(
    script: PermissionScript, *, check_db_only: bool = False, **_options: object
) -> LiteLLM_ObjectPermissionTable | None:
    script.calls.append(check_db_only)
    if script.fail:
        raise PermissionReadError("the object permission read failed")
    return script.table


def permission_with(fields: Row | None) -> PermissionScript:
    if fields is None:
        return PermissionScript(table=None, fail=False)
    table = LiteLLM_ObjectPermissionTable.model_validate(
        {"object_permission_id": string_value(fields.get("id"), "id"), "models": ["gpt-4"]}
    )
    return PermissionScript(table=table, fail=bool(fields.get("fail")))


def cached_auth(fields: Row | None) -> UserAPIKeyAuth | None:
    return None if fields is None else UserAPIKeyAuth.model_validate(fields)


def auth_fields(auth: UserAPIKeyAuth) -> JsonValue:
    role = auth.user_role
    return JSON.validate_json(json.dumps({
        "key_name": auth.key_name,
        "user_id": auth.user_id,
        "team_id": auth.team_id,
        "team_alias": auth.team_alias,
        "user_role": None if role is None else role.value,
        "is_session_token": auth.is_session_token,
        "models": list(auth.models or ()),
        "team_models": list(auth.team_models or ()),
        "object_permission_id": auth.object_permission_id,
        "has_object_permission": auth.object_permission is not None,
    }))


def error_fields(error: ProxyException) -> JsonValue:
    raw_type = error.type
    if isinstance(raw_type, ProxyErrorTypes):
        error_type: str | None = raw_type.value
    else:
        error_type = None if raw_type is None else str(raw_type)
    return JSON.validate_json(json.dumps({
        "type": error_type,
        "param": error.param,
        "code": None if error.code is None else str(error.code),
    }))


def read_script(call: Call) -> ReadScript:
    reconnect = optional_row(call.get("reconnect"), "reconnect")
    if reconnect is None:
        raise ValueError("a key object call needs a reconnect fixture")
    return ReadScript(
        steps=object_rows(call.get("rows", []), "rows"),
        timeout=optional_number(reconnect.get("timeout"), "timeout"),
        lock=optional_number(reconnect.get("lock"), "lock"),
        reconnect_answer=bool(reconnect.get("answer")),
    )


async def key_object(call: Call) -> JsonValue:
    script = read_script(call)
    client = ScriptedClient(script)
    cache_script = CacheScript(cached=cached_auth(optional_row(call.get("cached"), "cached")))
    cache = cache_with(cache_script)
    permission = permission_with(optional_row(call.get("permissions"), "permissions"))
    outcome: JsonValue = "ok"
    error: JsonValue = None
    fields: JsonValue = None
    check_cache_only = call.get("check_cache_only")
    patched = mock.patch.object(
        auth_checks, "get_object_permission", partial(scripted_permission, permission)
    )
    with patched:
        try:
            auth: UserAPIKeyAuth = await auth_checks.get_key_object(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3884-L3967
                string_value(call.get("hashed_token"), "hashed_token"),
                client,
                cache,
                check_cache_only=None if check_cache_only is None else bool(check_cache_only),
                check_db_only=bool(call.get("check_db_only")),
            )
            fields = auth_fields(auth)
        except ProxyException as raised:
            outcome = "missing_key"
            error = error_fields(raised)
        except PermissionReadError:
            outcome = "permission"
        except Exception as raised:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3914-L3924
            message = str(raised)
            cache_miss = message.startswith("Key doesn't exist in cache")
            outcome = "not_in_cache" if cache_miss else "database"
    return JSON.validate_json(json.dumps({
        "outcome": outcome,
        "error": error,
        "fields": fields,
        "use_writer": script.use_writer,
        "cache_written": len(cache_script.writes),
        "permission_calls": permission.calls,
        "reconnects": script.reconnects,
    }))


async def mapping(call: Call) -> JsonValue:
    lookup = optional_row(call.get("lookup"), "lookup")
    if lookup is None:
        raise ValueError("a mapping call needs a lookup")
    mode = lookup.get("mode")
    found = None
    if mode == "object":
        found_row = optional_row(lookup.get("found"), "found")
        found = None if found_row is None else MappingRow(found_row)
    rows = [MappingRow(row) for row in object_rows(call.get("rows", []), "rows")]
    script = MappingScript(rows=rows, found=found)
    client = MappingClient(script)
    answer: dict[str, object]
    if mode == "token":
        keys: tuple[str, ...] = await auth_checks.get_jwt_key_mapping_cache_keys_for_token(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3827-L3836
            string_value(lookup.get("hashed_token"), "hashed_token"), client
        )
        answer = {"keys": list(keys)}
    elif mode == "tokens":
        batch = string_list(lookup.get("hashed_tokens"), "hashed_tokens")
        keys = await auth_checks.get_jwt_key_mapping_cache_keys_for_tokens(batch, client)
        answer = {"keys": list(keys)}
    elif mode == "object":
        issuer = call.get("issuer")
        token: str | None = await auth_checks.get_jwt_key_mapping_object(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3855-L3881
            string_value(call.get("claim_name"), "claim_name"),
            string_value(call.get("claim_value"), "claim_value"),
            client,
            None if issuer is None else str(issuer),
        )
        answer = {"token": token}
    else:
        raise ValueError("a mapping lookup is token, tokens or object")
    where = None if not script.where else script.where[0]
    return JSON.validate_json(json.dumps({**answer, "where": where}))


@dataclass
class SessionScript:
    user: LiteLLM_UserTable | None
    outcome: Outcome
    team: LiteLLM_TeamTableCachedObj | None
    membership: LiteLLM_TeamMembership | None


async def scripted_user(script: SessionScript, **_kwargs: object) -> LiteLLM_UserTable | None:
    if script.outcome == "gone":
        raise UserNotFoundError(str(_kwargs.get("user_id")))
    if script.outcome == "outage":
        raise httpx.ConnectError("connection closed")
    return script.user


async def scripted_team(script: SessionScript, **_kwargs: object) -> LiteLLM_TeamTableCachedObj:
    if script.team is None:
        raise TeamNotFoundError(str(_kwargs.get("team_id")))
    return script.team


async def scripted_membership(
    script: SessionScript, **_kwargs: object
) -> LiteLLM_TeamMembership | None:
    return script.membership


def session_script(call: Call) -> SessionScript:
    user = optional_row(call.get("user"), "user")
    team = optional_row(call.get("team"), "team")
    membership = optional_row(call.get("membership"), "membership")
    return SessionScript(
        user=None if user is None else LiteLLM_UserTable.model_validate(user),
        outcome=call.get("user_outcome", "missing"),
        team=None if team is None else LiteLLM_TeamTableCachedObj.model_validate(team),
        membership=(
            None if membership is None else LiteLLM_TeamMembership.model_validate(membership)
        ),
    )


def denial_kind(raised: Exception) -> str:
    if isinstance(raised, TeamNotFoundError):
        return "team_gone"
    if isinstance(raised, ProxyException):
        return "user_gone"
    return "not_a_member"


async def session(call: Call) -> JsonValue:
    script = session_script(call)
    auth_row = optional_row(call.get("auth"), "auth")
    if auth_row is None:
        raise ValueError("a session call needs the minted auth")
    auth = UserAPIKeyAuth.model_validate(auth_row)
    client = ScriptedClient(ReadScript(steps=[]))
    cache = cache_with(CacheScript(cached=None))
    logging_obj = ProxyLogging(cache)
    outcome: JsonValue = "ok"
    denial: JsonValue = None
    fields: JsonValue = None
    with (
        mock.patch.object(user_api_key_auth, "get_user_object", partial(scripted_user, script)),
        mock.patch.object(user_api_key_auth, "get_team_object", partial(scripted_team, script)),
        mock.patch.object(
            user_api_key_auth, "get_team_membership", partial(scripted_membership, script)
        ),
    ):
        try:
            refreshed: UserAPIKeyAuth = await user_api_key_auth._refresh_session_token_grants(  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L1409-L1452
                auth, client, cache, None, logging_obj
            )
            fields = auth_fields(refreshed)
        except (ProxyException, TeamNotFoundError, HTTPException) as raised:
            outcome = "denied"
            denial = denial_kind(raised)
    return JSON.validate_json(json.dumps({"outcome": outcome, "denial": denial, "fields": fields}))


async def answer(call: Call) -> JsonValue:
    aspect = call["aspect"]
    if aspect == "cache_key":
        issuer = call.get("issuer")
        return JSON.validate_python({
            "key": auth_checks.jwt_key_mapping_cache_key(
                string_value(call.get("claim_name"), "claim_name"),
                string_value(call.get("claim_value"), "claim_value"),
                None if issuer is None else str(issuer),
            )
        })
    if aspect == "mapping":
        return await mapping(call)
    if aspect == "key_object":
        return await key_object(call)
    return await session(call)


def run(input_value: Call) -> JsonValue:
    return asyncio.run(answer(input_value))
