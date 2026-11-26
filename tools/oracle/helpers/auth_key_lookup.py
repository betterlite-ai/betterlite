import asyncio
from typing import Final, Literal, TypeAlias

import httpx
import litellm
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
from opentelemetry.trace import Span
from pydantic import BaseModel, Field, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

litellm.suppress_debug_info = True

Row = dict[str, JsonValue]
LoaderArg: TypeAlias = str | bool | None | Span | PrismaClient | UserApiKeyCache | ProxyLogging
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
JSON_OBJECT: Final[TypeAdapter[Row]] = TypeAdapter(Row)
TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)
TEXTS: Final[TypeAdapter[list[str]]] = TypeAdapter(list[str])
TIMEOUT: Final[TypeAdapter[float | None]] = TypeAdapter(float | None)
AUTH_KEYS: Final = ("key_name", "user_id", "team_id", "team_alias", "user_role", "is_session_token", "object_permission_id")  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py


class MappingRow(BaseModel):
    token: str
    jwt_claim_name: str
    jwt_claim_value: str
    jwt_issuer: str = ""


MAPPING_ROWS: Final[TypeAdapter[list[MappingRow]]] = TypeAdapter(list[MappingRow])


class KeyStep(BaseModel):
    kind: Literal["transport", "row"]
    fields: Row | None = None


class Call(BaseModel):
    aspect: Literal["cache_key", "mapping", "key_object", "session"]
    claim_name: str | None = None
    claim_value: str | None = None
    issuer: str | None = None
    rows: list[JsonValue] = Field(default_factory=list)
    lookup: Row | None = None
    hashed_token: str | None = None
    cached: Row | None = None
    reconnect: Row | None = None
    permissions: Row | None = None
    check_db_only: bool = False
    check_cache_only: bool | None = None
    auth: Row | None = None
    user: Row | None = None
    user_outcome: Literal["missing", "gone", "outage"] = "missing"
    team: Row | None = None
    membership: Row | None = None
    writers: list[bool] = Field(default_factory=list)
    reconnects: list[JsonValue] = Field(default_factory=list)
    permission_calls: list[bool] = Field(default_factory=list)
    mapping_rows: list[MappingRow] = Field(default_factory=list)
    found: MappingRow | None = None
    where: list[JsonValue] = Field(default_factory=list)
    table: LiteLLM_ObjectPermissionTable | None = None
    permission_fail: bool = False
    writes: int = 0


class MappingDb(PrismaWrapper):
    def __init__(self, state: Call) -> None:
        self.state = state
        self.litellm_jwtkeymapping = self

    async def find_many(self, *, where: Row, **_opts: JsonValue) -> list[MappingRow]:
        self.state.where.append(where)
        return self.state.mapping_rows

    async def find_first(self, *, where: Row, **_opts: JsonValue) -> MappingRow | None:
        self.state.where.append(where)
        return self.state.found


class FakeClient(PrismaClient):
    def __init__(self, state: Call) -> None:
        self.state = state
        self.db = MappingDb(state)
        reconnect = state.reconnect
        timeout = None if reconnect is None else TIMEOUT.validate_python(reconnect.get("timeout"))
        lock = None if reconnect is None else TIMEOUT.validate_python(reconnect.get("lock"))
        if timeout is not None:
            self._db_auth_reconnect_timeout_seconds = timeout
        if lock is not None:
            self._db_auth_reconnect_lock_timeout_seconds = lock

    async def __call__(self, object_permission_id: str, *_args: LoaderArg, **_kwargs: LoaderArg) -> LiteLLM_ObjectPermissionTable | None:  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3982-L3989
        del object_permission_id, _args
        self.state.permission_calls.append(_kwargs.get("check_db_only") is True)
        if self.state.permission_fail:
            raise PermissionReadError("the object permission read failed")
        return self.state.table

    @override
    async def get_data(self, **_opts: JsonValue) -> BaseModel | None:  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L4862-L4885
        self.state.writers.append(_opts.get("use_writer") is True)
        step = self.state.rows.pop(0) if self.state.rows else None
        if step is None:
            return None
        row = KeyStep.model_validate(step)
        if row.kind == "transport":
            raise httpx.ConnectError("connection closed")
        return LiteLLM_VerificationTokenView.model_validate(row.fields)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py

    @override
    async def attempt_db_reconnect(self, reason: str, **_opts: JsonValue) -> bool:  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L6434-L6450
        timeout = TIMEOUT.validate_python(_opts.get("timeout_seconds"))
        lock = TIMEOUT.validate_python(_opts.get("lock_timeout_seconds"))
        record: dict[str, JsonValue] = {"reason": reason, "timeout": timeout, "lock": lock}
        self.state.reconnects.append(JSON.validate_python(record))
        reconnect = self.state.reconnect
        answer = None if reconnect is None else reconnect.get("answer")
        return answer is True


