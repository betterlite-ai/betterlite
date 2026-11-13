import asyncio
from collections.abc import Awaitable, Callable, Coroutine
from contextlib import AbstractContextManager
from types import SimpleNamespace, TracebackType
from typing import Literal, Self, TypeAlias, TypeVar, cast
from unittest.mock import patch

import anyio
import litellm
from litellm._logging import verbose_logger
from litellm.experimental_mcp_client import client as mcp_client
from litellm.types.mcp import MCPStdioConfig, MCPTransportType
from mcp.types import (
    CallToolRequestParams,
    CallToolResult,
    InputRequiredResult,
    TextContent,
)
from pydantic import BaseModel, ConfigDict, Field

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
ErrorKind = Literal["plain", "broken"]
HostKind = Literal["none", "record", "fail"]
Result = TypeVar("Result")

TAGS: tuple[tuple[str, str], ...] = (
    ("MCP client listing tools from", "listing"),
    ("MCP client listed", "listed"),
    ("list_tools was cancelled", "list_cancelled"),
    ("list_tools failed", "list_failed"),
    ("during list_tools", "list_broken"),
    ("MCP client calling tool", "calling"),
    ("progress:", "progress"),
    ("Failed to forward to Host", "host_failed"),
    ("sending tool call to session", "sending"),
    ("completed successfully", "call_completed"),
    ("tool call timed out after", "call_cancelled"),
    ("call_tool failed", "call_failed"),
    ("tool call traceback", "traceback"),
    ("detected broken connection/stream -", "call_broken"),
    ("repeated a tools/list cursor", "cursor_repeat"),
    ("pagination exceeded the maximum", "page_limit"),
    ("reached the 1,000 page limit", "page_limit"),
    ("listing deadline", "deadline"),
)


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class Settings(Strict):
    server_url: str = "https://mcp.example.com/mcp"
    timeout_ms: int = 60_000
    listing_ms: int = 30_000


class ToolSpec(Strict):
    name: str


class Frame(Strict):
    delay_ms: int = 0
    tools: list[ToolSpec] = Field(default_factory=list)
    next_cursor: str | None = None


class ListScenario(Strict):
    pages: list[Frame] = Field(default_factory=list)
    error: ErrorKind | None = None
    cancel: bool = False
    raise_on_error: bool = False


class ProgressSpec(Strict):
    progress: float = 0.0
    total: float | None = None


class CallScenario(Strict):
    tool: str = "echo"
    arguments: dict[str, object] = Field(default_factory=dict)
    result_text: str = "tool output"
    input_required: Literal["none", "accept", "reject"] = "none"
    request_state: str = "state-1"
    error: ErrorKind | None = None
    progress: list[ProgressSpec] = Field(default_factory=list)
    host: HostKind = "none"
    cancel: bool = False
    raise_on_error: bool = False


class ListInput(Strict):
    operation: Literal["list_tools"]
    settings: Settings
    scenarios: list[ListScenario] = Field(default_factory=list)


class CallInput(Strict):
    operation: Literal["call_tool"]
    settings: Settings
    scenarios: list[CallScenario] = Field(default_factory=list)


class SimulatedError(Exception):
    pass


class BrokenResourceError(Exception):
    pass


class DeadlineError(Exception):
    pass


class Budget:
    def __init__(self) -> None:
        self.elapsed = 0.0
        self.deadline = 0.0

    def __call__(self, seconds: float) -> "Budget":
        self.deadline = seconds
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


def tag(message: str) -> str | None:
    if "run_with_session failed" in message or "waiting for a valid MCP response" in message:
        return None
    for needle, name in TAGS:
        if needle in message:
            return name
    return None


class Recording:
    def __init__(self) -> None:
        self.entries: list[list[str]] = []

    def remember(self, level: str) -> Callable[..., None]:
        def record(message: object, *_args: object, **_kwargs: object) -> None:
            name = tag(str(message))
            if name is not None:
                self.entries.append([level, name])

        return record

    def install(self) -> AbstractContextManager[None]:
        return patch.multiple(
            verbose_logger,
            debug=self.remember("debug"),
            info=self.remember("info"),
            warning=self.remember("warn"),
            error=self.remember("error"),
            exception=self.remember("error"),
        )


