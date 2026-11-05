import asyncio
import sys
import traceback
from collections.abc import Callable
from types import SimpleNamespace
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

from litellm._logging import verbose_logger
from litellm.responses.mcp import mcp_streaming_iterator as streaming
from litellm.types.llms.openai import ResponsesAPIResponse, ResponsesAPIStreamingResponse
from pydantic import BaseModel

if TYPE_CHECKING:
    from mcp.types import Tool as MCPTool

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Diagnostics: TypeAlias = list[dict[str, Json]]


def _dump(value: BaseModel) -> Json:
    return cast("Json", value.model_dump(mode="json"))


class _FixedIds:
    def __init__(self, ids: list[str]) -> None:
        self._remaining = list(ids)

    def uuid4(self) -> SimpleNamespace:
        return SimpleNamespace(hex=self._remaining.pop(0))


class _Tool:
    def __init__(self, spec: dict[str, Json]) -> None:
        self._spec = spec

    def __getattr__(self, name: str) -> object:
        raises = cast("dict[str, str]", self._spec.get("raises", {}))
        if name in raises:
            raise ValueError(raises[name])
        if name not in self._spec:
            raise AttributeError(name)
        return self._spec[name]


def _output_items_and_function_call_id(value: dict[str, Json]) -> Json:
    response = ResponsesAPIResponse.model_validate(
        {"id": "resp_1", "created_at": 1, "output": value["output"]}
    )
    items = streaming._output_items(response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return [streaming._function_call_id(item) for item in items]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def _sample_event(ids: list[str]) -> BaseModel:
    with patch.object(streaming, "uuid", _FixedIds(ids)):
        events = streaming.create_mcp_call_events("w", None, "{}", None, "mcp_x", 1, 1)
    return cast("BaseModel", events[0])


def _set_event_field(value: dict[str, Json]) -> Json:
    event = _sample_event([])
    streaming._set_event_field(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        cast("ResponsesAPIStreamingResponse", event), cast("str", value["field"]), value["value"]
    )
    return _dump(event)


def _list_tools(value: dict[str, Json]) -> Json:
    diagnostics: Diagnostics = []

    def log(level: str) -> Callable[..., None]:
        def record(message: str, *args: object, **_kwargs: object) -> None:
            diagnostics.append({"level": level, "message": message % args if args else message})

        return record

    def record_traceback() -> None:
        diagnostics.append({"level": "traceback", "message": str(sys.exc_info()[1])})

    tools = [_Tool(spec) for spec in cast("list[dict[str, Json]]", value["tools"])]
    servers = cast("list[dict[str, object]]", value["servers"])
    with (
        patch.object(verbose_logger, "debug", log("debug")),
        patch.object(verbose_logger, "error", log("error")),
        patch.object(traceback, "print_exc", record_traceback),
    ):
        events = asyncio.run(
            streaming.create_mcp_list_tools_events(
                servers, None, cast("str", value["base_item_id"]), cast("list[MCPTool]", tools)
            )
        )
    dumped = [_dump(cast("BaseModel", event)) for event in events]
    return {"events": dumped, "diagnostics": cast("Json", diagnostics)}


def _mcp_call(value: dict[str, Json]) -> Json:
    with patch.object(streaming, "uuid", _FixedIds(cast("list[str]", value.get("ids", [])))):
        events = streaming.create_mcp_call_events(
            cast("str", value["tool_name"]),
            None,
            cast("str", value["arguments"]),
            cast("str | None", value.get("result")),
            cast("str | None", value.get("base_item_id")),
            cast("int", value.get("sequence_start", 1)),
            cast("int", value.get("output_index", 0)),
        )
    return {"events": [_dump(cast("BaseModel", event)) for event in events], "diagnostics": []}


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "output_items_and_function_call_id": _output_items_and_function_call_id,
    "set_event_field": _set_event_field,
    "list_tools": _list_tools,
    "mcp_call": _mcp_call,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = cast("str", input_value["operation"])
    if operation not in HANDLERS:
        raise ValueError(f"unsupported operation {operation}")
    return HANDLERS[operation](input_value)
