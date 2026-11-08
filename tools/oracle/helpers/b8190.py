import asyncio
from types import SimpleNamespace
from typing import Literal, NotRequired, TypedDict, cast
from unittest.mock import patch

from litellm.proxy import proxy_server
from litellm.proxy._experimental.mcp_server import capabilities, exceptions, result_conversion
from litellm.proxy._experimental.mcp_server.result_conversion import (
    JsonResult,
    TextResult,
    ToolOutcome,
    WireCompat,
)
from litellm.types.mcp import MCPTransport
from mcp.server.context import HandlerResult, ServerRequestContext
from mcp.shared.exceptions import MCPError
from mcp.types import (
    CallToolResult,
    InitializeResult,
    InputRequiredResult,
    ServerCapabilities,
    Tool,
)

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class DiscoveryInput(TypedDict):
    function: Literal["discovery"]
    configured: list[str]
    revision: str
    transport: Literal["sse", "http", "stdio"]
    authorized_operations: list[str]
    upstream_versions: list[str]
    capabilities: dict[str, Json]
    client_extensions: NotRequired[list[str]]
    upstream_extensions: NotRequired[list[str]]
    instructions: NotRequired[str | None]


class VersionPolicyInput(TypedDict):
    function: Literal["version_policy"]
    versions: list[str]
    method: str
    params: NotRequired[dict[str, Json]]
    protocol_version: NotRequired[str | None]
    initialized: dict[str, Json]


class AdvertisedVersionsInput(TypedDict):
    function: Literal["advertised_versions"]
    configured: NotRequired[list[str] | None]


class WireCompatInput(TypedDict):
    function: Literal["wire_compat_for"]
    protocol_version: str


class ParseBodyInput(TypedDict):
    function: Literal["parse_http_body"]
    body: str


class TextOutcome(TypedDict):
    kind: Literal["text"]
    text: str


class JsonOutcome(TypedDict):
    kind: Literal["json"]
    value: Json
    original_text: str


class ResultOutcome(TypedDict):
    kind: Literal["call_tool_result", "input_required"]
    result: dict[str, Json]


class ExceptionOutcome(TypedDict):
    kind: Literal["exception"]
    type_name: str
    message: str


OutcomeInput = TextOutcome | JsonOutcome | ResultOutcome | ExceptionOutcome


class ConvertInput(TypedDict):
    function: Literal["to_call_tool_result", "complete_call_tool_result"]
    outcome: OutcomeInput
    compat: Literal["legacy", "modern"]


class GatewayToolInput(TypedDict):
    function: Literal["to_gateway_tool"]
    tool: dict[str, Json]
    name: str


class UrlCredentialsInput(TypedDict):
    function: Literal["url_credentials"]


class UpstreamAuthInput(TypedDict):
    function: Literal["upstream_auth"]
    status_code: int
    www_authenticate: NotRequired[str | None]
    server_name: str
    base_url: NotRequired[str | None]
    request_path: NotRequired[str | None]


class OpenApiUpstreamInput(TypedDict):
    function: Literal["openapi_upstream"]
    status_code: int
    server_name: str


class ToolResultErrorInput(TypedDict):
    function: Literal["tool_result_error"]
    message: str


class ServerListErrorInput(TypedDict):
    function: Literal["server_list_error"]
    fault: str
    server_name: str


CapabilityInput = DiscoveryInput | VersionPolicyInput | AdvertisedVersionsInput
ConversionInput = WireCompatInput | ParseBodyInput | ConvertInput | GatewayToolInput
ErrorInput = (
    UrlCredentialsInput
    | UpstreamAuthInput
    | OpenApiUpstreamInput
    | ToolResultErrorInput
    | ServerListErrorInput
)
Input = CapabilityInput | ConversionInput | ErrorInput


def _wire(model: CallToolResult | InputRequiredResult | InitializeResult | Tool) -> dict[str, Json]:
    return cast(dict[str, Json], model.model_dump(mode="json", by_alias=True, exclude_none=True))


def _discovery(input_value: DiscoveryInput) -> Json:
    result = capabilities.build_discovery(
        configured=tuple(input_value["configured"]),
        revision=input_value["revision"],
        transport=MCPTransport(input_value["transport"]),
        authorized_operations=frozenset(input_value["authorized_operations"]),
        upstream_versions=frozenset(input_value["upstream_versions"]),
        capabilities=ServerCapabilities.model_validate(input_value["capabilities"]),
        client_extensions=frozenset(input_value.get("client_extensions", [])),
        upstream_extensions=frozenset(input_value.get("upstream_extensions", [])),
        instructions=input_value.get("instructions"),
    )
    return cast(dict[str, Json], result.model_dump(mode="json", by_alias=True, exclude_none=True))


