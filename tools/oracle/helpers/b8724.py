from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.proxy._experimental.mcp_server import server
from litellm.proxy._types import UserAPIKeyAuth
from pydantic import JsonValue, TypeAdapter
from starlette.types import Scope

JSON: Final = TypeAdapter(JsonValue)


class PathQuery(TypedDict):
    operation: Literal["server_in_path"]
    paths: list[str]


class FingerprintCall(TypedDict):
    api_key: NotRequired[str]
    user_id: NotRequired[str]
    oauth2_headers: NotRequired[list[tuple[str, str]]]
    client_ip: NotRequired[str]


class FingerprintQuery(TypedDict):
    operation: Literal["owner_fingerprint"]
    calls: list[FingerprintCall]


class InitializeDetectionQuery(TypedDict):
    operation: Literal["initialize_detection"]
    bodies: list[str]


class ClientInfoQuery(TypedDict):
    operation: Literal["client_info"]
    bodies: list[str]


class TopLevelMethodQuery(TypedDict):
    operation: Literal["top_level_method_scan"]
    texts: list[str]


class ForwardedAuthorizationQuery(TypedDict):
    operation: Literal["forwarded_authorization"]
    headers: list[list[tuple[str, str]]]


Input: TypeAlias = (
    PathQuery
    | FingerprintQuery
    | InitializeDetectionQuery
    | ClientInfoQuery
    | TopLevelMethodQuery
    | ForwardedAuthorizationQuery
)


def header_scope(pairs: list[tuple[str, str]]) -> Scope:
    headers: list[tuple[bytes, bytes]] = [
        (name.encode("latin-1"), value.encode("latin-1")) for name, value in pairs
    ]
    scope: Scope = {"type": "http", "path": "/mcp", "headers": headers}
    return scope


def fingerprint_answer(call: FingerprintCall) -> str:
    auth = UserAPIKeyAuth(api_key=call.get("api_key"), user_id=call.get("user_id"))
    configured = call.get("oauth2_headers")
    oauth2_headers: dict[str, str] | None = None
    if configured is not None:
        oauth2_headers = dict(configured)
    return server._owner_fingerprint_for(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/server.py#L1185
        auth,
        oauth2_headers,
        call.get("client_ip"),
    )


def client_info_answer(body: str) -> JsonValue:
    client = server._extract_initialize_client_info(body.encode("utf-8"))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/server.py#L1254
    if client is None:
        return None
    return {"name": client.name, "version": client.version}


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    match input_value["operation"]:
        case "server_in_path":
            return JSON.validate_python(
                [  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/server.py#L1074
                    server._get_mcp_servers_in_path(path)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/server.py#L1074
                    for path in input_value["paths"]
                ]
            )
        case "owner_fingerprint":
            return JSON.validate_python(
                [fingerprint_answer(call) for call in input_value["calls"]]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/server.py#L1185
            )
        case "initialize_detection":
            return JSON.validate_python(
                [  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/server.py#L1241
                    server._is_initialize_request(body.encode("utf-8"))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/server.py#L1241
                    for body in input_value["bodies"]
                ]
            )
        case "client_info":
            return JSON.validate_python(
                [client_info_answer(body) for body in input_value["bodies"]]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/server.py#L1254
            )
        case "top_level_method_scan":
            return JSON.validate_python(
                [  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/server.py#L194
                    server._jsonrpc_text_has_top_level_method(text)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/server.py#L194
                    for text in input_value["texts"]
                ]
            )
        case "forwarded_authorization":
            return JSON.validate_python(
                [  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/server.py#L1837
                    server._get_forwarded_auth_from_scope(header_scope(pairs))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/server.py#L1837
                    for pairs in input_value["headers"]
                ]
            )
