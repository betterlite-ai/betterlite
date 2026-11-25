import asyncio
import importlib
import json
import ssl
from collections.abc import Awaitable, Callable, Iterable
from functools import partial
from inspect import Signature, signature
from typing import Final, Protocol, TypedDict

import httpx2
import pytest
from litellm.experimental_mcp_client.client import MCPClient as LiteLLMClient
from mcp import MCPError as TheirProtocolError
from mcp.types import CallToolRequestParams as TheirCallToolRequestParams
from mcp.types import GetPromptRequestParams as TheirGetPromptRequestParams
from mcp_server import DENIED_CODE, REFUSE, SLOW, UNAUTHORIZED_MODE, Server
from pydantic import AnyUrl, JsonValue, TypeAdapter

from betterlite._native import NativeMcpError
from betterlite.experimental_mcp_client import client as client_module
from betterlite.experimental_mcp_client.client import MCPClient
from betterlite.experimental_mcp_client.errors import (
    ERROR_TYPES,
    McpError,
    UnsupportedMcpArgument,
    raise_native_error,
)
from betterlite.experimental_mcp_client.types import (
    CallToolRequestParams,
    GetPromptRequestParams,
)
from betterlite.main import UnknownNativeError
from betterlite.types.mcp import MCPAuth, MCPUpstreamProtocol

TIMEOUT_VARIABLE: Final = "LITELLM_MCP_CLIENT_TIMEOUT"
DEFAULT_TIMEOUT: Final = 60.0
CONFIGURED_TIMEOUT: Final = "0.5"
CONFIGURED_SECONDS: Final = 0.5
EXPLICIT_TIMEOUT: Final = 7.0
BLANK_TIMEOUT: Final = " "
INVALID_TIMEOUT: Final = "fast"
SHORT_TIMEOUT: Final = 0.2
CA_BUNDLE: Final = "/etc/ssl/certs/ca.pem"
PLACEHOLDER_URL: Final = "http://127.0.0.1:1/mcp"
UNAUTHORIZED_STATUS: Final = 401
TOOL_NAME: Final = "search"
PROMPT_NAME: Final = "greet"
RESOURCE_URL: Final = "file:///notes.txt"
BEARER: Final = "tok-1"
UPDATED_BEARER: Final = "tok-2"
BASIC_VALUE: Final = "user:pass"
BASIC_HEADER: Final = "Basic dXNlcjpwYXNz"
HEADER_MAP: Final[dict[str, str]] = {"X-Api-Key": "k"}
DENIED_MESSAGE: Final = "tools/call denied by policy"
ERROR_MESSAGE: Final = "the server denied the call"
FINGERPRINT_LENGTH: Final = 64
REFUSED: Final = "refused"
MCP_LEGACY_VERSION: Final[MCPUpstreamProtocol] = "2025-06-18"
TOOL_ARGUMENTS: Final[dict[str, JsonValue]] = {"q": "x"}
EXPECTED_AUTH_HEADERS: Final[dict[str, JsonValue]] = {
    "bearer": f"Bearer {BEARER}",
    "basic": BASIC_HEADER,
    "map": HEADER_MAP["X-Api-Key"],
    "updated": f"Bearer {UPDATED_BEARER}",
}
STATUS_DATA: Final[dict[str, JsonValue]] = {
    "status": UNAUTHORIZED_STATUS,
    "body": "denied",
    "url": PLACEHOLDER_URL,
    "headers": [["WWW-Authenticate", 'Bearer realm="mcp"']],
}
FAILURE_MODES: Final[tuple[tuple[str, str, str | None], ...]] = (
    (UNAUTHORIZED_MODE, "HTTPStatusError: ", "HTTPStatusError: "),
    (SLOW, "TimeoutError: ", "TimeoutError: "),
)

class TextBlock(TypedDict):
    type: str
    text: str


class TextBlockResult(TypedDict):
    content: list[TextBlock]
    isError: bool


