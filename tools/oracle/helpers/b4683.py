import asyncio
from collections.abc import Awaitable, Callable
from typing import Final, Literal, TypeAlias, TypedDict

import httpx
from fastapi import HTTPException
from litellm.proxy._experimental.mcp_server import openapi_to_mcp_generator
from litellm.proxy._experimental.mcp_server.exceptions import (
    MCPOpenApiUpstreamError,
    MCPUpstreamAuthError,
)
from litellm.proxy._experimental.mcp_server.openapi_to_mcp_generator import (
    build_input_schema,
    create_tool_function,
    extract_parameters,
    get_base_url,
    resolve_operation_params,
    sanitize_openapi_tool_name,
)
from litellm.proxy._experimental.mcp_server.tool_outcome import JsonResult, TextResult
from litellm.types.mcp import MCPAuthType
from pydantic import JsonValue, TypeAdapter

_JSON: Final = TypeAdapter(JsonValue)
_AUTH_MODE: Final = TypeAdapter(MCPAuthType)
_ARGUMENTS: Final = TypeAdapter(dict[str, JsonValue])
_SKIPPED_HEADERS: Final = frozenset(
    {
        "host",
        "accept",
        "accept-encoding",
        "connection",
        "user-agent",
        "content-length",
        "content-type",
    }
)

Outcome: TypeAlias = (
    TextResult | JsonResult | HTTPException | MCPUpstreamAuthError | MCPOpenApiUpstreamError
)


class ToolNameInput(TypedDict):
    operation: Literal["tool_name"]
    raw: str


class BaseUrlInput(TypedDict):
    operation: Literal["base_url"]
    spec: JsonValue
    spec_path: str | None


class ResolveInput(TypedDict):
    operation: Literal["resolve_params"]
    spec_operation: JsonValue
    path_item: JsonValue
    components: JsonValue


class OperationInput(TypedDict):
    operation: Literal["extract_params", "input_schema"]
    spec_operation: JsonValue


class MergeHeadersInput(TypedDict):
    operation: Literal["merge_headers"]
    static_headers: dict[str, str]
    extra_headers: dict[str, str]
    auth_header: str | None
    resolved_auth_headers: dict[str, str]


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class CallInput(TypedDict):
    operation: Literal["call"]
    path: str
    method: str
    spec_operation: JsonValue
    base_url: str
    headers: dict[str, str]
    server_label: str | None
    relays_upstream_auth: bool
    auth_type: str | None
    upstream_token_header: str | None
    arguments: JsonValue
    auth_header: str | None
    extra_headers: dict[str, str]
    resolved_auth_headers: dict[str, str]
    reply: Reply


class SchemaOut(TypedDict, total=False):
    type: str
    properties: dict[str, JsonValue]


ParameterOut = TypedDict(
    "ParameterOut",
    {"in": str, "name": str, "description": str, "required": bool, "schema": SchemaOut},
    total=False,
)


_PARAMETERS: Final = TypeAdapter(list[ParameterOut])

Input: TypeAlias = (
    ToolNameInput | BaseUrlInput | ResolveInput | OperationInput | MergeHeadersInput | CallInput
)


class Capture:
    def __init__(self, reply: Reply) -> None:
        self.reply = reply
        self.request: httpx.Request | None = None

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.request = request
        return httpx.Response(
            self.reply["status"],
            headers=self.reply["headers"],
            text=self.reply["body"],
            request=request,
        )


def _parameter_type(parameter: ParameterOut) -> str | None:
    schema = parameter.get("schema")
    if schema is None:
        return None
    return schema.get("type")


def _request_answer(request: httpx.Request | None) -> JsonValue:
    if request is None:
        return None
    body = _JSON.validate_json(request.content) if request.content else None
    return _JSON.validate_python(
        {
            "method": request.method,
            "url": str(request.url.copy_with(query=None)),
            "query": [[name, value] for name, value in request.url.params.multi_items()],
            "headers": {
                name.lower(): value
                for name, value in request.headers.multi_items()
                if name.lower() not in _SKIPPED_HEADERS
            },
            "body": body,
        }
    )


def _outcome_answer(outcome: Outcome) -> JsonValue:
    answer: JsonValue = None
    match outcome:
        case TextResult(text=text):
            answer = _JSON.validate_python({"kind": "text", "text": text})
        case JsonResult(value=value, original_text=text):
            answer = _JSON.validate_python({"kind": "json", "text": text, "value": value})
        case HTTPException() | MCPUpstreamAuthError() | MCPOpenApiUpstreamError():
            answer = None
    return answer


def _error_answer(outcome: Outcome) -> JsonValue:
    answer: JsonValue = None
    match outcome:
        case TextResult() | JsonResult():
            answer = None
        case HTTPException(status_code=status):
            answer = _JSON.validate_python({"kind": "credential", "status": status})
        case MCPUpstreamAuthError() as failure:
            answer = _JSON.validate_python(
                {
                    "kind": "upstream_auth",
                    "status": failure.status_code,
                    "www_authenticate": failure.www_authenticate,
                    "server": failure.server_name,
                }
            )
        case MCPOpenApiUpstreamError() as failure:
            answer = _JSON.validate_python(
                {
                    "kind": "upstream_status",
                    "status": failure.status_code,
                    "server": failure.server_name,
                }
            )
    return answer


