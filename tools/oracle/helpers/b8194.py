from typing import TYPE_CHECKING, Literal, TypeAlias, cast

import httpx
from litellm.proxy._experimental.mcp_server.mcp_context import active_mcp_request_ctx_var
from litellm.proxy._experimental.mcp_server.mcp_debug import (
    MCPAuthDiagnostics,
    MCPDebug,
    _preview,  # pyright: ignore[reportPrivateUsage]
    record_auth_resolution,
    safe_upstream_url,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.types import AuthResolution
from pydantic import BaseModel, ConfigDict, Field
from starlette.requests import HTTPConnection
from starlette.types import Scope

if TYPE_CHECKING:
    from mcp.server.context import ServerRequestContext

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Pair: TypeAlias = tuple[str, str]


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class DebugScenario(Strict):
    headers: list[Pair] = Field(default_factory=list)
    oauth2_headers: list[Pair] | None = None
    litellm_api_key: str | None = None
    auth_resolution: str
    server_url: str | None = None
    server_auth_type: str | None = None


class DiagnosticScenario(Strict):
    records: list[Pair] = Field(default_factory=list)
    through_context: bool = False


class BodyScenario(Strict):
    body: str
    content_type: str | None = None
    secrets: list[str] = Field(default_factory=list)
    url: str | None = None


class Input(Strict):
    operation: Literal["run"]
    debug: list[DebugScenario] = Field(default_factory=list)
    diagnostics: list[DiagnosticScenario] = Field(default_factory=list)
    bodies: list[BodyScenario] = Field(default_factory=list)


class FakeContext:
    def __init__(self, connection: HTTPConnection) -> None:
        self.request = connection


def _debug(scenario: DebugScenario) -> Json:
    oauth2_headers = None if scenario.oauth2_headers is None else dict(scenario.oauth2_headers)
    headers = MCPDebug.build_debug_headers(
        inbound_headers=dict(scenario.headers),
        oauth2_headers=oauth2_headers,
        litellm_api_key=scenario.litellm_api_key,
        auth_resolution=scenario.auth_resolution,
        server_url=scenario.server_url,
        server_auth_type=scenario.server_auth_type,
    )
    return cast(
        "Json",
        {
            "enabled": MCPDebug.is_debug_enabled(dict(scenario.headers)),
            "headers": [[name, value] for name, value in headers.items()],
        },
    )


def _diagnostics(scenario: DiagnosticScenario) -> Json:
    diagnostics = MCPAuthDiagnostics()
    if not scenario.through_context:
        for server_id, resolution in scenario.records:
            diagnostics.record(server_id, AuthResolution(resolution))
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
                record_auth_resolution(server_id, AuthResolution(resolution))
        finally:
            active_mcp_request_ctx_var.reset(token)
    return cast(
        "Json",
        {
            "resolution": diagnostics.resolution(),
            "headers": [[name, value] for name, value in diagnostics.headers().items()],
        },
    )


def _body(scenario: BodyScenario) -> Json:
    return cast(
        "Json",
        {
            "preview": _preview(
                scenario.body.encode(), scenario.content_type or "", tuple(scenario.secrets)
            ),
            "url": None if scenario.url is None else safe_upstream_url(httpx.URL(scenario.url)),
        },
    )


def run(input_value: dict[str, Json]) -> Json:
    parsed = Input.model_validate(input_value)
    return cast(
        "Json",
        {
            "debug": [_debug(scenario) for scenario in parsed.debug],
            "diagnostics": [_diagnostics(scenario) for scenario in parsed.diagnostics],
            "bodies": [_body(scenario) for scenario in parsed.bodies],
        },
    )