class FakeSessionBase:
    def __init__(self, *, cancel: bool) -> None:
        self.cancel = cancel
        self.entered = asyncio.Event()

    async def initialize(self) -> SimpleNamespace:
        return SimpleNamespace(protocol_version="2025-06-18", instructions=None)

    def adopt(self, _result: object) -> None:
        return None

    async def send_notification(self, _notification: object) -> None:
        return None

    async def cancel_forever(self) -> None:
        self.entered.set()
        await asyncio.Event().wait()


class FakeListSession(FakeSessionBase):
    def __init__(self, scenario: ListScenario, budget: Budget) -> None:
        super().__init__(cancel=scenario.cancel)
        self.scenario = scenario
        self.budget = budget
        self.frames = list(scenario.pages)

    async def list_tools(self, params: object = None) -> SimpleNamespace:  # noqa: ARG002
        if self.scenario.cancel:
            await self.cancel_forever()
        match self.scenario.error:
            case "plain":
                raise SimulatedError("upstream failure: listing failed")
            case "broken":
                raise BrokenResourceError("the connection is broken")
            case None:
                pass
        frame = self.frames.pop(0)
        self.budget.elapsed += frame.delay_ms / 1000
        if self.budget.elapsed >= self.budget.deadline:
            raise DeadlineError
        tools = [SimpleNamespace(name=item.name) for item in frame.tools]
        return SimpleNamespace(tools=tools, next_cursor=frame.next_cursor)


class FakeCallSession(FakeSessionBase):
    def __init__(self, scenario: CallScenario) -> None:
        super().__init__(cancel=scenario.cancel)
        self.scenario = scenario

    async def call_tool(
        self,
        name: str,  # noqa: ARG002
        arguments: Json = None,  # noqa: ARG002
        progress_callback: Callable[..., Awaitable[None]] | None = None,
        *,
        allow_input_required: bool = False,  # noqa: ARG002
    ) -> CallToolResult | InputRequiredResult:
        if self.scenario.cancel:
            await self.cancel_forever()
        match self.scenario.error:
            case "plain":
                raise SimulatedError("upstream failure: tool crashed")
            case "broken":
                raise BrokenResourceError("the connection is broken")
            case None:
                pass
        if self.scenario.input_required == "reject":
            raise SimulatedError("the server returned an input_required result")
        if progress_callback is not None:
            for event in self.scenario.progress:
                await progress_callback(event.progress, event.total, None)
        if self.scenario.input_required == "accept":
            return InputRequiredResult(
                result_type="input_required", request_state=self.scenario.request_state
            )
        return CallToolResult(
            content=[TextContent(type="text", text=self.scenario.result_text)],
            is_error=False,
        )


class FakeContext:
    def __init__(self, session: FakeSessionBase) -> None:
        self.session = session

    async def __aenter__(self) -> FakeSessionBase:
        return self.session

    async def __aexit__(
        self,
        _kind: type[BaseException] | None,
        _value: BaseException | None,
        _trace: TracebackType | None,
    ) -> bool:
        return False

    def __call__(self, *_args: object, **_kwargs: object) -> "FakeContext":
        return self


class FakeTransport:
    async def __aenter__(self) -> tuple[object, object]:
        return (object(), object())

    async def __aexit__(
        self,
        _kind: type[BaseException] | None,
        _value: BaseException | None,
        _trace: TracebackType | None,
    ) -> bool:
        return False

    def __call__(self, _params: object) -> "FakeTransport":
        return self


class Host:
    def __init__(self, *, fail: bool) -> None:
        self.seen: list[list[Json]] = []
        self.fail = fail

    async def __call__(self, progress: float, total: float | None) -> None:
        self.seen.append([progress, total])
        if self.fail:
            raise SimulatedError("host")


