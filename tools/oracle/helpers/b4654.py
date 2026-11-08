from collections.abc import Callable
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.proxy import proxy_server
from litellm.proxy._experimental.mcp_server import oauth_utils
from litellm.proxy._experimental.mcp_server.auth.user_api_key_auth_mcp import (
    MCPRequestHandler,
    _parse_mcp_server_names_from_path,  # pyright: ignore[reportPrivateUsage]
)
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from starlette.datastructures import Headers
from starlette.requests import Request
from starlette.types import Scope

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
HEADER_PAIR_LEN = 2


class _ServerFields:
    def __init__(self, upstream_resource: str | None, url: str | None, server_id: str) -> None:
        self.upstream_resource = upstream_resource
        self.url = url
        self.server_id = server_id


class _StubRequest:
    def __init__(self, base_url: str, headers: dict[str, str]) -> None:
        self.base_url = base_url
        self.headers = headers


def _string(data: dict[str, Json], name: str) -> str:
    value = data.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_string(data: dict[str, Json], name: str) -> str | None:
    value = data.get(name)
    if value is None or isinstance(value, str):
        return value
    raise ValueError(f"{name} must be a string or null")


def _boolean(data: dict[str, Json], name: str) -> bool:
    value = data.get(name)
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def _pair(entry: Json) -> tuple[bytes, bytes]:
    if not (
        isinstance(entry, list)
        and len(entry) == HEADER_PAIR_LEN
        and all(isinstance(part, str) for part in entry)
    ):
        raise ValueError("each header pair is a [name, value] list of strings")
    parts = cast(list[str], entry)
    return (parts[0].encode("latin-1"), parts[1].encode("latin-1"))


def _headers(data: dict[str, Json]) -> Headers:
    raw = data.get("raw")
    if not isinstance(raw, list):
        raise ValueError("raw must be a list of [name, value] pairs")
    scope: dict[str, object] = {"type": "http", "headers": [_pair(entry) for entry in raw]}
    return MCPRequestHandler._safe_get_headers_from_scope(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        cast(Scope, scope)
    )


def _scope(data: dict[str, Json], path_name: str) -> Scope:
    scope: dict[str, object] = {"type": "http", "path": _string(data, path_name)}
    for key in ("original_path", "app_root_path", "root_path"):
        value = _optional_string(data, key)
        if value is not None:
            scope["_original_path" if key == "original_path" else key] = value
    return cast(Scope, scope)


def _server(data: dict[str, Json]) -> MCPServer:
    fields = _ServerFields(
        _optional_string(data, "upstream_resource"),
        _optional_string(data, "url"),
        _string(data, "server_id"),
    )
    return cast(MCPServer, fields)


def _header_servers(data: dict[str, Json]) -> list[str] | None:
    raw = data.get("header_servers")
    if raw is None:
        return None
    if not isinstance(raw, list) or any(not isinstance(entry, str) for entry in raw):
        raise ValueError("header_servers must be a list of strings or null")
    return cast(list[str], raw)


def _strip_default_port(data: dict[str, Json]) -> object:
    netloc = oauth_utils._strip_default_port(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        _string(data, "scheme"), _string(data, "netloc")
    )
    return {"netloc": netloc}


def _proxy_base_url(data: dict[str, Json]) -> object:
    del data
    return {"value": oauth_utils._resolve_proxy_base_url_env()}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def _request_base_url(data: dict[str, Json]) -> object:
    headers: dict[str, str] = {}
    for name, key in (
        ("X-Forwarded-Proto", "x_forwarded_proto"),
        ("X-Forwarded-Host", "x_forwarded_host"),
        ("X-Forwarded-Port", "x_forwarded_port"),
    ):
        value = _optional_string(data, key)
        if value is not None:
            headers[name] = value
    request = cast(Request, _StubRequest(_string(data, "base_url"), headers))
    with patch.object(
        oauth_utils.IPAddressUtils,
        "is_request_from_trusted_proxy",
        return_value=_boolean(data, "from_trusted_proxy"),
    ):
        return {"base_url": oauth_utils.get_request_base_url(request)}


def _well_known_root_suffix(data: dict[str, Json]) -> object:
    del data
    return {"value": oauth_utils.well_known_root_suffix()}


def _route_relative_path(data: dict[str, Json]) -> object:
    return {"value": oauth_utils.get_route_relative_request_path(_scope(data, "path"))}


def _challenge(data: dict[str, Json]) -> object:
    base_url = _string(data, "base_url")
    invalid = data.get("invalid_token")
    with patch.object(
        oauth_utils,
        "get_request_base_url",
        lambda _request: base_url,  # pyright: ignore[reportUnknownLambdaType, reportUnknownArgumentType]
    ):
        scope = _scope(data, "route_path")
        name = _string(data, "server_name")
        if invalid is None:
            value = oauth_utils.get_passthrough_resource_metadata_url(scope, name)
        else:
            value = oauth_utils.get_passthrough_www_authenticate(
                scope, name, _boolean(data, "invalid_token")
            )
    return {"value": value}


def _byok_authenticate(data: dict[str, Json]) -> object:
    root_path = _string(data, "request_root_path")
    with patch.object(
        oauth_utils,
        "get_request_root_path",
        lambda: root_path,  # pyright: ignore[reportUnknownLambdaType, reportUnknownArgumentType]
    ):
        return {"value": oauth_utils.get_byok_www_authenticate()}


