import asyncio
import sys
import types
from types import ModuleType
from typing import TypeAlias, cast

import httpx
from litellm.proxy._types import ProxyErrorTypes, ProxyException
from litellm.proxy.db.db_lookup_gate import DBLookupDeadlineExceeded
from litellm.proxy.db.exception_handler import (
    _TRANSIENT_DB_UNAVAILABLE_MESSAGE,  # pyright: ignore[reportPrivateUsage]
    PrismaDBExceptionHandler,
    _exception_chain,  # pyright: ignore[reportPrivateUsage]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_PROXY_SERVER = "litellm.proxy.proxy_server"
_SETTINGS: dict[str, Json] = {}


class _FakeError(Exception):
    message: str
    code: object
    meta: object


def _error_class(name: str, base: type[_FakeError], module: str) -> type[_FakeError]:
    def init(self: _FakeError, message: str = "") -> None:
        Exception.__init__(self, message)
        self.message = message

    def stringify(self: _FakeError) -> str:
        return self.message

    return type(name, (base,), {"__module__": module, "__init__": init, "__str__": stringify})


def _install_prisma() -> dict[str, type[_FakeError]]:
    errors = ModuleType("prisma.errors")
    engine_errors = ModuleType("prisma.engine.errors")
    prisma = ModuleType("prisma")
    engine = ModuleType("prisma.engine")
    prisma_error = _error_class("PrismaError", _FakeError, "prisma.errors")
    data_error = _error_class("DataError", prisma_error, "prisma.errors")
    classes = {
        "PrismaError": prisma_error,
        "DataError": data_error,
        "UniqueViolationError": _error_class("UniqueViolationError", data_error, "prisma.errors"),
        "ForeignKeyViolationError": _error_class(
            "ForeignKeyViolationError", data_error, "prisma.errors"
        ),
        "MissingRequiredValueError": _error_class(
            "MissingRequiredValueError", data_error, "prisma.errors"
        ),
        "RawQueryError": _error_class("RawQueryError", data_error, "prisma.errors"),
        "TableNotFoundError": _error_class("TableNotFoundError", data_error, "prisma.errors"),
        "RecordNotFoundError": _error_class("RecordNotFoundError", data_error, "prisma.errors"),
        "ClientNotConnectedError": _error_class(
            "ClientNotConnectedError", prisma_error, "prisma.errors"
        ),
        "HTTPClientClosedError": _error_class(
            "HTTPClientClosedError", prisma_error, "prisma.errors"
        ),
        "EngineConnectionError": _error_class(
            "EngineConnectionError", prisma_error, "prisma.engine.errors"
        ),
    }
    error_attributes = cast(dict[str, object], vars(errors))
    for name, value in classes.items():
        error_attributes[name] = value
    cast(dict[str, object], vars(engine_errors))["EngineConnectionError"] = classes[
        "EngineConnectionError"
    ]
    prisma_attributes = cast(dict[str, object], vars(prisma))
    prisma_attributes["errors"] = errors
    prisma_attributes["engine"] = engine
    cast(dict[str, object], vars(engine))["errors"] = engine_errors
    sys.modules.update(
        {
            "prisma": prisma,
            "prisma.errors": errors,
            "prisma.engine": engine,
            "prisma.engine.errors": engine_errors,
        }
    )
    return classes


_PRISMA_CLASSES = _install_prisma()
_PROXY_MODULE = ModuleType(_PROXY_SERVER)
cast(dict[str, object], vars(_PROXY_MODULE))["general_settings"] = _SETTINGS
sys.modules[_PROXY_SERVER] = _PROXY_MODULE

_ASYNC_PG = ModuleType("asyncpg")
_ASYNC_PG_EXCEPTIONS = ModuleType("asyncpg.exceptions")
_ASYNC_PG_CLASSES = {
    "postgres_connection": _error_class(
        "PostgresConnectionError", _FakeError, "asyncpg.exceptions"
    ),
    "interface": _error_class("InterfaceError", _FakeError, "asyncpg.exceptions"),
}
_ASYNC_PG_EXCEPTION_ATTRIBUTES = cast(dict[str, object], vars(_ASYNC_PG_EXCEPTIONS))
_ASYNC_PG_EXCEPTION_ATTRIBUTES["PostgresConnectionError"] = _ASYNC_PG_CLASSES["postgres_connection"]
_ASYNC_PG_EXCEPTION_ATTRIBUTES["InterfaceError"] = _ASYNC_PG_CLASSES["interface"]
cast(dict[str, object], vars(_ASYNC_PG))["exceptions"] = _ASYNC_PG_EXCEPTIONS

_TRANSPORT_TYPES: dict[str, type[Exception]] = {
    "connect": httpx.ConnectError,
    "connect_timeout": httpx.ConnectTimeout,
    "read": httpx.ReadError,
    "read_timeout": httpx.ReadTimeout,
}

_DATABASE_CLASSES: dict[str, str] = {
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


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _text_list(value: Json) -> list[str]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError("traceback_modules must be a list")
    return [_text(item, "traceback_modules entry") for item in cast(list[Json], value)]


def _index(value: Json, name: str) -> int | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def _traceback_error(modules: list[str], message: str) -> Exception:
    namespace: dict[str, object] = {"__name__": modules[-1], "message": message}
    function = types.FunctionType(
        compile("raise Exception(message)", "database_availability", "exec"), namespace
    )
    for name in reversed(modules[:-1]):
        outer_namespace: dict[str, object] = {"__name__": name, "inner": function}
        function = types.FunctionType(
            compile("inner()", "database_availability", "exec"), outer_namespace
        )
    try:
        function()
    except Exception as error:  # noqa: BLE001 - the fixture raises the generic engine error on purpose
        return error
    raise ValueError("the traceback fixture did not raise")


def _set_markers(error: _FakeError, node: dict[str, Json]) -> None:
    code = node.get("code")
    if isinstance(code, str):
        error.code = code
    meta_code = node.get("meta_code")
    if isinstance(meta_code, str):
        error.meta = {"code": meta_code}


def _database_error(node: dict[str, Json]) -> Exception:
    kind = _text(node.get("error_type"), "error_type")
    message = _text(node.get("message"), "message")
    transport = _TRANSPORT_TYPES.get(kind)
    if transport is not None:
        return transport(message)
    if kind == "deadline":
        return DBLookupDeadlineExceeded("database", 1.0)
    if kind == "no_db_connection":
        return ProxyException(message, ProxyErrorTypes.no_db_connection, None)
    class_name = _DATABASE_CLASSES.get(kind)
    if class_name is None:
        if kind != "other":
            raise ValueError(f"unknown database error type {kind}")
        modules = _text_list(node.get("traceback_modules"))
        return _traceback_error(modules, message) if modules else Exception(message)
    error = _PRISMA_CLASSES[class_name](message)
    _set_markers(error, node)
    return error


def _make_error(node: dict[str, Json]) -> BaseException:
    kind = _text(node.get("kind"), "kind")
    message = _text(node.get("message"), "message")
    if kind == "database":
        return _database_error(node)
    if kind == "asyncpg":
        fault = _text(node.get("fault"), "fault")
        return _ASYNC_PG_CLASSES[fault](message)
    constructors: dict[str, type[BaseException]] = {
        "os": OSError,
        "async_timeout": asyncio.TimeoutError,
        "other": Exception,
        "base_exception": BaseException,
    }
    constructor = constructors.get(kind)
    if constructor is None:
        raise ValueError(f"unknown exception kind {kind}")
    return constructor(message)


def _build_chain(chain_value: Json) -> list[BaseException]:
    if not isinstance(chain_value, list):
        raise ValueError("chain must be a list")
    nodes = [_object(item, "exception node") for item in cast(list[Json], chain_value)]
    errors = [_make_error(node) for node in nodes]
    for error, node in zip(errors, nodes, strict=True):
        cause = _index(node.get("cause"), "cause")
        if cause is not None:
            error.__cause__ = errors[cause]
        context = _index(node.get("context"), "context")
        if context is not None:
            error.__context__ = errors[context]
    if not errors:
        raise ValueError("chain must not be empty")
    return errors


def _asyncpg_available(value: Json) -> bool:
    if value == "available":
        return True
    if value is None or value == "unavailable":
        return False
    raise ValueError("asyncpg must be available or unavailable")


def _install_policy(scenario: dict[str, Json]) -> None:
    _SETTINGS.pop("allow_requests_on_db_unavailable", None)
    if "allow_requests" in scenario:
        _SETTINGS["allow_requests_on_db_unavailable"] = scenario["allow_requests"]


def _found_index(root: BaseException, links: list[BaseException]) -> int | None:
    found = PrismaDBExceptionHandler.find_database_service_unavailable_error_in_chain(root)
    if found is None:
        return None
    return next((index for index, link in enumerate(links) if link is found), None)


def _allow() -> tuple[bool | None, bool]:
    try:
        return PrismaDBExceptionHandler.should_allow_request_on_db_unavailable(), False
    except AttributeError:
        return None, True


def _handle(root: BaseException) -> str:
    try:
        PrismaDBExceptionHandler.handle_db_exception(cast(Exception, root))
    except BaseException as error:  # noqa: BLE001 - the rethrown original and the policy fault are the outcomes
        return "rethrown" if error is root else "policy_error"
    return "suppressed"


def _evaluate(errors: list[BaseException], *, available: bool) -> Json:
    saved = sys.modules.get("asyncpg")
    if available:
        sys.modules["asyncpg"] = _ASYNC_PG
    else:
        sys.modules.pop("asyncpg", None)
    try:
        root = errors[0]
        links = list(_exception_chain(root))
        accepted = [
            index
            for index, link in enumerate(links)
            if isinstance(link, Exception)
            and PrismaDBExceptionHandler.is_database_service_unavailable_error(link)
        ]
        found = _found_index(root, links)
        message = PrismaDBExceptionHandler.database_unavailable_message(cast(Exception, root))
        proxy = PrismaDBExceptionHandler.service_unavailable_proxy_exception(cast(Exception, root))
        allowed, policy_error = _allow()
        handled = _handle(root)
    finally:
        if saved is None:
            sys.modules.pop("asyncpg", None)
        else:
            sys.modules["asyncpg"] = saved
    return cast(
        Json,
        {
            "links": len(links),
            "accepted": accepted,
            "found": found,
            "permanent": message != _TRANSIENT_DB_UNAVAILABLE_MESSAGE,
            "error": {
                "status": int(proxy.code),
                "type": cast(ProxyErrorTypes, proxy.type).value,
                "param": proxy.param,
                "code": proxy.code,
                "message": proxy.message,
            },
            "allowed": allowed,
            "policy_error": policy_error,
            "handled": handled,
        },
    )


def _scenario(value: Json) -> Json:
    scenario = _object(value, "scenario")
    errors = _build_chain(scenario.get("chain"))
    _install_policy(scenario)
    return _evaluate(errors, available=_asyncpg_available(scenario.get("asyncpg")))


def run(input_value: dict[str, Json]) -> Json:
    scenarios = input_value.get("scenarios")
    if not isinstance(scenarios, list):
        raise ValueError("scenarios must be a list")
    return [_scenario(value) for value in cast(list[Json], scenarios)]
