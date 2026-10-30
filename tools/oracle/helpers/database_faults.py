import sys
import types
from typing import TypeAlias, cast

import httpx
from litellm.proxy._types import ProxyErrorTypes, ProxyException
from litellm.proxy.db.db_lookup_gate import DBLookupDeadlineExceeded
from litellm.proxy.db.exception_handler import PrismaDBExceptionHandler

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
    sys.modules.update({
        "prisma": prisma,
        "prisma.errors": errors,
        "prisma.engine": engine,
        "prisma.engine.errors": engine_errors,
    })
    return names


def _make_error(error: dict[str, Json]) -> Exception:
    types_by_name = _prisma_types()
    kind = cast(str, error["error_type"])
    message = cast(str, error.get("message", ""))
    if kind == "connection":
        return httpx.ConnectError(message)
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


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    error = cast(dict[str, Json], input_value["error"])
    value = _make_error(error)
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
