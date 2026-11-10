import asyncio
import json
from collections.abc import AsyncIterator, Callable
from typing import TYPE_CHECKING, Final, Literal, TypeAlias, cast

import httpx
from litellm.proxy._experimental.mcp_server import mcp_server_manager
from litellm.proxy._experimental.mcp_server.mcp_context import active_mcp_request_ctx_var
from litellm.proxy._experimental.mcp_server.mcp_debug import (
    _CAPTURE_EXTENSION,  # pyright: ignore[reportPrivateUsage]
    MCPAuthDiagnostics,
    MCPDebug,
    _masked_headers,  # pyright: ignore[reportPrivateUsage]
    _preview,  # pyright: ignore[reportPrivateUsage]
    _request_body_preview,  # pyright: ignore[reportPrivateUsage]
    _request_secret_values,  # pyright: ignore[reportPrivateUsage]
    capture_upstream_error_response,
    describe_upstream_http_failure,
    describe_upstream_response,
    record_auth_resolution,
    safe_upstream_url,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.types import AuthResolution
from pydantic import BaseModel, ConfigDict, Field
from starlette.requests import HTTPConnection
from starlette.types import Message, Scope

if TYPE_CHECKING:
    from mcp.server.context import ServerRequestContext

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Pair: TypeAlias = tuple[str, str]
Handler: TypeAlias = Callable[[dict[str, Json]], Json]
DUMMY_URL: Final = "https://placeholder.test/"


def _json(value: object) -> Json:
    return cast("Json", json.loads(json.dumps(value)))


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class ServerSpec(Strict):
    name: str
    alias: str | None = None
    url: str | None = None
    auth_type: str | None = None


class DebugScenario(Strict):
    headers: list[Pair] = Field(default_factory=list)
    oauth2_headers: list[Pair] | None = None
    litellm_api_key: str | None = None
    auth_resolution: str
    server_url: str | None = None
    server_auth_type: str | None = None
    servers: list[ServerSpec] = Field(default_factory=list)
    mcp_servers: list[str] = Field(default_factory=list)


class DebugInput(Strict):
    operation: Literal["debug_headers"]
    scenarios: list[DebugScenario] = Field(default_factory=list)


class DiagnosticScenario(Strict):
    records: list[Pair] = Field(default_factory=list)
    through_context: bool = False


class DiagnosticsInput(Strict):
    operation: Literal["auth_diagnostics"]
    scenarios: list[DiagnosticScenario] = Field(default_factory=list)


class WrapScenario(Strict):
    debug_headers: list[Pair] = Field(default_factory=list)
    records: list[Pair] = Field(default_factory=list)
    method: str = "POST"


class WrapInput(Strict):
    operation: Literal["wrap_send"]
    scenarios: list[WrapScenario] = Field(default_factory=list)


class MaskInput(Strict):
    operation: Literal["mask_secret"]
    values: list[str | None] = Field(default_factory=list)


class PreviewScenario(Strict):
    body: str
    content_type: str | None = None
    secrets: list[str] = Field(default_factory=list)


class PreviewInput(Strict):
    operation: Literal["preview"]
    scenarios: list[PreviewScenario] = Field(default_factory=list)


class RequestSpec(Strict):
    method: str
    url: str
    headers: list[Pair] = Field(default_factory=list)
    body: str | None = None


class RequestInput(Strict):
    operation: Literal["request_secrets"]
    scenarios: list[RequestSpec] = Field(default_factory=list)


class ResponseSpec(Strict):
    status: int
    headers: list[Pair] = Field(default_factory=list)
    body: str | None = None
    captured: str | None = None


class DescribeScenario(Strict):
    request: RequestSpec | None = None
    response: ResponseSpec


class DescribeInput(Strict):
    operation: Literal["describe_upstream"]
    urls: list[str] = Field(default_factory=list)
    scenarios: list[DescribeScenario] = Field(default_factory=list)


class FailureInput(Strict):
    operation: Literal["describe_failure"]
    responses: list[ResponseSpec] = Field(default_factory=list)
    chains: list[list[int | None]] = Field(default_factory=list)


class CaptureScenario(Strict):
    status: int
    headers: list[Pair] = Field(default_factory=list)
    body: str
    request: RequestSpec | None = None
    chunk_size: int | None = None
    fail_after_chunks: int | None = None


class CaptureInput(Strict):
    operation: Literal["capture"]
    scenarios: list[CaptureScenario] = Field(default_factory=list)


class FakeServer:
    def __init__(self, spec: ServerSpec) -> None:
        self.name = spec.name
        self.alias = spec.alias
        self.url = spec.url
        self.auth_type = spec.auth_type


class FakeManager:
    def __init__(self, specs: list[ServerSpec]) -> None:
        self.servers = [FakeServer(spec) for spec in specs]

    def get_mcp_server_by_name(self, name: str, client_ip: str | None = None) -> FakeServer | None:
        _unused = client_ip
        for server in self.servers:
            if name in (server.name, server.alias):
                return server
        return None


class FakeContext:
    def __init__(self, connection: HTTPConnection) -> None:
        self.request = connection


class ChunkStream(httpx.AsyncByteStream):
    def __init__(self, body: bytes, chunk_size: int, fail_after: int | None) -> None:
        self.body = body
        self.chunk_size = chunk_size
        self.fail_after = fail_after

    async def __aiter__(self) -> AsyncIterator[bytes]:
        offsets = range(0, len(self.body), self.chunk_size)
        for index, offset in enumerate(offsets):
            if self.fail_after is not None and index >= self.fail_after:
                raise httpx.StreamError("the stream broke")
            yield self.body[offset : offset + self.chunk_size]


def _latin1(pairs: list[Pair]) -> list[tuple[bytes, bytes]]:
    return [(name.encode("latin-1"), value.encode("latin-1")) for name, value in pairs]


def _resolution(name: str) -> AuthResolution:
    return AuthResolution(name)


def _diagnostics(records: list[Pair]) -> MCPAuthDiagnostics:
    diagnostics = MCPAuthDiagnostics()
    for server_id, resolution in records:
        diagnostics.record(server_id, _resolution(resolution))
    return diagnostics


def _debug_headers(scenario: DebugScenario) -> Json:
    raw_headers = dict(scenario.headers)
    oauth2_headers = None if scenario.oauth2_headers is None else dict(scenario.oauth2_headers)
    headers = MCPDebug.build_debug_headers(
        inbound_headers=raw_headers,
        oauth2_headers=oauth2_headers,
        litellm_api_key=scenario.litellm_api_key,
        auth_resolution=scenario.auth_resolution,
        server_url=scenario.server_url,
        server_auth_type=scenario.server_auth_type,
    )
    scope: dict[str, object] = {
        "type": "http",
        "method": "POST",
        "headers": _latin1(scenario.headers),
    }
    original = mcp_server_manager.global_mcp_server_manager
    setattr(mcp_server_manager, "global_mcp_server_manager", FakeManager(scenario.servers))  # noqa: B010
    try:
        maybe = MCPDebug.maybe_build_debug_headers(  # pyright: ignore[reportUnknownMemberType]
            raw_headers=raw_headers,
            scope=cast("dict[str, Json]", scope),
            mcp_servers=scenario.mcp_servers,
            oauth2_headers=oauth2_headers,
            client_ip=None,
        )
    finally:
        setattr(mcp_server_manager, "global_mcp_server_manager", original)  # noqa: B010
    return {
        "enabled": MCPDebug.is_debug_enabled(raw_headers),
        "headers": [[name, value] for name, value in headers.items()],
        "maybe_headers": None if not maybe else [[name, value] for name, value in maybe.items()],
    }


def _auth_diagnostics(scenario: DiagnosticScenario) -> Json:
    diagnostics = MCPAuthDiagnostics()
    if not scenario.through_context:
        for server_id, resolution in scenario.records:
            diagnostics.record(server_id, _resolution(resolution))
    else:
        scope: dict[str, object] = {
            "type": "http",
            "headers": [],
            "litellm.mcp.auth_diagnostics": diagnostics,
        }
        connection = HTTPConnection(cast(Scope, scope))
        context = cast("ServerRequestContext", FakeContext(connection))
        token = active_mcp_request_ctx_var.set(context)
        try:
            for server_id, resolution in scenario.records:
                record_auth_resolution(server_id, _resolution(resolution))
        finally:
            active_mcp_request_ctx_var.reset(token)
    return {
        "resolution": diagnostics.resolution(),
        "headers": [[name, value] for name, value in diagnostics.headers().items()],
    }


async def _sent_headers(scenario: WrapScenario) -> list[list[str]]:
    diagnostics = _diagnostics(scenario.records)
    sent: list[list[str]] = []

    async def send(message: Message) -> None:
        headers = cast("list[tuple[bytes, bytes]]", dict(message).get("headers", []))
        sent.extend([[key.decode(), value.decode()] for key, value in headers])

    wrapped = MCPDebug.wrap_send_with_debug_headers(
        send,
        dict(scenario.debug_headers),
        diagnostics.headers,
        request_method=scenario.method,
    )
    start: Message = {"type": "http.response.start", "status": 200, "headers": []}
    await wrapped(start)
    body: Message = {"type": "http.response.body", "body": b""}
    await wrapped(body)
    return sent


def _run_wrap(value: dict[str, Json]) -> Json:
    parsed = WrapInput.model_validate(value)
    results = [{"headers": asyncio.run(_sent_headers(scenario))} for scenario in parsed.scenarios]
    return _json({"results": results})


def _run_mask(value: dict[str, Json]) -> Json:
    parsed = MaskInput.model_validate(value)
    return {"masks": [MCPDebug.mask_secret(item) for item in parsed.values]}


def _run_preview(value: dict[str, Json]) -> Json:
    parsed = PreviewInput.model_validate(value)
    return {
        "previews": [
            _preview(scenario.body.encode(), scenario.content_type or "", tuple(scenario.secrets))
            for scenario in parsed.scenarios
        ]
    }


def _request(spec: RequestSpec) -> httpx.Request:
    headers = list(spec.headers)
    if spec.body is None:
        return httpx.Request(spec.method, spec.url, headers=headers, stream=httpx.ByteStream(b""))
    return httpx.Request(spec.method, spec.url, headers=headers, content=spec.body.encode())


def _response(spec: ResponseSpec, request: httpx.Request | None) -> httpx.Response:
    if spec.body is None:
        response = httpx.Response(
            spec.status,
            headers=list(spec.headers),
            stream=httpx.ByteStream(b""),
            request=request,
        )
    else:
        response = httpx.Response(
            spec.status,
            headers=list(spec.headers),
            content=spec.body.encode(),
            request=request,
        )
    if spec.captured is not None:
        response.extensions[_CAPTURE_EXTENSION] = spec.captured
    return response


def _run_request_secrets(value: dict[str, Json]) -> Json:
    parsed = RequestInput.model_validate(value)
    results: list[Json] = []
    for scenario in parsed.scenarios:
        request = _request(scenario)
        secrets = _request_secret_values(request)
        ordered = None if secrets is None else sorted(secrets, key=lambda item: (-len(item), item))
        results.append(
            _json(
                {
                    "secrets": ordered,
                    "request_preview": _request_body_preview(request, secrets),
                    "masked_headers": _masked_headers(request.headers),
                }
            )
        )
    return _json({"results": results})


def _describe(scenario: DescribeScenario) -> str:
    request = None if scenario.request is None else _request(scenario.request)
    return describe_upstream_response(_response(scenario.response, request))


def _run_describe(value: dict[str, Json]) -> Json:
    parsed = DescribeInput.model_validate(value)
    return {
        "urls": [safe_upstream_url(httpx.URL(url)) for url in parsed.urls],
        "descriptions": [_describe(scenario) for scenario in parsed.scenarios],
    }


def _failure_chain(entries: list[int | None], responses: list[httpx.Response]) -> BaseException:
    cause: BaseException | None = None
    for entry in reversed(entries):
        current: BaseException
        if entry is None:
            current = RuntimeError("no response")
        else:
            current = httpx.HTTPStatusError(
                "failed",
                request=httpx.Request("GET", DUMMY_URL),
                response=responses[entry],
            )
        current.__cause__ = cause
        cause = current
    return cause if cause is not None else RuntimeError("empty chain")


def _run_describe_failure(value: dict[str, Json]) -> Json:
    parsed = FailureInput.model_validate(value)
    responses = [_response(spec, None) for spec in parsed.responses]
    descriptions = [
        describe_upstream_http_failure(_failure_chain(chain, responses)) for chain in parsed.chains
    ]
    return _json({"descriptions": descriptions})


async def _capture_scenario(scenario: CaptureScenario) -> Json:
    request = None if scenario.request is None else _request(scenario.request)
    body = scenario.body.encode()
    chunk_size = scenario.chunk_size if scenario.chunk_size is not None else max(len(body), 1)
    stream = ChunkStream(body, chunk_size, scenario.fail_after_chunks)
    response = httpx.Response(
        scenario.status,
        headers=list(scenario.headers),
        stream=stream,
        request=request,
    )
    await capture_upstream_error_response(response)
    captured = cast(str | None, response.extensions.get(_CAPTURE_EXTENSION))
    try:
        content = response.content.decode("utf-8", "replace")
    except httpx.ResponseNotRead:
        content = None
    return {"captured": captured, "body": content}


async def _capture_all(scenarios: list[CaptureScenario]) -> list[Json]:
    return [await _capture_scenario(scenario) for scenario in scenarios]


def _run_capture(value: dict[str, Json]) -> Json:
    parsed = CaptureInput.model_validate(value)
    return {"results": asyncio.run(_capture_all(parsed.scenarios))}


def _run_debug(value: dict[str, Json]) -> Json:
    parsed = DebugInput.model_validate(value)
    return {"results": [_debug_headers(scenario) for scenario in parsed.scenarios]}


def _run_diagnostics(value: dict[str, Json]) -> Json:
    parsed = DiagnosticsInput.model_validate(value)
    return {"results": [_auth_diagnostics(scenario) for scenario in parsed.scenarios]}


HANDLERS: Final[dict[str, Handler]] = {
    "debug_headers": _run_debug,
    "auth_diagnostics": _run_diagnostics,
    "wrap_send": _run_wrap,
    "mask_secret": _run_mask,
    "preview": _run_preview,
    "request_secrets": _run_request_secrets,
    "describe_upstream": _run_describe,
    "describe_failure": _run_describe_failure,
    "capture": _run_capture,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    handler = HANDLERS.get(operation) if isinstance(operation, str) else None
    if handler is None:
        raise ValueError(f"unknown b8194 operation: {operation}")
    return handler(input_value)
