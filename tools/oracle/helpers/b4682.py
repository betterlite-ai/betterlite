import asyncio
from collections.abc import Mapping
from typing import TypeAlias, cast

from litellm.proxy._experimental.mcp_server.tool_catalog_guard import (
    apply_description_overrides,
    pin_tool_catalog,
    scan_tool_descriptions,
)
from litellm.proxy._experimental.mcp_server.utils import (
    logging_safe_mcp_headers,
    strip_known_server_prefix,
)
from litellm.proxy.utils import ProxyLogging
from litellm.types.mcp_server.mcp_server_manager import MCPServer, PinnedMCPTool
from mcp.types import Tool as MCPTool

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class OutcomeProxy:
    def __init__(self, outcomes: Mapping[str, Json]) -> None:
        self.outcomes = outcomes
        self.active = 0
        self.maximum = 0

    def _convert_mcp_to_llm_format(
        self,
        request: object,
        kwargs: dict[str, object],
    ) -> dict[str, object]:
        del request
        return kwargs

    async def pre_call_hook(
        self,
        *,
        user_api_key_dict: object,
        data: dict[str, object],
        call_type: str,
        guardrails_only: bool,
    ) -> dict[str, object] | None:
        del user_api_key_dict, call_type, guardrails_only
        name = cast(str, data["name"])
        self.active += 1
        self.maximum = max(self.maximum, self.active)
        await asyncio.sleep(0.002)
        self.active -= 1
        outcome = self.outcomes.get(name)
        if isinstance(outcome, dict):
            failure = outcome.get("failure")
            if isinstance(failure, dict):
                raise failure_exception(failure)
            guarded = outcome.get("guarded")
            if isinstance(guarded, dict):
                return cast(dict[str, object], guarded)
        return None


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    if operation == "strip_prefix":
        return strip_prefix(input_value)
    if operation == "sanitize_headers":
        headers = cast(dict[str, str], input_value.get("headers", {}))
        return cast(Json, logging_safe_mcp_headers(headers))
    if operation == "pin":
        return pin(input_value)
    if operation == "override":
        return overrides(input_value)
    if operation in {"scan", "batch"}:
        return scan(input_value, batch_report=operation == "batch")
    if operation == "error":
        return error_reason(input_value)
    raise ValueError(f"unknown b4682 operation: {operation}")


def strip_prefix(input_value: dict[str, Json]) -> str:
    name = cast(str, input_value["name"])
    server_value = input_value.get("server")
    server = None if server_value is None else server_from(cast(dict[str, Json], server_value))
    return strip_known_server_prefix(name, server)


def server_from(values: dict[str, Json]) -> MCPServer:
    return MCPServer.model_construct(
        server_id=cast(str, values.get("server_id", "")),
        name=cast(str, values.get("name", "server")),
        alias=cast(str | None, values.get("alias")),
        server_name=cast(str | None, values.get("server_name")),
        extra_headers=cast(list[str] | None, values.get("extra_headers", [])),
        tool_name_to_description=cast(
            dict[str, str] | None, values.get("tool_name_to_description", {})
        ),
    )


def pin(input_value: dict[str, Json]) -> Json:
    tools = [
        MCPTool.model_validate(item)
        for item in cast(list[dict[str, Json]], input_value["tools"])
    ]
    pins = {
        name: PinnedMCPTool.model_validate(value)
        for name, value in cast(dict[str, dict[str, Json]], input_value["pinned"]).items()
    }
    served, drift = pin_tool_catalog(tools, pins)
    return cast(
        Json,
        {
            "served": [
                tool.model_dump(mode="json", by_alias=True, exclude_none=True)
                for tool in served
            ],
            "added": [] if drift is None else list(drift.added),
            "removed": [] if drift is None else list(drift.removed),
            "changed": [] if drift is None else list(drift.changed),
        },
    )


def overrides(input_value: dict[str, Json]) -> Json:
    tools = [
        MCPTool.model_validate(item)
        for item in cast(list[dict[str, Json]], input_value["tools"])
    ]
    config = cast(dict[str, str], input_value["overrides"])
    server = server_from(
        cast(
            dict[str, Json],
            {
                "server_id": "server-id",
                "name": "server",
                "alias": cast(str, input_value.get("prefix", "")),
                "tool_name_to_description": config,
            },
        )
    )
    served = apply_description_overrides(tools, server)
    return cast(
        Json,
        [tool.model_dump(mode="json", by_alias=True, exclude_none=True) for tool in served],
    )


def scan(input_value: dict[str, Json], *, batch_report: bool) -> Json:
    tools = [
        MCPTool.model_validate(item)
        for item in cast(list[dict[str, Json]], input_value.get("tools", []))
    ]
    outcomes = cast(dict[str, Json], input_value.get("outcomes", {}))
    proxy = OutcomeProxy(outcomes)
    server = server_from(cast(dict[str, Json], input_value.get("server", {})))
    result = asyncio.run(
        scan_tool_descriptions(
            tools,
            server,
            cast(ProxyLogging, proxy),
            None,
            cast(dict[str, str], input_value.get("headers", {})),
        )
    )
    encoded: dict[str, Json] = {
        "served": [
            tool.model_dump(mode="json", by_alias=True, exclude_none=True)
            for tool in result.served
        ],
        "blocked": [{"name": item.name, "reason": item.reason} for item in result.blocked],
    }
    if batch_report:
        encoded = {
            "scan": encoded,
            "batch_size": proxy.maximum,
            "order": [tool.name for tool in result.served],
        }
    return encoded


def error_reason(input_value: dict[str, Json]) -> Json:
    failure = cast(dict[str, Json], input_value["failure"])
    result = cast(
        dict[str, Json],
        scan(
            {
                "tools": [{"name": "error", "description": "text", "inputSchema": {}}],
                "outcomes": {"error": {"failure": failure}},
            },
            batch_report=False,
        ),
    )
    blocked = cast(list[dict[str, Json]], result["blocked"])
    return {"reason": blocked[0]["reason"]}


def failure_exception(failure: dict[str, Json]) -> Exception:
    error_type = cast(str, failure.get("exception_type", "Exception"))
    message = cast(str, failure.get("exception_message", ""))
    detail_error = failure.get("detail_error")
    if detail_error is not None:
        return _DetailedError(message, {"error": detail_error})
    return ValueError(message) if error_type == "ValueError" else RuntimeError(message)


class _DetailedError(Exception):
    def __init__(self, message: str, detail: Mapping[str, object]) -> None:
        super().__init__(message)
        self.detail = detail