def _canonicalize(data: dict[str, Json]) -> object:
    url = _string(data, "url")
    if data.get("resource_only"):
        return {"value": oauth_utils.canonical_resource_uri(url)}
    return {"value": oauth_utils.canonicalize_url_identity(url)}


def _redact_url(data: dict[str, Json]) -> object:
    return {"value": oauth_utils._redact_mcp_resource_url(_optional_string(data, "url"))}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def _resolve_resource(data: dict[str, Json]) -> object:
    return {"value": oauth_utils.resolve_upstream_resource(_server(data))}


def _token_request(data: dict[str, Json]) -> object:
    built = oauth_utils.build_upstream_oauth2_token_request(
        _server(data),
        auth_method=_optional_string(data, "auth_method"),
        client_id=_optional_string(data, "client_id"),
        client_secret=_optional_string(data, "client_secret"),
    )
    return {"headers": built.headers, "body": built.body}


def _safe_headers(data: dict[str, Json]) -> object:
    headers = _headers(data)
    raw_lookups = data.get("lookups") or []
    if not isinstance(raw_lookups, list):
        raise ValueError("lookups must be a list of strings")
    lookups: dict[str, Json] = {}
    for entry in raw_lookups:
        if not isinstance(entry, str):
            raise ValueError("every lookup is a string")
        lookups[entry] = headers.get(entry)
    return {"items": [[name, value] for name, value in headers.items()], "lookups": lookups}


def _litellm_key(data: dict[str, Json]) -> object:
    headers = _headers(data)
    present = headers.get(MCPRequestHandler.LITELLM_API_KEY_HEADER_NAME_PRIMARY) is not None
    value = MCPRequestHandler.get_litellm_api_key_from_headers(headers)
    return {"value": value, "explicit_header_present": present}


def _mcp_auth_header(data: dict[str, Json]) -> object:
    general = _optional_string(data, "general_settings")
    settings: dict[str, str] = {}
    if general is not None:
        settings["mcp_client_side_auth_header_name"] = general
    headers = _headers(data)
    with patch.object(proxy_server, "general_settings", settings):
        header_name = MCPRequestHandler.get_mcp_client_side_auth_header_name()
        value = MCPRequestHandler._get_mcp_auth_header_from_headers(headers)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {"header_name": header_name, "value": value}


def _server_auth(data: dict[str, Json]) -> object:
    return {"value": MCPRequestHandler._get_mcp_server_auth_headers_from_headers(_headers(data))}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def _oauth2(data: dict[str, Json]) -> object:
    return {"value": MCPRequestHandler._get_oauth2_headers_from_headers(_headers(data))}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def _list_header(data: dict[str, Json]) -> object:
    headers = _headers(data)
    if data.get("header") == "access_groups":
        return {"value": MCPRequestHandler.get_mcp_access_groups_from_headers(headers)}
    header = headers.get(MCPRequestHandler.LITELLM_MCP_SERVERS_HEADER_NAME)
    if header is None:
        return {"value": None}
    return {"value": [s.strip() for s in header.split(",") if s.strip()]}


def _target_names(data: dict[str, Json]) -> object:
    raw_paths = data.get("paths")
    if not isinstance(raw_paths, list):
        raise ValueError("paths must be a list of strings")
    values: list[list[str]] = []
    for entry in raw_paths:
        if not isinstance(entry, str):
            raise ValueError("every path is a string")
        values.append(MCPRequestHandler.extract_target_server_names_from_path(entry))
    return {"values": values}


def _target_selection(data: dict[str, Json]) -> object:
    path = _string(data, "path")
    servers = _header_servers(data)
    if data.get("mode") == "cold_start":
        value = _parse_mcp_server_names_from_path(path, servers)  # pyright: ignore[reportPrivateUsage]
    else:
        value = MCPRequestHandler._resolve_target_server_names(path, servers)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {"value": value}


_HANDLERS: dict[str, Callable[[dict[str, Json]], object]] = {
    "strip_default_port": _strip_default_port,
    "proxy_base_url": _proxy_base_url,
    "request_base_url": _request_base_url,
    "well_known_root_suffix": _well_known_root_suffix,
    "route_relative_path": _route_relative_path,
    "metadata_url": _challenge,
    "www_authenticate": _challenge,
    "byok_authenticate": _byok_authenticate,
    "canonicalize": _canonicalize,
    "redact_url": _redact_url,
    "resolve_resource": _resolve_resource,
    "token_request": _token_request,
    "safe_headers": _safe_headers,
    "litellm_key": _litellm_key,
    "mcp_auth_header": _mcp_auth_header,
    "server_auth": _server_auth,
    "oauth2": _oauth2,
    "list_header": _list_header,
    "target_names": _target_names,
    "target_selection": _target_selection,
}


def run(data: dict[str, Json]) -> object:
    function = data.get("function")
    if not isinstance(function, str):
        raise ValueError("every b4654 call names a function")
    handler = _HANDLERS.get(function)
    if handler is None:
        raise ValueError(f"unknown b4654 function: {function}")
    return handler(data)
