import asyncio
import json
import ssl
from collections.abc import Callable
from functools import partial
from inspect import Signature, signature
from typing import Final, Protocol, TypedDict

import httpx2
import pytest
from litellm.experimental_mcp_client.client import MCPClient as LiteLLMClient
from litellm.types.mcp import MCPAuth as TheirAuth
from mcp import MCPError as TheirProtocolError
from mcp.types import CallToolRequestParams as TheirCallToolRequestParams
from mcp.types import CallToolResult as TheirCallToolResult
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
SHORT_TIMEOUT: Final = 0.2
CA_BUNDLE: Final = "/etc/ssl/certs/ca.pem"
PLACEHOLDER_URL: Final = "http://127.0.0.1:1/mcp"
INTERNAL_ERROR: Final = -32603
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

MEMBERS: Final[tuple[tuple[str, Signature, Signature], ...]] = (
    ("MCPClient", signature(MCPClient), signature(LiteLLMClient)),
    ("list_tools", signature(MCPClient.list_tools), signature(LiteLLMClient.list_tools)),
    ("call_tool", signature(MCPClient.call_tool), signature(LiteLLMClient.call_tool)),
    ("list_prompts", signature(MCPClient.list_prompts), signature(LiteLLMClient.list_prompts)),
    ("get_prompt", signature(MCPClient.get_prompt), signature(LiteLLMClient.get_prompt)),
    (
        "list_resources",
        signature(MCPClient.list_resources),
        signature(LiteLLMClient.list_resources),
    ),
    (
        "list_resource_templates",
        signature(MCPClient.list_resource_templates),
        signature(LiteLLMClient.list_resource_templates),
    ),
    ("read_resource", signature(MCPClient.read_resource), signature(LiteLLMClient.read_resource)),
    (
        "discovery_auth_fingerprint",
        signature(MCPClient.discovery_auth_fingerprint),
        signature(LiteLLMClient.discovery_auth_fingerprint),
    ),
    (
        "error_tool_result",
        signature(MCPClient.error_tool_result),
        signature(LiteLLMClient.error_tool_result),
    ),
    (
        "update_auth_value",
        signature(MCPClient.update_auth_value),
        signature(LiteLLMClient.update_auth_value),
    ),
    (
        "run_with_session",
        signature(MCPClient.run_with_session),
        signature(LiteLLMClient.run_with_session),
    ),
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


# ast-grep-ignore: no-one-line-functions
def dumped(value: Dumpable) -> dict[str, JsonValue]:
    return value.model_dump(by_alias=True, exclude_none=True)


def error_text(fields: dict[str, JsonValue]) -> str:
    result = RESULT.validate_python(fields)
    return result["content"][0]["text"]


def parameters_of(declaration: Signature) -> list[tuple[str, str, str]]:
    parameters = declaration.parameters.values()
    return [
        (parameter.name, str(parameter.kind), repr(parameter.default))
        for parameter in parameters
    ]


def refusal_of(build: Callable[[], MCPClient]) -> str:
    try:
        build()
    except UnsupportedMcpArgument as failure:
        return failure.argument
    raise AssertionError("the refused argument was accepted")


def native_payload(kind: str) -> str:
    payload: dict[str, JsonValue] = {
        "type": kind,
        "message": ERROR_MESSAGE,
        "code": DENIED_CODE,
        "data": None,
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


async def our_tools(server: Server) -> list[dict[str, JsonValue]]:
    client = MCPClient(server_url=server.url)
    return [dumped(tool) for tool in await client.list_tools()]


async def their_tools(server: Server) -> list[dict[str, JsonValue]]:
    client = LiteLLMClient(server_url=server.url)
    return [dumped(tool) for tool in await client.list_tools()]


async def our_call(server: Server) -> dict[str, JsonValue]:
    client = MCPClient(server_url=server.url)
    params = CallToolRequestParams(name=TOOL_NAME, arguments=TOOL_ARGUMENTS)
    return dumped(await client.call_tool(params))


async def their_call(server: Server) -> dict[str, JsonValue]:
    client = LiteLLMClient(server_url=server.url)
    params = TheirCallToolRequestParams(name=TOOL_NAME, arguments=TOOL_ARGUMENTS)
    return dumped(await client.call_tool(params))


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


async def our_resources(server: Server) -> dict[str, JsonValue]:
    client = MCPClient(server_url=server.url)
    resources: list[JsonValue] = [dumped(item) for item in await client.list_resources()]
    templates: list[JsonValue] = [
        dumped(template) for template in await client.list_resource_templates()
    ]
    read = dumped(await client.read_resource(AnyUrl(RESOURCE_URL)))
    return {"resources": resources, "templates": templates, "read": read}


async def their_resources(server: Server) -> dict[str, JsonValue]:
    client = LiteLLMClient(server_url=server.url)
    resources: list[JsonValue] = [dumped(item) for item in await client.list_resources()]
    templates: list[JsonValue] = [
        dumped(template) for template in await client.list_resource_templates()
    ]
    read = dumped(await client.read_resource(AnyUrl(RESOURCE_URL)))
    return {"resources": resources, "templates": templates, "read": read}


async def our_auth_headers(server: Server) -> dict[str, JsonValue]:
    bearer = MCPClient(server_url=server.url, auth_type=MCPAuth.bearer_token, auth_value=BEARER)
    await bearer.list_tools()
    listed = server.record.headers.get("authorization")
    basic = MCPClient(server_url=server.url, auth_type=MCPAuth.basic, auth_value=BASIC_VALUE)
    await basic.list_tools()
    encoded = server.record.headers.get("authorization")
    header_map = MCPClient(server_url=server.url, auth_type=MCPAuth.api_key, auth_value=HEADER_MAP)
    await header_map.list_tools()
    mapped = server.record.headers.get("x-api-key")
    updated = MCPClient(server_url=server.url, auth_type=MCPAuth.bearer_token, auth_value=BEARER)
    updated.update_auth_value(UPDATED_BEARER)
    await updated.list_tools()
    renewed = server.record.headers.get("authorization")
    return {"bearer": listed, "basic": encoded, "map": mapped, "updated": renewed}


async def their_auth_headers(server: Server) -> dict[str, JsonValue]:
    bearer = LiteLLMClient(
        server_url=server.url, auth_type=TheirAuth.bearer_token, auth_value=BEARER
    )
    await bearer.list_tools()
    listed = server.record.headers.get("authorization")
    basic = LiteLLMClient(server_url=server.url, auth_type=TheirAuth.basic, auth_value=BASIC_VALUE)
    await basic.list_tools()
    encoded = server.record.headers.get("authorization")
    header_map = LiteLLMClient(
        server_url=server.url, auth_type=TheirAuth.api_key, auth_value=HEADER_MAP
    )
    await header_map.list_tools()
    mapped = server.record.headers.get("x-api-key")
    updated = LiteLLMClient(
        server_url=server.url, auth_type=TheirAuth.bearer_token, auth_value=BEARER
    )
    updated.update_auth_value(UPDATED_BEARER)
    await updated.list_tools()
    renewed = server.record.headers.get("authorization")
    return {"bearer": listed, "basic": encoded, "map": mapped, "updated": renewed}


async def our_fingerprints(server: Server) -> tuple[str, str]:
    client = MCPClient(server_url=server.url, auth_type=MCPAuth.bearer_token, auth_value=BEARER)
    first = await client.discovery_auth_fingerprint()
    client.update_auth_value(UPDATED_BEARER)
    second = await client.discovery_auth_fingerprint()
    return first, second


async def both_swallowed_calls(
    server: Server,
) -> tuple[dict[str, JsonValue], TheirCallToolResult]:
    ours = await MCPClient(server_url=server.url).call_tool(CallToolRequestParams(name=TOOL_NAME))
    theirs = await LiteLLMClient(server_url=server.url).call_tool(
        TheirCallToolRequestParams(name=TOOL_NAME)
    )
    return dumped(ours), theirs


async def both_swallowed_tools(
    server: Server,
) -> tuple[list[dict[str, JsonValue]], list[dict[str, JsonValue]]]:
    ours = [dumped(tool) for tool in await MCPClient(server_url=server.url).list_tools()]
    theirs = [dumped(tool) for tool in await LiteLLMClient(server_url=server.url).list_tools()]
    return ours, theirs


async def both_slow_tools(
    server: Server,
) -> tuple[list[dict[str, JsonValue]], list[dict[str, JsonValue]]]:
    client = MCPClient(server_url=server.url, timeout=SHORT_TIMEOUT)
    reference = LiteLLMClient(server_url=server.url, timeout=SHORT_TIMEOUT)
    ours = [dumped(tool) for tool in await client.list_tools()]
    theirs = [dumped(tool) for tool in await reference.list_tools()]
    return ours, theirs


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
    except TheirProtocolError as failure:
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


async def both_slow_calls(
    server: Server,
) -> tuple[dict[str, JsonValue], TheirCallToolResult]:
    ours = await MCPClient(server_url=server.url, timeout=SHORT_TIMEOUT).call_tool(
        CallToolRequestParams(name=TOOL_NAME)
    )
    theirs = await LiteLLMClient(server_url=server.url, timeout=SHORT_TIMEOUT).call_tool(
        TheirCallToolRequestParams(name=TOOL_NAME)
    )
    return dumped(ours), theirs


def test_signature_members() -> None:
    for label, ours, theirs in MEMBERS:
        assert parameters_of(ours) == parameters_of(theirs), label


def test_signature_refused_args() -> None:
    assert refusal_of(partial(MCPClient, aws_auth=REFUSED)) == "aws_auth"
    assert refusal_of(partial(MCPClient, resolved_auth=REFUSED)) == "resolved_auth"
    assert refusal_of(partial(MCPClient, sampling_callback=REFUSED)) == "sampling_callback"
    assert refusal_of(partial(MCPClient, elicitation_callback=REFUSED)) == "elicitation_callback"
    assert refusal_of(partial(MCPClient, logging_callback=REFUSED)) == "logging_callback"


# ast-grep-ignore: no-one-line-functions
def test_signature_progress() -> None:
    assert asyncio.run(progress_refused()) == "host_progress_callback"


def test_signature_ssl_verify() -> None:
    context = ssl.create_default_context()
    assert refusal_of(partial(MCPClient, ssl_verify=context)) == "ssl_verify"


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


def test_methods_list_tools() -> None:
    with Server() as server:
        ours = asyncio.run(our_tools(server))
        theirs = asyncio.run(their_tools(server))
    assert ours == theirs


def test_methods_call_tool() -> None:
    with Server() as server:
        ours = asyncio.run(our_call(server))
        theirs = asyncio.run(their_call(server))
    assert ours == theirs


def test_methods_prompts() -> None:
    with Server() as server:
        ours = asyncio.run(our_prompts(server))
        theirs = asyncio.run(their_prompts(server))
    assert ours == theirs


def test_methods_resources() -> None:
    with Server() as server:
        ours = asyncio.run(our_resources(server))
        theirs = asyncio.run(their_resources(server))
    assert ours == theirs


def test_methods_auth_headers() -> None:
    with Server() as server:
        ours = asyncio.run(our_auth_headers(server))
        theirs = asyncio.run(their_auth_headers(server))
    assert ours == theirs
    assert ours["bearer"] == f"Bearer {BEARER}"
    assert ours["basic"] == BASIC_HEADER
    assert ours["map"] == HEADER_MAP["X-Api-Key"]
    assert ours["updated"] == f"Bearer {UPDATED_BEARER}"


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


def test_errors_swallowed_result() -> None:
    with Server(mode=REFUSE) as server:
        ours, theirs = asyncio.run(both_swallowed_calls(server))
    assert ours["isError"] is True
    assert ours == dumped(theirs)
    assert error_text(ours) == f"MCPError: {DENIED_MESSAGE}"


def test_errors_swallowed_tools() -> None:
    with Server(mode=REFUSE) as server:
        ours, theirs = asyncio.run(both_swallowed_tools(server))
    assert ours == theirs == []


def test_errors_raised_protocol() -> None:
    with Server(mode=REFUSE) as server:
        ours = asyncio.run(our_raised_details(server))
        theirs = asyncio.run(their_raised_details(server))
    assert ours == theirs
    assert ours == f"{DENIED_CODE}: tools/list denied by policy"


def test_errors_401_swallowed() -> None:
    with Server(mode=UNAUTHORIZED_MODE) as server:
        ours_tools, theirs_tools = asyncio.run(both_swallowed_tools(server))
        ours_call, theirs_call = asyncio.run(both_swallowed_calls(server))
    assert ours_tools == theirs_tools == []
    assert ours_call["isError"] is True
    assert dumped(theirs_call)["isError"] is True


def test_errors_401_raised() -> None:
    with Server(mode=UNAUTHORIZED_MODE) as server:
        with pytest.raises(McpError) as ours:
            asyncio.run(our_unauthorized(server))
        with pytest.raises(httpx2.HTTPStatusError):
            asyncio.run(their_unauthorized(server))
    assert ours.value.code == INTERNAL_ERROR


def test_errors_timeout_swallowed() -> None:
    with Server(mode=SLOW) as server:
        ours_tools, theirs_tools = asyncio.run(both_slow_tools(server))
        ours_call, theirs_call = asyncio.run(both_slow_calls(server))
    assert ours_tools == theirs_tools == []
    assert ours_call["isError"] is True
    assert error_text(ours_call).startswith("TimeoutError: ")
    assert error_text(dumped(theirs_call)).startswith("TimeoutError: ")


def test_errors_timeout_raised() -> None:
    with Server(mode=SLOW) as server:
        with pytest.raises(TimeoutError):
            asyncio.run(our_timeout(server))
        with pytest.raises(TimeoutError):
            asyncio.run(their_timeout(server))
