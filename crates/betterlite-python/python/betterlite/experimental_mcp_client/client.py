import json
from collections.abc import Awaitable, Callable
from typing import Final, Literal, Protocol, TypeVar, runtime_checkable

from pydantic import AnyUrl, JsonValue, TypeAdapter
from typing_extensions import Required, TypedDict

from betterlite._native import NativeMcpClient, NativeMcpError, NativeMcpSession
from betterlite._settings.env import env_number
from betterlite.experimental_mcp_client.errors import (
    UnsupportedMcpArgument,
    native_answer,
    raise_native_error,
)
from betterlite.experimental_mcp_client.types import (
    CALL_RESULT,
    PROMPT_LIST,
    PROMPT_RESULT,
    READ_RESULT,
    RESOURCE_LIST,
    TEMPLATE_LIST,
    TOOL_CALL,
    TOOL_LIST,
    TOOL_PAGE,
    CallToolRequestParams,
    CallToolResult,
    GetPromptRequestParams,
    GetPromptResult,
    InputRequiredResult,
    ListToolsResult,
    PaginatedRequestParams,
    Prompt,
    ReadResourceResult,
    Resource,
    ResourceTemplate,
    Tool,
)
from betterlite.types.llms.custom_http import VerifyTypes
from betterlite.types.llms.openai.chat.tool_params import ChatCompletionToolParam
from betterlite.types.mcp import (
    MCP_LEGACY_VERSIONS,
    MCPAuthType,
    MCPStdioConfig,
    MCPTransport,
    MCPTransportType,
    MCPUpstreamProtocol,
)

CLIENT_TIMEOUT_VARIABLE: Final = "LITELLM_MCP_CLIENT_TIMEOUT"
CLIENT_TIMEOUT_SECONDS: Final = 60.0
OPENAI_TOOLS: Final[TypeAdapter[list[ChatCompletionToolParam]]] = TypeAdapter(
    list[ChatCompletionToolParam]
)


# A value the Rust client cannot carry: refused by name, so the annotation accepts any value
class RefusedArgument(Protocol):
    ...


# The native applies the value before it answers, so only the returned coroutine needs closing
@runtime_checkable
class Pending(Awaitable[None], Protocol):
    def close(self) -> None:
        ...


class ClientSettings(TypedDict):
    server_url: str
    transport_type: str
    auth_type: str | None
    auth_value: str | dict[str, str] | None
    auth_header_name: str | None
    timeout: float
    stdio_config: MCPStdioConfig | None
    extra_headers: dict[str, str] | None
    ssl_verify: bool | str | None
    protocol_version: str


class CallToolWire(TypedDict, total=False):
    name: Required[str]
    arguments: dict[str, JsonValue] | None
    allow_input_required: bool
    raise_on_error: bool


class GetPromptWire(TypedDict):
    name: str
    arguments: dict[str, str] | None


class TextContentWire(TypedDict):
    type: Literal["text"]
    text: str


class ErrorResultWire(TypedDict):
    content: list[TextContentWire]
    isError: bool


SessionResult = TypeVar("SessionResult")


class MCPSession:
    def __init__(self, session: NativeMcpSession) -> None:
        self._session = session

    async def list_tools(self, params: PaginatedRequestParams | None = None) -> ListToolsResult:
        cursor = None if params is None else params.cursor
        text = await native_answer(self._session.list_tools_page(cursor))
        return TOOL_PAGE.validate_json(text)

    async def call_tool(
        self, name: str, arguments: dict[str, JsonValue] | None = None
    ) -> CallToolResult:
        request: CallToolWire = {"name": name, "arguments": arguments}
        text = await native_answer(self._session.call_tool(json.dumps(request)))
        return CALL_RESULT.validate_json(text)

    async def load_mcp_tools(self, listing_deadline: float | None = None) -> list[Tool]:
        text = await native_answer(self._session.load_tools("mcp", listing_deadline))
        return TOOL_LIST.validate_json(text)

    async def load_openai_tools(self) -> list[ChatCompletionToolParam]:
        text = await native_answer(self._session.load_tools("openai"))
        return OPENAI_TOOLS.validate_json(text)


