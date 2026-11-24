import asyncio
from collections.abc import Awaitable
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, cast

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

_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
_AUTH_MODE: Final[TypeAdapter[MCPAuthType]] = TypeAdapter(MCPAuthType)
_ARGUMENTS: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
_SKIPPED_HEADERS: Final[frozenset[str]] = frozenset(
    {"host", "accept", "accept-encoding", "connection",
     "user-agent", "content-length", "content-type"},
)

Failure: TypeAlias = HTTPException | MCPUpstreamAuthError | MCPOpenApiUpstreamError


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


CallInput = TypedDict(
    "CallInput",
    {
        "operation": Literal["call"], "path": str, "method": str, "base_url": str,
        "spec_operation": JsonValue, "headers": dict[str, str],
        "server_label": str | None, "relays_upstream_auth": bool,
        "auth_type": str | None, "upstream_token_header": str | None,
        "arguments": JsonValue, "auth_header": str | None,
        "extra_headers": dict[str, str], "resolved_auth_headers": dict[str, str],
        "reply": Reply,
    },
)

SchemaOut = TypedDict("SchemaOut", {"type": str, "properties": dict[str, JsonValue]}, total=False)
ParameterOut = TypedDict(
    "ParameterOut",
    {"in": str, "name": str, "description": str, "required": bool, "schema": SchemaOut},
    total=False,
)
ResultOut = TypedDict("ResultOut", {"kind": str, "text": str, "value": JsonValue}, total=False)
ErrorOut = TypedDict(
    "ErrorOut",
    {"kind": str, "status": int, "www_authenticate": str | None, "server": str},
    total=False,
)

_PARAMETERS: Final = TypeAdapter(list[ParameterOut])

Input: TypeAlias = (
    ToolNameInput | BaseUrlInput | ResolveInput | OperationInput | MergeHeadersInput | CallInput
)


class ToolCall(Protocol):
    def __call__(self, **arguments: JsonValue) -> Awaitable[TextResult | JsonResult]: ...


class Capture:
    def __init__(self, reply: Reply) -> None:
        self.reply = reply
        self.request: httpx.Request | None = None

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.request = request
        return httpx.Response(
            self.reply["status"], headers=self.reply["headers"],
            text=self.reply["body"], request=request,
        )


def _parameter_json(parameter: ParameterOut) -> dict[str, JsonValue]:
    schema = parameter.get("schema")
    return {
        "name": parameter.get("name"),
        "in": parameter.get("in"),
        "description": parameter.get("description"),
        "required": parameter.get("required"),
        "type": None if schema is None else schema.get("type"),
    }


def _request_answer(request: httpx.Request | None) -> dict[str, JsonValue] | None:
    if request is None:
        return None
    body: JsonValue = _JSON.validate_json(request.content) if request.content else None
    query: list[list[str]] = [list(pair) for pair in request.url.params.multi_items()]
    headers: dict[str, str] = {
        name.lower(): value
        for name, value in request.headers.multi_items()
        if name.lower() not in _SKIPPED_HEADERS
    }
    return {
        "method": request.method,
        "url": str(request.url.copy_with(query=None)),
        "query": _JSON.validate_python(query),
        "headers": _JSON.validate_python(headers),
        "body": body,
    }


def _answers(
    outcome: TextResult | JsonResult | None, failure: Failure | None
) -> tuple[JsonValue, JsonValue]:
    result: JsonValue = None
    if isinstance(outcome, TextResult):
        text: ResultOut = ResultOut(kind="text", text=outcome.text)
        result = _JSON.validate_python(text)
    elif isinstance(outcome, JsonResult):
        value: ResultOut = ResultOut(kind="json", text=outcome.original_text, value=outcome.value)
        result = _JSON.validate_python(value)
    error: JsonValue = None
    if isinstance(failure, MCPUpstreamAuthError):
        challenge: ErrorOut = {
            "kind": "upstream_auth", "status": failure.status_code,
            "www_authenticate": failure.www_authenticate, "server": failure.server_name,
        }
        error = _JSON.validate_python(challenge)
    elif isinstance(failure, MCPOpenApiUpstreamError):
        status: ErrorOut = {
            "kind": "upstream_status", "status": failure.status_code, "server": failure.server_name,
        }
        error = _JSON.validate_python(status)
    elif failure is not None:
        credential: ErrorOut = {"kind": "credential", "status": failure.status_code}
        error = _JSON.validate_python(credential)
    return result, error