async def _invoke(
    tool: Callable[..., Awaitable[TextResult | JsonResult]],
    arguments: dict[str, JsonValue],
    reply: Reply,
    call: CallInput,
) -> tuple[httpx.Request | None, Outcome]:
    capture = Capture(reply)
    client = httpx.AsyncClient(transport=httpx.MockTransport(capture.handle))
    handler = openapi_to_mcp_generator._upstream_client()  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L568
    original = handler.client
    auth_token = openapi_to_mcp_generator._request_auth_header.set(call["auth_header"])  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L109
    extra_token = openapi_to_mcp_generator._request_extra_headers.set(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L112
        call["extra_headers"]
    )
    resolved_token = openapi_to_mcp_generator._request_resolved_auth_headers.set(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L118
        call["resolved_auth_headers"]
    )
    handler._client = client  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L629-L647
    try:
        outcome = await tool(**arguments)
    except (HTTPException, MCPUpstreamAuthError, MCPOpenApiUpstreamError) as error:
        return capture.request, error
    finally:
        handler._client = original  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L629-L647
        await client.aclose()
        openapi_to_mcp_generator._request_auth_header.reset(auth_token)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L109
        openapi_to_mcp_generator._request_extra_headers.reset(extra_token)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L112
        openapi_to_mcp_generator._request_resolved_auth_headers.reset(resolved_token)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L118
    return capture.request, outcome


def _base_url(input_value: BaseUrlInput) -> JsonValue:
    url = get_base_url(
        input_value["spec"],  # noqa: E501  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L187
        input_value["spec_path"],
    )
    return _JSON.validate_python({"url": url})


def _resolve(input_value: ResolveInput) -> JsonValue:
    resolved = resolve_operation_params(
        input_value["spec_operation"],  # noqa: E501  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L265
        input_value["path_item"],  # noqa: E501  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L267
        input_value["components"],  # noqa: E501  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L268
    )
    parameters = _PARAMETERS.validate_python(resolved.get("parameters"))
    return _JSON.validate_python(
        {
            "parameters": [
                {
                    "name": parameter.get("name"),
                    "in": parameter.get("in"),
                    "description": parameter.get("description"),
                    "required": parameter.get("required"),
                    "type": _parameter_type(parameter),
                }
                for parameter in parameters
            ]
        }
    )


def _extract(input_value: OperationInput) -> JsonValue:
    path, query, body = extract_parameters(
        input_value["spec_operation"]  # noqa: E501  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L293
    )
    return _JSON.validate_python({"path": list(path), "query": list(query), "body": list(body)})


def _schema(input_value: OperationInput) -> JsonValue:
    schema = build_input_schema(
        input_value["spec_operation"]  # noqa: E501  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L319
    )
    return _JSON.validate_python(schema)


def _merge_headers(input_value: MergeHeadersInput) -> JsonValue:
    auth_token = openapi_to_mcp_generator._request_auth_header.set(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L109
        input_value["auth_header"]
    )
    extra_token = openapi_to_mcp_generator._request_extra_headers.set(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L112
        input_value["extra_headers"]
    )
    resolved_token = openapi_to_mcp_generator._request_resolved_auth_headers.set(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L118
        input_value["resolved_auth_headers"]
    )
    try:
        merged = openapi_to_mcp_generator._merge_openapi_tool_request_headers(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L393
            input_value["static_headers"]
        )
    finally:
        openapi_to_mcp_generator._request_auth_header.reset(auth_token)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L109
        openapi_to_mcp_generator._request_extra_headers.reset(extra_token)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L112
        openapi_to_mcp_generator._request_resolved_auth_headers.reset(resolved_token)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L118
    return _JSON.validate_python({"headers": merged})


def _call(input_value: CallInput) -> JsonValue:
    tool = create_tool_function(
        input_value["path"],
        input_value["method"],
        input_value["spec_operation"],  # noqa: E501  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L468
        input_value["base_url"],
        headers=input_value["headers"],
        server_label=input_value["server_label"],
        relays_upstream_auth=input_value["relays_upstream_auth"],
        auth_type=_AUTH_MODE.validate_python(input_value["auth_type"]),
        upstream_token_header=input_value["upstream_token_header"],
    )
    arguments = _ARGUMENTS.validate_python(input_value["arguments"])
    request, outcome = asyncio.run(_invoke(tool, arguments, input_value["reply"], input_value))
    return _JSON.validate_python(
        {
            "request": _request_answer(request),
            "result": _outcome_answer(outcome),
            "error": _error_answer(outcome),
        }
    )


def _operation(input_value: OperationInput) -> JsonValue:
    if input_value["operation"] == "extract_params":
        return _extract(input_value)
    return _schema(input_value)


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "tool_name":
        return _JSON.validate_python({"name": sanitize_openapi_tool_name(input_value["raw"])})
    if input_value["operation"] == "base_url":
        return _base_url(input_value)
    if input_value["operation"] == "resolve_params":
        return _resolve(input_value)
    if input_value["operation"] == "merge_headers":
        return _merge_headers(input_value)
    if input_value["operation"] == "call":
        return _call(input_value)
    return _operation(input_value)