class MCPClient:
    # LiteLLM's fifteen constructor arguments, the refused ones included (client.py L386-L428)
    def __init__(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/experimental_mcp_client/client.py#L386-L428
        self,
        server_url: str = "",
        transport_type: MCPTransportType = MCPTransport.http,
        auth_type: MCPAuthType = None,
        auth_value: str | dict[str, str] | None = None,
        auth_header_name: str | None = None,
        timeout: float | None = None,
        stdio_config: MCPStdioConfig | None = None,
        extra_headers: dict[str, str] | None = None,
        ssl_verify: VerifyTypes | RefusedArgument | None = None,
        aws_auth: RefusedArgument | None = None,
        resolved_auth: RefusedArgument | None = None,
        sampling_callback: RefusedArgument | None = None,
        elicitation_callback: RefusedArgument | None = None,
        logging_callback: RefusedArgument | None = None,
        protocol_version: MCPUpstreamProtocol = "auto",
    ) -> None:
        refused = (
            ("aws_auth", aws_auth),
            ("resolved_auth", resolved_auth),
            ("sampling_callback", sampling_callback),
            ("elicitation_callback", elicitation_callback),
            ("logging_callback", logging_callback),
        )
        for name, value in refused:
            if value is not None:
                raise UnsupportedMcpArgument(name)
        allowed = ("auto", *MCP_LEGACY_VERSIONS)
        if protocol_version not in allowed:
            raise ValueError(f"protocol_version must be one of {list(allowed)}")
        self.ssl_verify: bool | str | None = None
        if ssl_verify is not None:
            match ssl_verify:
                case bool() | str():
                    self.ssl_verify = ssl_verify
                case object():
                    raise UnsupportedMcpArgument("ssl_verify")
        default_timeout = env_number(CLIENT_TIMEOUT_VARIABLE, CLIENT_TIMEOUT_SECONDS, float)
        self.server_url: str = server_url
        self.transport_type: MCPTransport = transport_type
        self.auth_type: MCPAuthType = auth_type
        self.timeout: float = default_timeout if timeout is None else timeout
        self.protocol_version: MCPUpstreamProtocol = protocol_version
        self._mcp_auth_value: str | dict[str, str] | None = auth_value
        self.stdio_config: MCPStdioConfig | None = stdio_config
        self.extra_headers: dict[str, str] | None = extra_headers
        settings: ClientSettings = {
            "server_url": server_url,
            "transport_type": transport_type,
            "auth_type": auth_type,
            "auth_value": auth_value,
            "auth_header_name": auth_header_name,
            "timeout": self.timeout,
            "stdio_config": stdio_config,
            "extra_headers": extra_headers,
            "ssl_verify": self.ssl_verify,
            "protocol_version": protocol_version,
        }
        try:
            self._client: NativeMcpClient = NativeMcpClient(json.dumps(settings))
        except NativeMcpError as failure:
            raise_native_error(failure)

    # ast-grep-ignore: no-one-line-functions
    async def discovery_auth_fingerprint(self) -> str:
        return await native_answer(self._client.discovery_auth_fingerprint())

    async def list_tools(self, raise_on_error: bool = False) -> list[Tool]:  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/experimental_mcp_client/client.py#L817
        text = await native_answer(self._client.list_tools(raise_on_error))
        return TOOL_LIST.validate_json(text)

    @staticmethod
    def error_tool_result(exc: Exception) -> CallToolResult:
        message = f"{type(exc).__name__}: {exc}"
        result: ErrorResultWire = {
            "content": [{"type": "text", "text": message}],
            "isError": True,
        }
        return CALL_RESULT.validate_python(result)

    async def call_tool(
        self,
        call_tool_request_params: CallToolRequestParams,
        host_progress_callback: RefusedArgument | None = None,
        raise_on_error: bool = False, allow_input_required: bool = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/experimental_mcp_client/client.py#L878-L884
    ) -> CallToolResult | InputRequiredResult:
        if host_progress_callback is not None:
            raise UnsupportedMcpArgument("host_progress_callback")
        request: CallToolWire = {
            "name": call_tool_request_params.name,
            "arguments": call_tool_request_params.arguments,
            "allow_input_required": allow_input_required,
            "raise_on_error": raise_on_error,
        }
        text = await native_answer(self._client.call_tool(json.dumps(request)))
        return TOOL_CALL.validate_json(text)

    async def list_prompts(self, *, raise_on_error: bool = False) -> list[Prompt]:
        text = await native_answer(self._client.list_prompts(raise_on_error))
        return PROMPT_LIST.validate_json(text)

    async def get_prompt(
        self, get_prompt_request_params: GetPromptRequestParams
    ) -> GetPromptResult:
        request: GetPromptWire = {
            "name": get_prompt_request_params.name,
            "arguments": get_prompt_request_params.arguments,
        }
        text = await native_answer(self._client.get_prompt(json.dumps(request)))
        return PROMPT_RESULT.validate_json(text)

    async def list_resources(self, *, raise_on_error: bool = False) -> list[Resource]:
        text = await native_answer(self._client.list_resources(raise_on_error))
        return RESOURCE_LIST.validate_json(text)

    async def list_resource_templates(
        self, *, raise_on_error: bool = False
    ) -> list[ResourceTemplate]:
        text = await native_answer(self._client.list_resource_templates(raise_on_error))
        return TEMPLATE_LIST.validate_json(text)

    async def read_resource(self, url: AnyUrl) -> ReadResourceResult:
        text = await native_answer(self._client.read_resource(str(url)))
        return READ_RESULT.validate_json(text)

    def update_auth_value(self, mcp_auth_value: str | dict[str, str]) -> None:
        pending: Awaitable[None] = self._client.update_auth_value(
            json.dumps(mcp_auth_value)
        )
        self._mcp_auth_value = mcp_auth_value
        match pending:
            case Pending():
                pending.close()
            case Awaitable():
                pass

    async def run_with_session(
        self,
        operation: Callable[[MCPSession], Awaitable[SessionResult]],
        *,
        quiet_on_error: bool = False,
    ) -> SessionResult:
        native = await native_answer(self._client.open_session(quiet_on_error))
        session = MCPSession(native)
        try:
            return await operation(session)
        finally:
            await native_answer(native.aclose())
