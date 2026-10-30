import asyncio
import sys
import types
from collections.abc import Callable
from typing import TypeAlias, cast
from unittest.mock import MagicMock, patch

import httpx
from litellm.proxy._types import ProxyErrorTypes, ProxyException
from litellm.proxy.db.db_lookup_gate import DBLookupDeadlineExceeded
from litellm.proxy.db.exception_handler import (
    PrismaDBExceptionHandler,
    call_with_db_reconnect_retry,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _FakeError(Exception):
    message: str
    code: object
    meta: object


def _exception_class(name: str, base: type[_FakeError], module: str) -> type[_FakeError]:
    def init(self: _FakeError, message: str = "") -> None:
        Exception.__init__(self, message)
        self.message = message

    def stringify(self: _FakeError) -> str:
        return cast(str, getattr(self, "message", ""))

    return type(name, (base,), {"__module__": module, "__init__": init, "__str__": stringify})


def _prisma_types() -> dict[str, type[_FakeError]]:
    errors = types.ModuleType("prisma.errors")
    engine_errors = types.ModuleType("prisma.engine.errors")
    prisma = types.ModuleType("prisma")
    engine = types.ModuleType("prisma.engine")
    prisma_error = _exception_class("PrismaError", _FakeError, "prisma.errors")
    data_error = _exception_class("DataError", prisma_error, "prisma.errors")
    names = {
        "PrismaError": prisma_error,
        "DataError": data_error,
        "UniqueViolationError": _exception_class(
            "UniqueViolationError", data_error, "prisma.errors"
        ),
        "ForeignKeyViolationError": _exception_class(
            "ForeignKeyViolationError", data_error, "prisma.errors"
        ),
        "MissingRequiredValueError": _exception_class(
            "MissingRequiredValueError", data_error, "prisma.errors"
        ),
        "RawQueryError": _exception_class("RawQueryError", data_error, "prisma.errors"),
        "TableNotFoundError": _exception_class("TableNotFoundError", data_error, "prisma.errors"),
        "RecordNotFoundError": _exception_class("RecordNotFoundError", data_error, "prisma.errors"),
        "ClientNotConnectedError": _exception_class(
            "ClientNotConnectedError", prisma_error, "prisma.errors"
        ),
        "HTTPClientClosedError": _exception_class(
            "HTTPClientClosedError", prisma_error, "prisma.errors"
        ),
    }
    names["EngineConnectionError"] = _exception_class(
        "EngineConnectionError", prisma_error, "prisma.engine.errors"
    )
    for name, value in names.items():
        setattr(errors, name, value)
    engine_error_name = "EngineConnectionError"
    setattr(engine_errors, engine_error_name, names[engine_error_name])
    for module, name, value in (
        (prisma, "errors", errors),
        (prisma, "engine", engine),
        (engine, "errors", engine_errors),
    ):
        setattr(module, name, value)
    sys.modules.update(
        {
            "prisma": prisma,
            "prisma.errors": errors,
            "prisma.engine": engine,
            "prisma.engine.errors": engine_errors,
        }
    )
    return names


def _make_error(error: dict[str, Json], types_by_name: dict[str, type[_FakeError]]) -> Exception:
    kind = cast(str, error["error_type"])
    message = cast(str, error.get("message", ""))
    transport_types = {
        "connect": httpx.ConnectError,
        "connect_timeout": httpx.ConnectTimeout,
        "read": httpx.ReadError,
        "read_timeout": httpx.ReadTimeout,
    }
    if kind in transport_types:
        return transport_types[kind](message)
    if kind == "deadline":
        return DBLookupDeadlineExceeded("database", 1.0)
    if kind == "no_db_connection":
        return ProxyException(message, ProxyErrorTypes.no_db_connection, None)
    type_names = {
        "prisma": "PrismaError",
        "data": "DataError",
        "unique_violation": "UniqueViolationError",
        "foreign_key_violation": "ForeignKeyViolationError",
        "missing_required_value": "MissingRequiredValueError",
        "raw_query": "RawQueryError",
        "table_not_found": "TableNotFoundError",
        "record_not_found": "RecordNotFoundError",
        "client_not_connected": "ClientNotConnectedError",
        "http_client_closed": "HTTPClientClosedError",
        "engine_connection": "EngineConnectionError",
    }
    if kind in type_names:
        error_value = types_by_name[type_names[kind]](message)
        fake_value = error_value
        if error.get("code") is not None:
            fake_value.code = error["code"]
        if error.get("meta_code") is not None:
            fake_value.meta = {"code": error["meta_code"]}
        return error_value
    if error.get("traceback_modules"):
        modules = cast(list[Json], error["traceback_modules"])
        namespace = {"__name__": cast(str, modules[0]), "message": message}
        function = types.FunctionType(
            compile("raise Exception(message)", "database_faults", "exec"), namespace
        )
        try:
            function()
        except Exception as error_value:  # noqa: BLE001 - the fixture deliberately raises a generic engine error
            return error_value
    return Exception(message)


def run(input_value: dict[str, Json]) -> Json:
    types_by_name = _prisma_types()
    if "scenarios" in input_value:
        return [
            asyncio.run(_retry(cast(dict[str, Json], script), types_by_name))
            for script in cast(list[Json], input_value["scenarios"])
        ]
    if "errors" in input_value:
        return [run({"error": error}) for error in cast(list[Json], input_value["errors"])]
    error = cast(dict[str, Json], input_value["error"])
    value = _make_error(error, types_by_name)
    return {
        "connection": PrismaDBExceptionHandler.is_database_connection_error(value),
        "infrastructure": PrismaDBExceptionHandler.is_database_infrastructure_error(value),
        "data": PrismaDBExceptionHandler.is_prisma_data_error(value),
        "transport": PrismaDBExceptionHandler.is_database_transport_error(value),
        "prisma": PrismaDBExceptionHandler.is_prisma_error(value),
        "deadlock": PrismaDBExceptionHandler.is_deadlock_error(value),
        "sqlstate": PrismaDBExceptionHandler.postgres_sqlstate(value),
        "read_only": PrismaDBExceptionHandler.is_read_only_transaction_error(value),
        "engine_internal": PrismaDBExceptionHandler.is_prisma_engine_internal_error(value),
        "permanent": PrismaDBExceptionHandler.is_permanent_database_fault(value),
    }


def _timeout(value: Json) -> object:
    if isinstance(value, dict) and value.get("mock") is True:
        return MagicMock()
    return value


class _Client:
    def __init__(
        self,
        script: dict[str, Json],
        effects: list[Json],
        make_error: Callable[[dict[str, Json]], Exception],
    ) -> None:
        self.script = script
        self.effects = effects
        self.make_error = make_error
        self._db_auth_reconnect_timeout_seconds: object = _timeout(
            script.get("client_timeout_seconds")
        )
        self._db_auth_reconnect_lock_timeout_seconds: object = _timeout(
            script.get("client_lock_timeout_seconds")
        )

    async def attempt_db_reconnect(
        self,
        *,
        reason: str,
        timeout_seconds: float | None = None,
        lock_timeout_seconds: float | None = None,
    ) -> bool:
        self.effects.append(
            {
                "effect": "reconnect",
                "reason": reason,
                "timeout_seconds": timeout_seconds,
                "lock_timeout_seconds": lock_timeout_seconds,
            }
        )
        outcome = cast(dict[str, Json], self.script["reconnect"])
        if "Err" in outcome:
            raise self.make_error(cast(dict[str, Json], outcome["Err"]))
        return cast(bool, outcome["Ok"])


async def _retry(script: dict[str, Json], types_by_name: dict[str, type[_FakeError]]) -> Json:
    effects: list[Json] = []
    reads = iter(cast(list[dict[str, Json]], script["reads"]))

    def make_error(value: dict[str, Json]) -> Exception:
        return _make_error(value, types_by_name)

    async def read() -> int:
        effects.append({"effect": "read"})
        outcome = next(reads)
        if "Err" in outcome:
            raise make_error(cast(dict[str, Json], outcome["Err"]))
        return cast(int, outcome["Ok"])

    def warning(message: str, reason: str, error: Exception) -> None:
        key = "reconnect_error" if "reconnect_error=%s" in message else "error"
        effects.append({"effect": "warning", "fields": {"reason": reason, key: str(error)}})

    allowed = cast(list[str] | None, script.get("retry_safe_error_types"))
    safe_types = (
        None
        if allowed is None
        else tuple(type(make_error({"error_type": kind})) for kind in allowed)
    )
    client = (
        _Client(script, effects, make_error) if script.get("reconnect") is not None else object()
    )
    result: Json
    cause: Json = None
    with patch(
        "litellm.proxy.db.exception_handler.verbose_proxy_logger.warning", side_effect=warning
    ):
        try:
            value = await call_with_db_reconnect_retry(
                cast(_Client, client),
                read,
                reason=cast(str, script["reason"]),
                retry_safe_error_types=safe_types,
                timeout_seconds=cast(float | None, script.get("timeout_seconds")),
                lock_timeout_seconds=cast(float | None, script.get("lock_timeout_seconds")),
            )
            result = {"Ok": value}
        except Exception as error:  # noqa: BLE001 - record the owned helper's exception without replacing its class
            result = {"Err": type(error).__name__}
            cause = type(error.__cause__).__name__ if error.__cause__ is not None else None
    return {"name": script["name"], "result": result, "cause": cause, "effects": effects}
