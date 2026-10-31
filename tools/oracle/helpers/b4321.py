import os
from typing import TypeAlias, cast
from unittest import mock

from litellm._redis import (
    _get_redis_client_logic,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    _uses_tls,  # pyright: ignore[reportPrivateUsage]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_OWN_FIELDS = (
    "host",
    "port",
    "db",
    "username",
    "password",
    "url",
    "ssl",
    "socket_timeout",
    "socket_connect_timeout",
    "socket_keepalive",
    "max_connections",
    "health_check_interval",
    "credential_provider",
    "startup_nodes",
    "sentinel_nodes",
    "sentinel_password",
    "service_name",
    "ssl_password",
)


def _kwargs_from_overrides(overrides: dict[str, Json]) -> dict[str, object]:
    kwargs: dict[str, object] = {
        name: overrides[name] for name in _OWN_FIELDS if overrides.get(name) is not None
    }
    # litellm distinguishes an explicit `url: null` from an omitted url: the key stays in
    # **env_overrides and overrides whatever REDIS_URL supplied, where an omitted key does not.
    if "url" in overrides and overrides["url"] is None:
        kwargs["url"] = None
    if "password" in overrides and overrides["password"] is None:
        kwargs["password"] = None
    if kwargs.get("credential_provider") is True:
        kwargs["credential_provider"] = object()
    passthrough = overrides.get("passthrough") or {}
    if not isinstance(passthrough, dict):
        raise ValueError("passthrough must be an object")
    kwargs.update({name: value for name, value in passthrough.items() if value is not None})
    kwargs.update(
        {
            name: value
            for name, value in overrides.items()
            if name not in _OWN_FIELDS and name != "passthrough" and value is not None
        }
    )
    return kwargs


def resolve_redis_kwargs(environment: dict[str, Json], overrides: dict[str, Json]) -> Json:
    variables = {name: value for name, value in environment.items() if isinstance(value, str)}
    if len(variables) != len(environment):
        raise ValueError("environment values must be strings")
    kwargs = _kwargs_from_overrides(overrides)
    with mock.patch.dict(os.environ, variables, clear=True):
        result = _get_redis_client_logic(**kwargs)
    known = {*_OWN_FIELDS, "redis_connect_func"}
    payload: dict[str, object] = {
        "host": result.get("host"),
        "port": result.get("port"),
        "db": result.get("db"),
        "username": result.get("username"),
        "password": result.get("password"),
        "url": result.get("url"),
        "ssl": result.get("ssl"),
        "socket_timeout": result.get("socket_timeout"),
        "socket_connect_timeout": result.get("socket_connect_timeout"),
        "socket_keepalive": result.get("socket_keepalive"),
        "max_connections": result.get("max_connections"),
        "health_check_interval": result.get("health_check_interval"),
        "credential_provider": result.get("credential_provider") is not None,
        "startup_nodes": result.get("startup_nodes"),
        "sentinel_nodes": result.get("sentinel_nodes"),
        "sentinel_password": result.get("sentinel_password"),
        "service_name": result.get("service_name"),
        "ssl_password": result.get("ssl_password"),
        "passthrough": {name: value for name, value in result.items() if name not in known},
        "uses_tls": _uses_tls(result),
    }
    return cast(Json, payload)


def run(data: dict[str, Json]) -> Json:
    match data:
        case {"function": "resolve_redis_kwargs", "overrides": dict(overrides)}:
            environment = data.get("environment") or {}
            if not isinstance(environment, dict):
                raise ValueError("environment must be an object")
            return resolve_redis_kwargs(environment, overrides)
        case _:
            raise ValueError("unknown redis connection settings function")