async def _invoke(
    tool: ToolCall,
    arguments: dict[str, JsonValue],
    reply: Reply,
    call: CallInput,
) -> tuple[httpx.Request | None, TextResult | JsonResult | None, Failure | None]:
    capture = Capture(reply)
    client = httpx.AsyncClient(transport=httpx.MockTransport(capture.handle))
    handler = openapi_to_mcp_generator._upstream_client()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L378
    original = handler.client
    auth_token = openapi_to_mcp_generator._request_auth_header.set(call["auth_header"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L109
    extra_token = openapi_to_mcp_generator._request_extra_headers.set(call["extra_headers"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L112
    resolved_token = openapi_to_mcp_generator._request_resolved_auth_headers.set(call["resolved_auth_headers"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L118
    handler._client = client  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L629-L647
    try:
        outcome = await tool(**arguments)
    except (HTTPException, MCPUpstreamAuthError, MCPOpenApiUpstreamError) as failure:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L442-L465
        return capture.request, None, failure
    finally:
        handler._client = original  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L629-L647
        await client.aclose()
        openapi_to_mcp_generator._request_auth_header.reset(auth_token)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L109
        openapi_to_mcp_generator._request_extra_headers.reset(extra_token)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L112
        openapi_to_mcp_generator._request_resolved_auth_headers.reset(resolved_token)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L118
    return capture.request, outcome, None


def _base_url(input_value: BaseUrlInput) -> JsonValue:
    url = get_base_url(input_value["spec"], input_value["spec_path"])  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L187
    answer: dict[str, JsonValue] = {"url": url}
    return _JSON.validate_python(answer)


def _resolve(input_value: ResolveInput) -> JsonValue:
    resolved = resolve_operation_params(
        input_value["spec_operation"],  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L265
        input_value["path_item"],  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L267
        input_value["components"],  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L268
    )
    parameters = _PARAMETERS.validate_python(resolved.get("parameters"))
    views = [_parameter_json(parameter) for parameter in parameters]
    answer: dict[str, JsonValue] = {"parameters": _JSON.validate_python(views)}
    return _JSON.validate_python(answer)


def _extract(input_value: OperationInput) -> JsonValue:
    path, query, body = extract_parameters(input_value["spec_operation"])  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L293
    answer: dict[str, JsonValue] = {
        "path": _JSON.validate_python(list(path)),
        "query": _JSON.validate_python(list(query)),
        "body": _JSON.validate_python(list(body)),
    }
    return _JSON.validate_python(answer)


def _schema(input_value: OperationInput) -> JsonValue:
    return _JSON.validate_python(build_input_schema(input_value["spec_operation"]))  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L319


def _merge_headers(input_value: MergeHeadersInput) -> JsonValue:
    auth_token = openapi_to_mcp_generator._request_auth_header.set(input_value["auth_header"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L109
    extra_token = openapi_to_mcp_generator._request_extra_headers.set(input_value["extra_headers"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L112
    resolved_token = openapi_to_mcp_generator._request_resolved_auth_headers.set(input_value["resolved_auth_headers"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L118
    try:
        merged = openapi_to_mcp_generator._merge_openapi_tool_request_headers(input_value["static_headers"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L393
    finally:
        openapi_to_mcp_generator._request_auth_header.reset(auth_token)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L109
        openapi_to_mcp_generator._request_extra_headers.reset(extra_token)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L112
        openapi_to_mcp_generator._request_resolved_auth_headers.reset(resolved_token)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L118
    answer: dict[str, JsonValue] = {"headers": merged}
    return _JSON.validate_python(answer)


def _call(input_value: CallInput) -> JsonValue:
    tool = cast(ToolCall, create_tool_function(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L468
        input_value["path"],
        input_value["method"],
        input_value["spec_operation"],  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/openapi_to_mcp_generator.py#L468
        input_value["base_url"],
        headers=input_value["headers"],
        server_label=input_value["server_label"],
        relays_upstream_auth=input_value["relays_upstream_auth"],
        auth_type=_AUTH_MODE.validate_python(input_value["auth_type"]),
        upstream_token_header=input_value["upstream_token_header"],
    ))
    arguments = _ARGUMENTS.validate_python(input_value["arguments"])
    request, outcome, failure = asyncio.run(
        _invoke(tool, arguments, input_value["reply"], input_value)
    )
    result, error = _answers(outcome, failure)
    answer: dict[str, JsonValue] = {
        "request": _request_answer(request),
        "result": result,
        "error": error,
    }
    return _JSON.validate_python(answer)


def _operation(input_value: OperationInput) -> JsonValue:
    if input_value["operation"] == "extract_params":
        return _extract(input_value)
    return _schema(input_value)


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "tool_name":
        answer: dict[str, JsonValue] = {"name": sanitize_openapi_tool_name(input_value["raw"])}
        return _JSON.validate_python(answer)
    if input_value["operation"] == "base_url":
        return _base_url(input_value)
    if input_value["operation"] == "resolve_params":
        return _resolve(input_value)
    if input_value["operation"] == "merge_headers":
        return _merge_headers(input_value)
    if input_value["operation"] == "call":
        return _call(input_value)
    return _operation(input_value)
