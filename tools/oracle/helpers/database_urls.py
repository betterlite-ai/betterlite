import os
from collections.abc import Callable
from typing import TypeAlias, cast
from unittest import mock

from litellm.proxy.db.db_url_settings import (
    DatabaseURLSettings,
    unsupported_db_scheme,
    unsupported_db_scheme_message,
)
from litellm.proxy.utils import construct_database_url_from_env_vars

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def optional_text(value: Json) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError("the setting must be a string")
    return value


def text_with_default(value: Json, default: str) -> str:
    text = optional_text(value)
    return default if text is None else text


def optional_integer(value: Json) -> int | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError("the setting must be an integer")
    return value


def flag_value(value: Json) -> bool:
    if value is None:
        return False
    if not isinstance(value, bool):
        raise ValueError("the setting must be a boolean")
    return value


def settings_from(value: dict[str, Json]) -> DatabaseURLSettings:
    return DatabaseURLSettings(
        database_url=optional_text(value.get("database_url")),
        direct_url=optional_text(value.get("direct_url")),
        database_host=optional_text(value.get("database_host")),
        database_port=text_with_default(value.get("database_port"), "5432"),
        database_user=optional_text(value.get("database_user")),
        database_name=optional_text(value.get("database_name")),
        database_schema=optional_text(value.get("database_schema")),
        database_password=optional_text(value.get("database_password")),
        database_url_read_replica=optional_text(value.get("database_url_read_replica")),
        database_host_read_replica=optional_text(value.get("database_host_read_replica")),
        database_port_read_replica=optional_text(value.get("database_port_read_replica")),
        database_user_read_replica=optional_text(value.get("database_user_read_replica")),
        database_name_read_replica=optional_text(value.get("database_name_read_replica")),
        database_schema_read_replica=optional_text(value.get("database_schema_read_replica")),
        database_password_read_replica=optional_text(value.get("database_password_read_replica")),
        database_sslmode=optional_text(value.get("database_sslmode")),
        database_sslrootcert=optional_text(value.get("database_sslrootcert")),
        max_idle_connection_lifetime=optional_integer(value.get("max_idle_connection_lifetime")),
        disable_prepared_statements=flag_value(value.get("disable_prepared_statements")),
        iam_token_db_auth=flag_value(value.get("iam_token_db_auth")),
        azure_postgresql_auth=flag_value(value.get("azure_postgresql_auth")),
    )


def settings_argument(input_value: dict[str, Json]) -> DatabaseURLSettings:
    return settings_from(object_value(input_value["settings"], "settings"))


def require_password_auth(input_value: dict[str, Json]) -> None:
    auth = input_value.get("auth", "password")
    if auth != "password":
        raise ValueError("the oracle helper covers the password branch only")


def environment_values(value: dict[str, Json]) -> dict[str, str]:
    return {name: text_value(item, name) for name, item in value.items()}


def text_value(item: Json, name: str) -> str:
    if not isinstance(item, str):
        raise ValueError(f"environment value for {name} must be a string")
    return item


def settings_summary(settings: DatabaseURLSettings) -> Json:
    reader_password_set = settings.database_password_read_replica is not None
    return cast(
        Json,
        {
            "database_url": settings.database_url,
            "direct_url": settings.direct_url,
            "database_host": settings.database_host,
            "database_port": settings.database_port,
            "database_user": settings.database_user,
            "database_name": settings.database_name,
            "database_schema": settings.database_schema,
            "database_password_set": settings.database_password is not None,
            "database_url_read_replica": settings.database_url_read_replica,
            "database_host_read_replica": settings.database_host_read_replica,
            "database_port_read_replica": settings.database_port_read_replica,
            "database_user_read_replica": settings.database_user_read_replica,
            "database_name_read_replica": settings.database_name_read_replica,
            "database_schema_read_replica": settings.database_schema_read_replica,
            "database_password_read_replica_set": reader_password_set,
            "database_sslmode": settings.database_sslmode,
            "database_sslrootcert": settings.database_sslrootcert,
            "max_idle_connection_lifetime": settings.max_idle_connection_lifetime,
            "disable_prepared_statements": settings.disable_prepared_statements,
            "iam_token_db_auth": settings.iam_token_db_auth,
            "azure_postgresql_auth": settings.azure_postgresql_auth,
        },
    )


def run_build_writer_url(input_value: dict[str, Json]) -> Json:
    require_password_auth(input_value)
    return settings_argument(input_value).build_writer_url()


def run_build_reader_url(input_value: dict[str, Json]) -> Json:
    require_password_auth(input_value)
    return settings_argument(input_value).build_reader_url()


def run_tls_params(input_value: dict[str, Json]) -> Json:
    settings = settings_argument(input_value)
    return cast(Json, [[key, value] for key, value in settings.tls_params().items()])


def run_from_env(input_value: dict[str, Json]) -> Json:
    environment = environment_values(object_value(input_value["environment"], "environment"))
    with mock.patch.dict(os.environ, environment, clear=True):
        settings = DatabaseURLSettings.from_env()
    return settings_summary(settings)


def run_validate_schemes(input_value: dict[str, Json]) -> Json:
    settings_argument(input_value)._raise_for_unsupported_scheme()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return None


def run_validate_cli_urls(input_value: dict[str, Json]) -> Json:
    environment = environment_values(object_value(input_value["environment"], "environment"))
    with mock.patch.dict(os.environ, environment, clear=True):
        for name in ("DATABASE_URL", "DIRECT_URL"):
            url = os.getenv(name)
            if url is None:
                continue
            bad_scheme = unsupported_db_scheme(url)
            if bad_scheme is not None:
                raise RuntimeError(unsupported_db_scheme_message(name, bad_scheme))
    return None


def run_apply_writer_url(input_value: dict[str, Json]) -> Json:
    settings = settings_argument(input_value)
    with mock.patch.dict(os.environ, {}, clear=True):
        replaced = settings.apply_writer_url_to_env()
        written = os.environ.get("DATABASE_URL")
    updates: list[Json] = []
    if replaced and written is not None:
        updates.append(cast(Json, ["DATABASE_URL", written]))
    return cast(Json, {"replaced": replaced, "updates": updates})


def run_construct_cli_url(input_value: dict[str, Json]) -> Json:
    environment = environment_values(object_value(input_value["environment"], "environment"))
    with mock.patch.dict(os.environ, environment, clear=True):
        return construct_database_url_from_env_vars()


def run_batch(input_value: dict[str, Json]) -> Json:
    return [scenario(item) for item in list_value(input_value["calls"], "calls")]


def scenario(value: Json) -> Json:
    try:
        return cast(Json, {"ok": True, "value": run(object_value(value, "call"))})
    except Exception:  # noqa: BLE001 - every failure is the compared outcome of one scenario
        return cast(Json, {"ok": False})


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "apply_writer_url": run_apply_writer_url,
    "batch": run_batch,
    "build_reader_url": run_build_reader_url,
    "build_writer_url": run_build_writer_url,
    "construct_cli_url": run_construct_cli_url,
    "from_env": run_from_env,
    "tls_params": run_tls_params,
    "validate_cli_urls": run_validate_cli_urls,
    "validate_schemes": run_validate_schemes,
}


def run(input_value: dict[str, Json]) -> Json:
    function = input_value["function"]
    handler = HANDLERS.get(cast(str, function))
    if handler is None:
        raise ValueError(f"unknown database_urls function: {function}")
    return handler(input_value)