class Downstream:
    def __init__(self, input_value: VersionPolicyInput) -> None:
        self.input_value: VersionPolicyInput = input_value

    async def __call__(self, _context: ServerRequestContext[object, object]) -> HandlerResult:
        if self.input_value["method"] == "initialize":
            return InitializeResult.model_validate(self.input_value["initialized"])
        return {"called": True}


async def _apply_policy(input_value: VersionPolicyInput) -> Json:
    policy = capabilities.GatewayVersionPolicy(lambda: tuple(input_value["versions"]))
    context = SimpleNamespace(
        method=input_value["method"],
        params=input_value.get("params"),
        protocol_version=input_value.get("protocol_version"),
    )

    try:
        result = await policy(
            cast(ServerRequestContext[object, object], cast(object, context)),
            Downstream(input_value),
        )
    except MCPError as rejection:
        return {
            "rejected": {
                "code": rejection.error.code,
                "message": rejection.error.message,
                "data": cast(Json, rejection.error.data),
            }
        }
    if isinstance(result, InitializeResult):
        return {"result": _wire(result)}
    return {"result": cast(Json, result)}


def _advertised_versions(input_value: AdvertisedVersionsInput) -> Json:
    settings = {"mcp_advertised_versions": input_value.get("configured")}
    with patch.object(proxy_server, "general_settings_view", lambda: settings):
        return list(capabilities.configured_versions())


def _outcome(source: OutcomeInput) -> ToolOutcome:
    match source["kind"]:
        case "text":
            return TextResult(source["text"])
        case "json":
            return JsonResult(value=source["value"], original_text=source["original_text"])
        case "call_tool_result":
            return CallToolResult.model_validate(source["result"])
        case "input_required":
            return InputRequiredResult.model_validate(source["result"])
        case "exception":
            return type(source["type_name"], (Exception,), {})(source["message"])


def _convert(input_value: ConvertInput) -> Json:
    outcome = _outcome(input_value["outcome"])
    compat = WireCompat(input_value["compat"])
    if input_value["function"] == "complete_call_tool_result":
        return {"complete": _wire(result_conversion.complete_call_tool_result(outcome, compat))}
    converted = result_conversion.to_call_tool_result(outcome, compat)
    if isinstance(converted, InputRequiredResult):
        return {"input_required": _wire(converted)}
    return {"complete": _wire(converted)}


def _errors(input_value: ErrorInput) -> Json:
    match input_value["function"]:
        case "url_credentials":
            url_error = exceptions.MCPServerURLCredentialsError()
            return {
                "status_code": url_error.status_code,
                "detail": cast(Json, url_error.detail),
                "headers": cast(Json, dict(url_error.headers or {})),
            }
        case "upstream_auth":
            auth_error = exceptions.MCPUpstreamAuthError(
                input_value["status_code"],
                input_value.get("www_authenticate"),
                input_value["server_name"],
            )
            http_error = auth_error.to_http_exception(
                input_value.get("base_url"), input_value.get("request_path")
            )
            return {
                "message": str(auth_error),
                "status_code": http_error.status_code,
                "detail": cast(Json, http_error.detail),
                "headers": cast(Json, dict(http_error.headers or {})),
            }
        case "openapi_upstream":
            openapi_error = exceptions.MCPOpenApiUpstreamError(
                input_value["status_code"], input_value["server_name"]
            )
            return {
                "message": str(openapi_error),
                "status_code": openapi_error.status_code,
                "server_name": openapi_error.server_name,
            }
        case "tool_result_error":
            return {"message": str(exceptions.MCPToolResultError(input_value["message"]))}
        case "server_list_error":
            list_error = exceptions.MCPServerListError(
                input_value["fault"], input_value["server_name"]
            )
            return {
                "message": str(list_error),
                "fault": cast(Json, list_error.fault),
                "server_name": list_error.server_name,
            }


def _conversion(input_value: ConversionInput | ErrorInput) -> Json:
    match input_value["function"]:
        case "wire_compat_for":
            return result_conversion.wire_compat_for(input_value["protocol_version"]).value
        case "parse_http_body":
            out = result_conversion.parse_http_body(input_value["body"])
            if isinstance(out, JsonResult):
                return {"kind": "json", "value": out.value, "original_text": out.original_text}
            return {"kind": "text", "text": out.text}
        case "to_call_tool_result" | "complete_call_tool_result":
            return _convert(input_value)
        case "to_gateway_tool":
            tool = Tool.model_validate(input_value["tool"])
            return _wire(result_conversion.to_gateway_tool(tool, input_value["name"]))
        case _:
            return _errors(input_value)


def run(input_value: Input) -> Json:
    match input_value["function"]:
        case "discovery":
            return _discovery(input_value)
        case "version_policy":
            return asyncio.run(_apply_policy(input_value))
        case "advertised_versions":
            return _advertised_versions(input_value)
        case _:
            return _conversion(input_value)
