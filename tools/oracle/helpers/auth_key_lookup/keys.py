from typing import Protocol, TypeAlias

import httpx
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.models.object_permission import LiteLLM_ObjectPermissionTable
from litellm.proxy._types import LiteLLM_VerificationTokenView, ProxyException, UserAPIKeyAuth
from litellm.proxy.auth import auth_checks
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient, ProxyLogging
from opentelemetry.trace import Span
from pydantic import BaseModel, JsonValue
from typing_extensions import override

from .inputs import JSON, STEP_LIST, Call, Permission, Row, Step
from .views import auth_fields, error_fields

# every argument type the scripted readers take, so each callable accepts their keyword calls
LookupArgument: TypeAlias = (
    str | bool | None | Span | PrismaClient | UserApiKeyCache | ProxyLogging
)


class ReadScript:
    def __init__(
        self,
        steps: list[Step],
        timeout: float | None = None,
        lock: float | None = None,
        answer: bool = False,
    ) -> None:
        self.steps = steps
        self.timeout = timeout
        self.lock = lock
        self.answer = answer
        self.use_writer: list[bool] = []
        self.reconnects: list[JsonValue] = []


class StaleEngine(Protocol):
    def is_still_live(self, current: PrismaWrapper) -> bool: ...


class ScriptedClient(PrismaClient):
    def __init__(self, script: ReadScript) -> None:
        self.script = script
        if script.timeout is not None:
            self._db_auth_reconnect_timeout_seconds = script.timeout
        if script.lock is not None:
            self._db_auth_reconnect_lock_timeout_seconds = script.lock

    @override
    async def get_data(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L4862-L4885
        self, **_options: JsonValue
    ) -> BaseModel | None:
        used_writer = _options.get("use_writer")
        self.script.use_writer.append(used_writer is True)
        step = self.script.steps.pop(0) if self.script.steps else None
        if step is None:
            return None
        match step["kind"]:
            case "transport":
                raise httpx.ConnectError("connection closed")
            case "row":
                fields = step.get("fields")
                if fields is None:
                    raise ValueError("a row step needs fields")
                row: BaseModel = LiteLLM_VerificationTokenView.model_validate(fields)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py
                return row

    @override
    async def attempt_db_reconnect(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L6434-L6450
        self,
        reason: str,
        force: bool = False,
        timeout_seconds: float | None = None,
        lock_timeout_seconds: float | None = None,
        force_recreate: bool = False,
        stale_read_engine: StaleEngine | None = None,
    ) -> bool:
        del force, force_recreate, stale_read_engine
        record: dict[str, JsonValue] = {
            "reason": reason,
            "timeout": timeout_seconds,
            "lock": lock_timeout_seconds,
        }
        self.script.reconnects.append(JSON.validate_python(record))
        return self.script.answer


class CacheScript:
    def __init__(self, cached: Row | None) -> None:
        self.cached = cached
        self.writes = 0


class SeededCache(InMemoryCache):
    def __init__(self, script: CacheScript) -> None:
        super().__init__()
        self.script = script

    @override
    def get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        del key
        return self.script.cached


class KeyCache(UserApiKeyCache):
    def __init__(self, script: CacheScript) -> None:
        super().__init__(in_memory_cache=SeededCache(script), default_in_memory_ttl=60.0)
        self.script = script

    @override
    async def async_set_cache(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/user_api_key_cache.py#L215-L233
        self, key: str, value: BaseModel, *, model_type: type[BaseModel], ttl: float
    ) -> None:
        del key, value, model_type, ttl
        self.script.writes += 1


class PermissionReadError(Exception):
    pass


class PermissionScript:
    def __init__(self, table: LiteLLM_ObjectPermissionTable | None, fail: bool) -> None:
        self.table = table
        self.fail = fail
        self.calls: list[bool] = []


class PermissionReader:
    def __init__(self, script: PermissionScript) -> None:
        self.script = script

    async def __call__(
        self,
        object_permission_id: str,
        *_args: LookupArgument,
        **_kwargs: LookupArgument,
    ) -> LiteLLM_ObjectPermissionTable | None:
        del object_permission_id
        db_only = _kwargs.get("check_db_only")
        self.script.calls.append(db_only is True)
        if self.script.fail:
            raise PermissionReadError("the object permission read failed")
        return self.script.table


def permission_script(permission: Permission | None) -> PermissionScript:
    if permission is None:
        return PermissionScript(None, False)
    models: list[str] = ["gpt-4"]
    record: dict[str, JsonValue] = {
        "object_permission_id": permission["id"],
        "models": JSON.validate_python(models),
    }
    table: LiteLLM_ObjectPermissionTable = LiteLLM_ObjectPermissionTable.model_validate(record)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/object_permission.py
    failed = permission.get("fail")
    return PermissionScript(table, failed is True)


def read_steps(rows: list[JsonValue] | None) -> list[Step]:
    if rows is None:
        return []
    return STEP_LIST.validate_python(rows)


def read_script(call: Call) -> ReadScript:
    reconnect = call.get("reconnect")
    if reconnect is None:
        raise ValueError("a key object call needs a reconnect fixture")
    answer = reconnect.get("answer")
    return ReadScript(
        read_steps(call.get("rows")),
        reconnect.get("timeout"),
        reconnect.get("lock"),
        False if answer is None else answer,
    )


async def key_object(call: Call) -> JsonValue:
    hashed_token = call.get("hashed_token")
    if hashed_token is None:
        raise ValueError("a key object call needs hashed_token")
    script = read_script(call)
    client = ScriptedClient(script)
    cache_script = CacheScript(call.get("cached"))
    cache = KeyCache(cache_script)
    permission = permission_script(call.get("permissions"))
    auth_checks.get_object_permission = PermissionReader(permission)
    outcome: JsonValue = "ok"
    error: JsonValue = None
    fields: JsonValue = None
    try:
        auth: UserAPIKeyAuth = await auth_checks.get_key_object(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3884-L3967
            hashed_token,
            client,
            cache,
            check_cache_only=call.get("check_cache_only"),
            check_db_only=call.get("check_db_only") is True,
        )
        fields = auth_fields(auth)
    except ProxyException as raised:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3924-L3931
        outcome = "missing_key"
        error = error_fields(raised)
    except PermissionReadError:
        outcome = "permission"
    except Exception as raised:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3933-L3960
        message = str(raised)
        outcome = "not_in_cache" if message.startswith("Key doesn't exist in cache") else "database"
    record: dict[str, JsonValue] = {
        "outcome": outcome,
        "error": error,
        "fields": fields,
        "use_writer": JSON.validate_python(script.use_writer),
        "cache_written": cache_script.writes,
        "permission_calls": JSON.validate_python(permission.calls),
        "reconnects": script.reconnects,
    }
    return JSON.validate_python(record)