RESULT: Final[TypeAdapter[TextBlockResult]] = TypeAdapter(TextBlockResult)


class Dumpable(Protocol):
    def model_dump(
        self, *, by_alias: bool = False, exclude_none: bool = False
    ) -> dict[str, JsonValue]: ...


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


class AuthClient(Protocol):
    def list_tools(self) -> Awaitable[Iterable[Dumpable]]: ...
    def update_auth_value(self, mcp_auth_value: str | dict[str, str]) -> None: ...


class ResourceClient(Protocol):
    def list_resources(self) -> Awaitable[Iterable[Dumpable]]: ...
    def list_resource_templates(self) -> Awaitable[Iterable[Dumpable]]: ...
    def read_resource(self, url: AnyUrl) -> Awaitable[Dumpable]: ...


class MemberMethod(Protocol):
    def __call__(self, *args: JsonValue, **kwargs: JsonValue) -> Dumpable: ...


class ClientClass(Protocol):
    @staticmethod
    def error_tool_result(exc: Exception) -> Dumpable: ...
    def __call__(
        self,
        *,
        server_url: str = "",
        auth_type: str | None = None,
        auth_value: str | dict[str, str] | None = None,
    ) -> AuthClient: ...
    async def call_tool(
        self, call_tool_request_params: TheirCallToolRequestParams
    ) -> Dumpable: ...
    def __getattr__(self, name: str) -> MemberMethod: ...


class ClientModule(Protocol):
    def __getattr__(self, name: str) -> ClientClass: ...


class McpErrorClasses(Protocol):
    def __getattr__(self, name: str) -> type[TheirProtocolError]: ...


LITELLM_CLIENT: ClientModule = importlib.import_module("litellm.experimental_mcp_client.client")
MCP_ERRORS: McpErrorClasses = importlib.import_module("mcp")

MEMBERS: Final[tuple[tuple[str, Signature, Signature], ...]] = (
    ("MCPClient", signature(MCPClient), signature(LITELLM_CLIENT.MCPClient)),
    ("list_tools", signature(MCPClient.list_tools), signature(LITELLM_CLIENT.MCPClient.list_tools)),
    ("call_tool", signature(MCPClient.call_tool), signature(LITELLM_CLIENT.MCPClient.call_tool)),
    (
        "list_prompts",
        signature(MCPClient.list_prompts),
        signature(LITELLM_CLIENT.MCPClient.list_prompts),
    ),
    (
        "get_prompt",
        signature(MCPClient.get_prompt),
        signature(LITELLM_CLIENT.MCPClient.get_prompt),
    ),
    (
        "list_resources",
        signature(MCPClient.list_resources),
        signature(LITELLM_CLIENT.MCPClient.list_resources),
    ),
    (
        "list_resource_templates",
        signature(MCPClient.list_resource_templates),
        signature(LITELLM_CLIENT.MCPClient.list_resource_templates),
    ),
    (
        "read_resource",
        signature(MCPClient.read_resource),
        signature(LITELLM_CLIENT.MCPClient.read_resource),
    ),
    (
        "discovery_auth_fingerprint",
        signature(MCPClient.discovery_auth_fingerprint),
        signature(LITELLM_CLIENT.MCPClient.discovery_auth_fingerprint),
    ),
    (
        "error_tool_result",
        signature(MCPClient.error_tool_result),
        signature(LITELLM_CLIENT.MCPClient.error_tool_result),
    ),
    (
        "update_auth_value",
        signature(MCPClient.update_auth_value),
        signature(LITELLM_CLIENT.MCPClient.update_auth_value),
    ),
    (
        "run_with_session",
        signature(MCPClient.run_with_session),
        signature(LITELLM_CLIENT.MCPClient.run_with_session),
    ),
)


# ast-grep-ignore: no-one-line-functions
def dumped(value: Dumpable) -> dict[str, JsonValue]:
    return value.model_dump(by_alias=True, exclude_none=True)