class PermissionReadError(Exception):
    pass


class KeyCache(UserApiKeyCache):
    def __init__(self, state: Call) -> None:
        super().__init__()
        self.state = state

    @override
    async def async_get_cache(self, key: str, **_kwargs: JsonValue) -> UserAPIKeyAuth | None:  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/user_api_key_cache.py#L175-L213
        del key
        cached = self.state.cached
        return None if cached is None else UserAPIKeyAuth.model_validate(cached)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py

    @override
    async def async_set_cache(self, key: str, value: BaseModel, *, model_type: type[BaseModel], ttl: float) -> None:  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/user_api_key_cache.py#L215-L233
        del key, value, model_type, ttl
        self.state.writes += 1


class Scripted:
    def __init__(self, state: Call) -> None:
        self.state = state


class ScriptedUser(Scripted):
    async def __call__(self, *_args: LoaderArg, **_kwargs: LoaderArg) -> LiteLLM_UserTable | None:
        outcome = self.state.user_outcome
        if outcome == "gone":
            raise UserNotFoundError("gone")
        if outcome == "outage":
            raise httpx.ConnectError("connection closed")
        user = self.state.user
        return None if user is None else LiteLLM_UserTable.model_validate(user)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py


class ScriptedTeam(Scripted):
    async def __call__(self, *_args: LoaderArg, **_kwargs: LoaderArg) -> LiteLLM_TeamTableCachedObj:
        if self.state.team is None:
            raise TeamNotFoundError("gone")
        return LiteLLM_TeamTableCachedObj.model_validate(self.state.team)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py


class ScriptedMembership(Scripted):
    async def __call__(self, *_args: LoaderArg, **_kwargs: LoaderArg) -> LiteLLM_TeamMembership | None:  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py
        membership = self.state.membership
        return None if membership is None else LiteLLM_TeamMembership.model_validate(membership)


def auth_fields(auth: UserAPIKeyAuth) -> JsonValue:
    view = JSON_OBJECT.validate_json(auth.model_dump_json())
    record: dict[str, JsonValue] = {name: view.get(name) for name in AUTH_KEYS}
    empty_models: list[JsonValue] = []
    record["models"] = empty_models if view.get("models") is None else view.get("models")
    record["team_models"] = empty_models if view.get("team_models") is None else view.get("team_models")
    record["has_object_permission"] = view.get("object_permission") is not None
    return record


def cache_key(state: Call) -> JsonValue:
    claim_name = TEXT.validate_python(state.claim_name)
    claim_value = TEXT.validate_python(state.claim_value)
    return {"key": auth_checks.jwt_key_mapping_cache_key(claim_name, claim_value, state.issuer)}


