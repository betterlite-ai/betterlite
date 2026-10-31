import asyncio
from types import SimpleNamespace, TracebackType
from typing import Literal, Self, TypeAlias, cast
from unittest.mock import patch

import anyio
from litellm._logging import verbose_logger
from litellm.experimental_mcp_client import tools
from litellm.types.utils import ChatCompletionMessageToolCall
from mcp import ClientSession
from mcp.types import Tool

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class DeadlineError(Exception):
    pass


class Budget:
    def __init__(self) -> None:
        self.elapsed = 0.0
        self.deadline = 0.0

    def __call__(self, seconds: float) -> "Budget":
        self.deadline = seconds * 1000
        return self

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self,
        kind: type[BaseException] | None,
        value: BaseException | None,
        trace: TracebackType | None,
    ) -> bool:
        return kind is DeadlineError


class Session:
    def __init__(self, pages: list[dict[str, Json]], budget: Budget) -> None:
        self.pages = iter(pages)
        self.budget = budget
        self.cursors: list[Json] = []

    async def list_tools(self, params: object = None) -> SimpleNamespace:
        self.cursors.append(cast(str | None, getattr(params, "cursor", None)))
        page = next(self.pages)
        self.budget.elapsed += cast(int, page.get("delay_ms", 0))
        if self.budget.elapsed >= self.budget.deadline:
            raise DeadlineError
        return SimpleNamespace(
            tools=[
                make_tool(cast(dict[str, Json], item)) for item in cast(list[Json], page["tools"])
            ],
            next_cursor=page.get("nextCursor"),
        )

    async def call_tool(self, name: str, arguments: Json) -> dict[str, Json]:
        return {"name": name, "arguments": arguments}


def make_tool(source: dict[str, Json]) -> Tool:
    return cast(
        Tool,
        SimpleNamespace(
            name=source["name"],
            description=source.get("description"),
            input_schema=source.get("inputSchema"),
            source=source,
        ),
    )


async def paginate(input_value: dict[str, Json]) -> Json:
    budget = Budget()
    pages = cast(list[dict[str, Json]], input_value.get("pages", []))
    pages = pages + [
        {"tools": [{"name": "generated", "inputSchema": {}}], "nextCursor": str(index)}
        for index in range(cast(int, input_value.get("generated_pages", 0)))
    ]
    session = Session(pages, budget)
    client = cast(int, input_value["client_ms"]) / 1000
    listing = cast(int, input_value["listing_ms"]) / 1000
    deadline = cast(int | None, input_value.get("deadline_ms"))
    with (
        patch.object(anyio, "move_on_after", budget),
        patch.object(tools, "MCP_CLIENT_TIMEOUT", client),
        patch.object(tools, "MCP_TOOL_LISTING_TIMEOUT", listing),
        patch.object(verbose_logger, "warning") as warning,
    ):
        if deadline is None:
            loaded = await tools.load_mcp_tools(
                cast(ClientSession, session),
                cast(Literal["mcp", "openai"], input_value.get("format", "mcp")),
            )
        else:
            loaded = await tools.list_tools_with_pagination(
                cast(ClientSession, session), deadline / 1000
            )
        result = cast(list[Json], loaded)
        if input_value.get("format", "mcp") == "mcp":
            result = [
                cast(
                    Json,
                    {
                        **cast(dict[str, Json], cast(SimpleNamespace, item).source),
                        "description": item.description,
                        "inputSchema": cast(Json, item.input_schema),
                    },
                )
                for item in cast(list[Tool], loaded)
            ]
        return {"tools": result, "cursors": session.cursors, "warnings": warning.call_count}


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation", "mcp_tool_schema")
    if operation == "mcp_tool_schema":
        tool = make_tool(cast(dict[str, Json], input_value["tool"]))
        return cast(
            Json,
            {
                "openai": tools.transform_mcp_tool_to_openai_tool(tool),
                "responses": tools.transform_mcp_tool_to_openai_responses_api_tool(tool),
                "anthropic": tools.transform_mcp_tool_to_anthropic_tool(tool),
            },
        )
    if operation == "call":
        return cast(
            Json,
            asyncio.run(
                tools.call_openai_tool(
                    cast(ClientSession, Session([], Budget())),
                    cast(ChatCompletionMessageToolCall, input_value["call"]),
                )
            ),
        )
    if operation == "pagination":
        return asyncio.run(paginate(input_value))
    raise ValueError(f"unknown operation: {operation}")