def error_text(fields: dict[str, JsonValue]) -> str:
    result = RESULT.validate_python(fields)
    return result["content"][0]["text"]


def parameters_of(declaration: Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in declaration.parameters.values():
        default: DefaultView = parameter
        rows.append((parameter.name, str(parameter.kind), repr(default.default)))
    return rows


def refusal_of(build: Callable[[], MCPClient]) -> str:
    try:
        build()
    except UnsupportedMcpArgument as failure:
        return failure.argument
    raise AssertionError("the refused argument was accepted")


def native_payload(kind: str) -> str:
    data: JsonValue = STATUS_DATA if kind == "HTTPStatusError" else None
    payload: dict[str, JsonValue] = {
        "type": kind,
        "message": ERROR_MESSAGE,
        "code": DENIED_CODE,
        "data": data,
    }
    return json.dumps(payload)


async def progress_refused() -> str:
    client = MCPClient(server_url=PLACEHOLDER_URL)
    params = CallToolRequestParams(name=TOOL_NAME)
    try:
        await client.call_tool(params, host_progress_callback=REFUSED)
    except UnsupportedMcpArgument as failure:
        return failure.argument
    raise AssertionError("the refused argument was accepted")


async def our_prompts(server: Server) -> dict[str, JsonValue]:
    client = MCPClient(server_url=server.url)
    params = GetPromptRequestParams(name=PROMPT_NAME)
    prompts: list[JsonValue] = [dumped(prompt) for prompt in await client.list_prompts()]
    prompt = dumped(await client.get_prompt(params))
    return {"prompts": prompts, "prompt": prompt}


async def their_prompts(server: Server) -> dict[str, JsonValue]:
    client = LiteLLMClient(server_url=server.url)
    params = TheirGetPromptRequestParams(name=PROMPT_NAME)
    prompts: list[JsonValue] = [dumped(prompt) for prompt in await client.list_prompts()]
    prompt = dumped(await client.get_prompt(params))
    return {"prompts": prompts, "prompt": prompt}


async def both_tool_lists(
    server: Server, client_timeout: float | None = None
) -> tuple[list[dict[str, JsonValue]], list[dict[str, JsonValue]]]:
    ours = await MCPClient(server_url=server.url, timeout=client_timeout).list_tools()
    theirs = await LiteLLMClient(server_url=server.url, timeout=client_timeout).list_tools()
    return [dumped(tool) for tool in ours], [dumped(tool) for tool in theirs]


async def both_calls(
    server: Server, client_timeout: float | None = None
) -> tuple[dict[str, JsonValue], dict[str, JsonValue]]:
    ours = await MCPClient(server_url=server.url, timeout=client_timeout).call_tool(
        CallToolRequestParams(name=TOOL_NAME, arguments=TOOL_ARGUMENTS)
    )
    theirs = await LiteLLMClient(server_url=server.url, timeout=client_timeout).call_tool(
        TheirCallToolRequestParams(name=TOOL_NAME, arguments=TOOL_ARGUMENTS)
    )
    return dumped(ours), dumped(theirs)


async def observe_resources(client: ResourceClient) -> dict[str, JsonValue]:
    resources: list[JsonValue] = [dumped(item) for item in await client.list_resources()]
    templates: list[JsonValue] = [
        dumped(template) for template in await client.list_resource_templates()
    ]
    read = dumped(await client.read_resource(AnyUrl(RESOURCE_URL)))
    return {"resources": resources, "templates": templates, "read": read}


def our_auth_clients(server: Server) -> tuple[AuthClient, AuthClient, AuthClient, AuthClient]:
    return (
        MCPClient(server_url=server.url, auth_type=MCPAuth.bearer_token, auth_value=BEARER),
        MCPClient(server_url=server.url, auth_type=MCPAuth.basic, auth_value=BASIC_VALUE),
        MCPClient(server_url=server.url, auth_type=MCPAuth.api_key, auth_value=HEADER_MAP),
        MCPClient(server_url=server.url, auth_type=MCPAuth.bearer_token, auth_value=BEARER),
    )


def their_auth_clients(server: Server) -> tuple[AuthClient, AuthClient, AuthClient, AuthClient]:
    return (
        LiteLLMClient(server_url=server.url, auth_type="bearer_token", auth_value=BEARER),
        LiteLLMClient(server_url=server.url, auth_type="basic", auth_value=BASIC_VALUE),
        LiteLLMClient(server_url=server.url, auth_type="api_key", auth_value=HEADER_MAP),
        LiteLLMClient(server_url=server.url, auth_type="bearer_token", auth_value=BEARER),
    )


async def observe_auth(
    server: Server,
    clients: tuple[AuthClient, AuthClient, AuthClient, AuthClient],
) -> dict[str, JsonValue]:
    bearer, basic, keyed, renewed = clients
    await bearer.list_tools()
    listed = server.record.headers.get("authorization")
    await basic.list_tools()
    encoded = server.record.headers.get("authorization")
    await keyed.list_tools()
    mapped = server.record.headers.get("x-api-key")
    renewed.update_auth_value(UPDATED_BEARER)
    await renewed.list_tools()
    refreshed = server.record.headers.get("authorization")
    return {"bearer": listed, "basic": encoded, "map": mapped, "updated": refreshed}


async def our_fingerprints(server: Server) -> tuple[str, str]:
    client = MCPClient(server_url=server.url, auth_type=MCPAuth.bearer_token, auth_value=BEARER)
    first = await client.discovery_auth_fingerprint()
    client.update_auth_value(UPDATED_BEARER)
    second = await client.discovery_auth_fingerprint()
    return first, second


async def our_raised_details(server: Server) -> str:
    client = MCPClient(server_url=server.url)
    try:
        await client.list_tools(raise_on_error=True)
    except McpError as failure:
        return f"{failure.code}: {failure.message}"
    raise AssertionError("the protocol error became a result")


async def their_raised_details(server: Server) -> str:
    client = LiteLLMClient(server_url=server.url)
    try:
        await client.list_tools(raise_on_error=True)
    except MCP_ERRORS.MCPError as failure:
        return f"{failure.error.code}: {failure.error.message}"
    raise AssertionError("the protocol error became a result")


async def our_unauthorized(server: Server) -> None:
    client = MCPClient(server_url=server.url)
    await client.list_tools(raise_on_error=True)


async def their_unauthorized(server: Server) -> None:
    client = LiteLLMClient(server_url=server.url)
    await client.list_tools(raise_on_error=True)


async def our_timeout(server: Server) -> None:
    client = MCPClient(server_url=server.url, timeout=SHORT_TIMEOUT)
    await client.list_tools(raise_on_error=True)


async def their_timeout(server: Server) -> None:
    client = LiteLLMClient(server_url=server.url, timeout=SHORT_TIMEOUT)
    await client.list_tools(raise_on_error=True)


def test_signature_members() -> None:
    for label, ours, theirs in MEMBERS:
        assert parameters_of(ours) == parameters_of(theirs), label


def test_signature_refused_args() -> None:
    assert refusal_of(partial(MCPClient, aws_auth=REFUSED)) == "aws_auth"
    assert refusal_of(partial(MCPClient, resolved_auth=REFUSED)) == "resolved_auth"
    assert refusal_of(partial(MCPClient, sampling_callback=REFUSED)) == "sampling_callback"
    assert refusal_of(partial(MCPClient, elicitation_callback=REFUSED)) == "elicitation_callback"
    assert refusal_of(partial(MCPClient, logging_callback=REFUSED)) == "logging_callback"


def test_signature_oauth() -> None:
    with pytest.raises(ValueError, match="not supported by the native MCP client"):
        MCPClient(auth_type=MCPAuth.oauth2_id_jag)
    with pytest.raises(ValueError, match="not supported by the native MCP client"):
        MCPClient(auth_type=MCPAuth.true_passthrough)
    with pytest.raises(ValueError, match="not supported by the native MCP client"):
        MCPClient(auth_type=MCPAuth.oauth_delegate)


# ast-grep-ignore: no-one-line-functions
def test_signature_progress() -> None:
    assert asyncio.run(progress_refused()) == "host_progress_callback"


def test_signature_ssl_verify() -> None:
    context = ssl.create_default_context()
    refused_list: list[str] = [CA_BUNDLE]
    assert refusal_of(partial(MCPClient, ssl_verify=context)) == "ssl_verify"
    assert refusal_of(partial(MCPClient, ssl_verify=7)) == "ssl_verify"
    assert refusal_of(partial(MCPClient, ssl_verify=refused_list)) == "ssl_verify"


def test_signature_ssl_values() -> None:
    assert MCPClient(ssl_verify=True).ssl_verify is True
    assert MCPClient(ssl_verify=CA_BUNDLE).ssl_verify == CA_BUNDLE
    assert MCPClient().ssl_verify is None


def test_signature_protocol(monkeypatch: pytest.MonkeyPatch) -> None:
    accepted = MCPClient(protocol_version=MCP_LEGACY_VERSION)
    assert accepted.protocol_version == MCP_LEGACY_VERSION
    assert MCPClient().protocol_version == "auto"
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(client_module, "MCP_LEGACY_VERSIONS", ())
    with pytest.raises(ValueError, match="protocol_version must be one of") as raised:
        MCPClient(protocol_version=MCP_LEGACY_VERSION)
    assert str(raised.value) == "protocol_version must be one of ['auto']"


def test_signature_timeout(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv(TIMEOUT_VARIABLE, raising=False)
    assert MCPClient().timeout == DEFAULT_TIMEOUT
    monkeypatch.setenv(TIMEOUT_VARIABLE, CONFIGURED_TIMEOUT)
    assert MCPClient().timeout == CONFIGURED_SECONDS
    assert MCPClient(timeout=EXPLICIT_TIMEOUT).timeout == EXPLICIT_TIMEOUT
    monkeypatch.setenv(TIMEOUT_VARIABLE, BLANK_TIMEOUT)
    assert MCPClient().timeout == DEFAULT_TIMEOUT
    monkeypatch.setenv(TIMEOUT_VARIABLE, INVALID_TIMEOUT)
    with pytest.raises(ValueError, match=TIMEOUT_VARIABLE):
        MCPClient()


def test_methods_list_tools() -> None:
    with Server() as server:
        ours, theirs = asyncio.run(both_tool_lists(server))
    assert ours == theirs


def test_methods_call_tool() -> None:
    with Server() as server:
        ours, theirs = asyncio.run(both_calls(server))
    assert ours == theirs


def test_methods_prompts() -> None:
    with Server() as server:
        ours = asyncio.run(our_prompts(server))
        theirs = asyncio.run(their_prompts(server))
    assert ours == theirs


def test_methods_resources() -> None:
    with Server() as server:
        ours = asyncio.run(observe_resources(MCPClient(server_url=server.url)))
        theirs = asyncio.run(observe_resources(LiteLLMClient(server_url=server.url)))
    assert ours == theirs


def test_methods_auth_headers() -> None:
    with Server() as server:
        ours = asyncio.run(observe_auth(server, our_auth_clients(server)))
        theirs = asyncio.run(observe_auth(server, their_auth_clients(server)))
    assert ours == theirs == EXPECTED_AUTH_HEADERS


def test_methods_fingerprints() -> None:
    with Server() as server:
        first, second = asyncio.run(our_fingerprints(server))
    assert len(first) == FINGERPRINT_LENGTH
    assert first != second


def test_errors_native_types() -> None:
    for kind, expected in ERROR_TYPES.items():
        native = NativeMcpError(native_payload(kind))
        with pytest.raises(expected) as caught:
            raise_native_error(native)
        assert caught.type is expected
        assert caught.value.__cause__ is native


def test_errors_unknown_type() -> None:
    payload = native_payload("MysteryError")
    native = NativeMcpError(payload)
    with pytest.raises(UnknownNativeError) as caught:
        raise_native_error(native)
    assert caught.value.payload == payload
    assert caught.value.__cause__ is native


def test_errors_tool_result() -> None:
    failure = ValueError("boom")
    ours = dumped(MCPClient.error_tool_result(failure))
    theirs = dumped(LITELLM_CLIENT.MCPClient.error_tool_result(failure))
    assert ours["isError"] is True
    assert ours == theirs
    assert error_text(ours) == "ValueError: boom"


def test_errors_swallowed_result() -> None:
    with Server(mode=REFUSE) as server:
        ours, theirs = asyncio.run(both_calls(server))
    assert ours["isError"] is True
    assert ours == theirs
    assert error_text(ours) == f"MCPError: {DENIED_MESSAGE}"


def test_errors_swallowed_tools() -> None:
    with Server(mode=REFUSE) as server:
        ours, theirs = asyncio.run(both_tool_lists(server))
    assert ours == theirs == []


def test_errors_raised_protocol() -> None:
    with Server(mode=REFUSE) as server:
        ours = asyncio.run(our_raised_details(server))
        theirs = asyncio.run(their_raised_details(server))
    assert ours == theirs
    assert ours == f"{DENIED_CODE}: tools/list denied by policy"


@pytest.mark.parametrize(("mode", "ours_prefix", "theirs_prefix"), FAILURE_MODES)
def test_errors_swallowed_failure(mode: str, ours_prefix: str, theirs_prefix: str | None) -> None:
    with Server(mode=mode) as server:
        ours_tools, theirs_tools = asyncio.run(both_tool_lists(server, SHORT_TIMEOUT))
        ours_call, theirs_call = asyncio.run(both_calls(server, SHORT_TIMEOUT))
    assert ours_tools == theirs_tools == []
    assert ours_call["isError"] is True
    assert error_text(ours_call).startswith(ours_prefix)
    assert theirs_call["isError"] is True
    if theirs_prefix is not None:
        assert error_text(theirs_call).startswith(theirs_prefix)


def test_errors_http_status() -> None:
    native = NativeMcpError(native_payload("HTTPStatusError"))
    with pytest.raises(httpx2.HTTPStatusError) as caught:
        raise_native_error(native)
    assert caught.value.response.status_code == UNAUTHORIZED_STATUS
    assert caught.value.response.text == "denied"
    assert caught.value.response.headers["www-authenticate"] == 'Bearer realm="mcp"'
    assert str(caught.value.request.url) == PLACEHOLDER_URL
    assert str(caught.value) == ERROR_MESSAGE


def test_errors_401_raised() -> None:
    with Server(mode=UNAUTHORIZED_MODE) as server:
        with pytest.raises(httpx2.HTTPStatusError) as ours:
            asyncio.run(our_unauthorized(server))
        with pytest.raises(httpx2.HTTPStatusError) as theirs:
            asyncio.run(their_unauthorized(server))
    their_status = theirs.value.response.status_code
    assert ours.value.response.status_code == their_status == UNAUTHORIZED_STATUS
    assert ours.value.request.url == theirs.value.request.url
    assert str(ours.value) == str(theirs.value)


def test_errors_timeout_raised() -> None:
    with Server(mode=SLOW) as server:
        with pytest.raises(TimeoutError):
            asyncio.run(our_timeout(server))
        with pytest.raises(TimeoutError):
            asyncio.run(their_timeout(server))