async def mapping(state: Call) -> JsonValue:
    lookup = state.lookup
    if lookup is None:
        raise ValueError("a mapping call needs a lookup")
    state.mapping_rows = MAPPING_ROWS.validate_python(state.rows)
    found = lookup.get("found")
    state.found = None if found is None else MappingRow.model_validate(found)
    client = FakeClient(state)
    record: dict[str, JsonValue] = {}
    mode = lookup.get("mode")
    if mode == "token":
        hashed_token = TEXT.validate_python(lookup.get("hashed_token"))
        keys: tuple[str, ...] = await auth_checks.get_jwt_key_mapping_cache_keys_for_token(hashed_token, client)  # type: ignore[misc]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3827-L3836
        record["keys"] = JSON.validate_python(list(keys))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3827-L3836
    elif mode == "tokens":
        hashed_tokens = TEXTS.validate_python(lookup.get("hashed_tokens"))
        batched = await auth_checks.get_jwt_key_mapping_cache_keys_for_tokens(hashed_tokens, client)
        record["keys"] = JSON.validate_python(list(batched))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3843-L3852
    else:
        claim_name = TEXT.validate_python(state.claim_name)
        claim_value = TEXT.validate_python(state.claim_value)
        token: str | None = await auth_checks.get_jwt_key_mapping_object(claim_name, claim_value, client, state.issuer)  # type: ignore[misc]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3855-L3881
        record["token"] = token
    record["where"] = state.where[0] if state.where else None
    return record


async def key_object(state: Call) -> JsonValue:
    hashed_token = TEXT.validate_python(state.hashed_token)
    permission = state.permissions
    if permission is not None:
        object_id = TEXT.validate_python(permission.get("id"))
        state.table = LiteLLM_ObjectPermissionTable(object_permission_id=object_id)
        state.permission_fail = permission.get("fail") is True
    client = FakeClient(state)
    cache = KeyCache(state)
    auth_checks.get_object_permission = client
    outcome: JsonValue = "ok"
    error: JsonValue = None
    fields: JsonValue = None
    try:
        auth: UserAPIKeyAuth = await auth_checks.get_key_object(hashed_token, client, cache, check_cache_only=state.check_cache_only, check_db_only=state.check_db_only)  # type: ignore[misc]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3884-L3967
        fields = auth_fields(auth)
    except ProxyException as raised:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3926-L3932
        outcome = "missing_key"
        error_type = ProxyErrorTypes(raised.type).value
        error = {"type": error_type, "param": raised.param, "code": raised.code}
    except PermissionReadError:
        outcome = "permission"
    except Exception as raised:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3947-L3954
        message = str(raised)
        outcome = "not_in_cache" if message.startswith("Key doesn't exist in cache") else "database"
    return {
        "outcome": outcome,
        "error": error,
        "fields": fields,
        "use_writer": JSON.validate_python(state.writers),
        "cache_written": state.writes,
        "permission_calls": JSON.validate_python(state.permission_calls),
        "reconnects": state.reconnects,
    }


async def session(state: Call) -> JsonValue:
    auth: UserAPIKeyAuth = UserAPIKeyAuth.model_validate(state.auth)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py
    client = FakeClient(state)
    cache = KeyCache(state)
    logging_obj = ProxyLogging(cache)
    result: JsonValue = "ok"
    denial: JsonValue = None
    fields: JsonValue = None
    user_api_key_auth.get_user_object = ScriptedUser(state)  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L1409-L1452
    user_api_key_auth.get_team_object = ScriptedTeam(state)  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L1409-L1452
    user_api_key_auth.get_team_membership = ScriptedMembership(state)  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L1409-L1452
    try:
        refreshed: UserAPIKeyAuth = await user_api_key_auth._refresh_session_token_grants(auth, client, cache, None, logging_obj)  # noqa: SLF001, E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L1409-L1452
        fields = auth_fields(refreshed)
    except TeamNotFoundError:
        result, denial = "denied", "team_gone"
    except ProxyException:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3926-L3932
        result, denial = "denied", "user_gone"
    except HTTPException:
        result, denial = "denied", "not_a_member"
    return {"outcome": result, "denial": denial, "fields": fields}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    state = Call.model_validate(input_value)
    match state.aspect:
        case "cache_key":
            return cache_key(state)
        case "mapping":
            return asyncio.run(mapping(state))
        case "key_object":
            return asyncio.run(key_object(state))
        case "session":
            return asyncio.run(session(state))