def make_client(settings: Settings) -> mcp_client.MCPClient:
    return mcp_client.MCPClient(
        server_url=settings.server_url,
        transport_type=cast(MCPTransportType, "stdio"),
        timeout=settings.timeout_ms / 1000,
        stdio_config=cast(MCPStdioConfig, {"command": "server", "args": [], "env": {}}),
    )


def patched(settings: Settings, session: FakeSessionBase) -> AbstractContextManager[None]:
    return patch.multiple(
        mcp_client,
        ClientSession=FakeContext(session),
        stdio_client=FakeTransport(),
        MCP_TOOL_LISTING_TIMEOUT=settings.listing_ms / 1000,
    )


async def drive(
    coroutine: Callable[[], Coroutine[object, object, Result]], session: FakeSessionBase
) -> tuple[Literal["ok", "raised"], Result | None]:
    task = asyncio.create_task(coroutine())
    if session.cancel:
        await session.entered.wait()
        task.cancel()
    try:
        value = await task
    except asyncio.CancelledError:
        return "raised", None
    except Exception:  # noqa: BLE001 - every helper exception maps to a raised outcome
        return "raised", None
    return "ok", value


def result_value(result: CallToolResult | InputRequiredResult) -> Json:
    if isinstance(result, InputRequiredResult):
        return {
            "kind": "input_required",
            "result_type": result.result_type,
            "request_state": result.request_state,
        }
    return {
        "kind": "complete",
        "is_error": bool(result.is_error),
        "content": [
            {"type": "text", "text": block.text}
            if isinstance(block, TextContent)
            else {"type": block.type}
            for block in result.content
        ],
    }


Caller: TypeAlias = Callable[..., Coroutine[object, object, CallToolResult | InputRequiredResult]]


async def list_scenario(settings: Settings, scenario: ListScenario) -> Json:
    recording = Recording()
    budget = Budget()
    session = FakeListSession(scenario, budget)
    client = make_client(settings)
    with (
        recording.install(),
        patched(settings, session),
        patch.object(anyio, "move_on_after", budget),
    ):
        outcome, tools = await drive(
            lambda: client.list_tools(raise_on_error=scenario.raise_on_error), session
        )
    logs = cast(Json, recording.entries)
    if outcome != "ok" or tools is None:
        error = "cancelled" if scenario.cancel else "error"
        return {"outcome": "raised", "error": error, "tools": None, "logs": logs}
    return {"outcome": "ok", "tools": [tool.name for tool in tools], "logs": logs}


async def call_scenario(settings: Settings, scenario: CallScenario) -> Json:
    recording = Recording()
    session = FakeCallSession(scenario)
    client = make_client(settings)
    host = Host(fail=scenario.host == "fail") if scenario.host != "none" else None
    params = CallToolRequestParams(name=scenario.tool, arguments=scenario.arguments)
    caller = cast(Caller, client.call_tool)
    with recording.install(), patched(settings, session):
        outcome, result = await drive(
            lambda: caller(
                params,
                host_progress_callback=host,
                raise_on_error=scenario.raise_on_error,
                allow_input_required=scenario.input_required == "accept",
            ),
            session,
        )
    progress = cast(Json, host.seen if host is not None else [])
    logs = cast(Json, recording.entries)
    if outcome != "ok" or result is None:
        error = "cancelled" if scenario.cancel else "error"
        return {
            "outcome": "raised",
            "error": error,
            "result": None,
            "progress": progress,
            "logs": logs,
        }
    return {
        "outcome": "ok",
        "result": result_value(result),
        "progress": progress,
        "logs": logs,
    }


async def run_operation(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    match operation:
        case "list_tools":
            parsed = ListInput.model_validate(input_value)
            return {
                "results": [await list_scenario(parsed.settings, item) for item in parsed.scenarios]
            }
        case "call_tool":
            parsed = CallInput.model_validate(input_value)
            return {
                "results": [await call_scenario(parsed.settings, item) for item in parsed.scenarios]
            }
        case _:
            raise ValueError(f"unknown b4745 operation: {operation}")


def run(input_value: dict[str, Json]) -> Json:
    _ = litellm
    return asyncio.run(run_operation(input_value))
