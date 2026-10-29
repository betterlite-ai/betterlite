import logging
from collections.abc import Callable
from typing import TypeAlias, cast

from litellm._logging import verbose_proxy_logger
from litellm.proxy.db.db_url_settings import (
    add_missing_query_params,
    connection_params_from_url,
    idle_lifetime_params,
    reader_shareable_params,
    token_refresh_params_from_url,
    unsupported_db_scheme,
    unsupported_db_scheme_message,
)
from litellm.proxy.db.token_auth import token_auth_flag_enabled
from litellm.proxy.proxy_cli import (
    DatabaseTimeoutSettings,
    _build_db_connection_url_params,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    _pg_options_with_timeouts,  # pyright: ignore[reportPrivateUsage]
    _url_query_value,  # pyright: ignore[reportPrivateUsage]
    _with_query_value,  # pyright: ignore[reportPrivateUsage]
    append_query_params,  # pyright: ignore[reportUnknownVariableType]
)
from litellm_proxy_extras.utils import str_to_bool

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def pairs_to_dict(pairs: Json) -> dict[str, str | int | float]:
    result: dict[str, str | int | float] = {}
    for pair in cast(list[Json], pairs):
        key, value = cast(list[Json], pair)
        result[cast(str, key)] = cast("str | int | float", value)
    return result


def dict_to_pairs(params: dict[str, str | int | float]) -> Json:
    return [[key, value] for key, value in params.items()]


def run_idle_lifetime_params(input_value: dict[str, Json]) -> Json:
    configured = cast("int | float | None", input_value["configured"])
    return dict_to_pairs(dict(idle_lifetime_params(configured)))


def run_add_missing_query_params(input_value: dict[str, Json]) -> Json:
    url = cast(str, input_value["url"])
    params = pairs_to_dict(input_value["params"])
    return add_missing_query_params(url, params)


def run_reader_shareable_params(input_value: dict[str, Json]) -> Json:
    params = pairs_to_dict(input_value["params"])
    return dict_to_pairs(dict(reader_shareable_params(params)))


def run_connection_params_from_url(input_value: dict[str, Json]) -> Json:
    url = cast(str, input_value["url"])
    return dict_to_pairs(dict(connection_params_from_url(url)))


def run_token_refresh_params_from_url(input_value: dict[str, Json]) -> Json:
    url = cast(str, input_value["url"])
    return dict_to_pairs(dict(token_refresh_params_from_url(url)))


def run_unsupported_db_scheme(input_value: dict[str, Json]) -> Json:
    database_url = cast(str, input_value["database_url"])
    return unsupported_db_scheme(database_url)


def run_unsupported_db_scheme_message(input_value: dict[str, Json]) -> Json:
    env_var = cast(str, input_value["env_var"])
    scheme = cast(str, input_value["scheme"])
    return unsupported_db_scheme_message(env_var, scheme)


def run_build_db_connection_url_params(input_value: dict[str, Json]) -> Json:
    connection_limit = cast(int, input_value["connection_limit"])
    pool_timeout = cast("float | None", input_value["pool_timeout"])
    connect_timeout = cast("float | None", input_value["connect_timeout"])
    socket_timeout = cast("float | None", input_value["socket_timeout"])
    disable_prepared_statements = cast(bool, input_value["disable_prepared_statements"])
    extra_params = pairs_to_dict(input_value["extra_params"])
    params = cast(
        "dict[str, str | int | float]",
        _build_db_connection_url_params(
            connection_limit,
            pool_timeout,
            connect_timeout,
            socket_timeout,
            disable_prepared_statements,
            extra_params,
        ),
    )
    return dict_to_pairs(params)


def run_pg_options_with_timeouts(input_value: dict[str, Json]) -> Json:
    existing_options = cast(str, input_value["existing_options"])
    settings = DatabaseTimeoutSettings(
        database_statement_timeout=cast("float | None", input_value["statement_timeout"]),
        database_lock_timeout=cast("float | None", input_value["lock_timeout"]),
    )
    return _pg_options_with_timeouts(
        existing_options,
        settings.database_statement_timeout,
        settings.database_lock_timeout,
    )


def run_url_query_value(input_value: dict[str, Json]) -> Json:
    url = cast("str | None", input_value["url"])
    key = cast(str, input_value["key"])
    return _url_query_value(url, key)


def run_with_query_value(input_value: dict[str, Json]) -> Json:
    url = cast(str, input_value["url"])
    key = cast(str, input_value["key"])
    value = cast(str, input_value["value"])
    return _with_query_value(url, key, value)


class _WarningCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.messages: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def run_append_query_params(input_value: dict[str, Json]) -> Json:
    url = cast("str | None", input_value["url"])
    params: dict[str, str | int | float | list[str | int | float]] = {}
    for pair in cast(list[Json], input_value["params"]):
        key, value = cast(list[Json], pair)
        params[cast(str, key)] = cast("str | int | float | list[str | int | float]", value)
    capture = _WarningCapture()
    capture.setLevel(logging.WARNING)
    previous_level = verbose_proxy_logger.level
    verbose_proxy_logger.addHandler(capture)
    verbose_proxy_logger.setLevel(logging.WARNING)
    try:
        modified_url = append_query_params(url, params)
    finally:
        verbose_proxy_logger.removeHandler(capture)
        verbose_proxy_logger.setLevel(previous_level)
    warning = capture.messages[0] if capture.messages else None
    return {"url": modified_url, "warning": warning}


def run_token_auth_flag_enabled(input_value: dict[str, Json]) -> Json:
    value = cast("str | bool | None", input_value["value"])
    env_var = cast(str, input_value["env_var"])
    return token_auth_flag_enabled(value, env_var=env_var)


def run_str_to_bool(input_value: dict[str, Json]) -> Json:
    value = cast("str | None", input_value["value"])
    return str_to_bool(value)


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "idle_lifetime_params": run_idle_lifetime_params,
    "add_missing_query_params": run_add_missing_query_params,
    "reader_shareable_params": run_reader_shareable_params,
    "connection_params_from_url": run_connection_params_from_url,
    "token_refresh_params_from_url": run_token_refresh_params_from_url,
    "unsupported_db_scheme": run_unsupported_db_scheme,
    "unsupported_db_scheme_message": run_unsupported_db_scheme_message,
    "build_db_connection_url_params": run_build_db_connection_url_params,
    "pg_options_with_timeouts": run_pg_options_with_timeouts,
    "url_query_value": run_url_query_value,
    "with_query_value": run_with_query_value,
    "append_query_params": run_append_query_params,
    "token_auth_flag_enabled": run_token_auth_flag_enabled,
    "str_to_bool": run_str_to_bool,
}


def run(input_value: dict[str, Json]) -> Json:
    function = input_value["function"]
    handler = HANDLERS.get(cast(str, function))
    if handler is None:
        raise ValueError(f"unknown database_url_policy function: {function}")
    return handler(input_value)
