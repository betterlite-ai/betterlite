from __future__ import annotations

import asyncio
import datetime as dt
import logging
from collections.abc import Callable
from typing import TYPE_CHECKING, Final, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm._logging import verbose_logger
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils import litellm_logging
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.mcp import MCPPostCallResponseObject
from mcp.types import CallToolResult, TextContent

if TYPE_CHECKING:
    from mcp.types import ContentBlock, EmbeddedResource, ImageContent

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
Callbacks: TypeAlias = list[str | Callable[..., object] | CustomLogger]

START_TIME: Final = dt.datetime.fromisoformat("2026-01-01T00:00:00+00:00")
END_TIME: Final = dt.datetime.fromisoformat("2026-01-01T00:00:01+00:00")
NON_BLOCKING: Final = "LiteLLM.LoggingError: [Non-Blocking] Exception occurred while logging"


class InputError(ValueError):
    pass


class _Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.ERROR)
        self.lines = 0

    def emit(self, record: logging.LogRecord) -> None:
        if record.getMessage().startswith(NON_BLOCKING):
            self.lines += 1


def _object(value: Json, name: str) -> Fields:
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    return cast("Fields", value)


def _items(value: Json, name: str) -> list[Json]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise InputError(f"{name} must be a list")
    return cast("list[Json]", value)


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise InputError(f"{name} must be a string")
    return value


def _number(value: Json, name: str) -> float | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise InputError(f"{name} must be a number")
    return float(value)


def _content(value: Json, name: str) -> TextContent:
    fields = _object(value, name)
    kind = _text(fields.get("type"), f"{name}.type")
    if kind != "text":
        raise InputError(f"{name}.type must be text")
    return TextContent(type="text", text=_text(fields.get("text"), f"{name}.text"))


class _Scripted(CustomLogger):
    def __init__(self, wire: Fields) -> None:
        super().__init__()  # pyright: ignore[reportUnknownMemberType]
        self.name = _text(wire.get("name"), "name")
        self.behavior = _text(wire.get("behavior"), "behavior")
        self.content = [
            _content(item, "content item") for item in _items(wire.get("content"), "content")
        ]
        self.response_cost = _number(wire.get("response_cost"), "response_cost")
        self.structured = wire.get("structured_content")

    async def async_post_mcp_tool_call_hook(
        self,
        kwargs: dict[str, Json],
        response_obj: MCPPostCallResponseObject,
        start_time: dt.datetime,
        end_time: dt.datetime,
    ) -> MCPPostCallResponseObject | None:
        del start_time, end_time
        if self.behavior == "none":
            return None
        if self.behavior == "edit_in_place":
            response_obj.mcp_tool_call_response.append(TextContent(type="text", text="edited"))
            return None
        if self.behavior == "replace":
            hidden = response_obj.hidden_params
            if self.response_cost is not None:
                hidden.response_cost = self.response_cost
            return MCPPostCallResponseObject(
                mcp_tool_call_response=cast(
                    "list[TextContent | ImageContent | EmbeddedResource]", list(self.content)
                ),
                hidden_params=hidden,
            )
        if self.behavior == "replace_unchanged":
            return response_obj
        if self.behavior == "fail":
            raise RuntimeError("logger exploded")
        if self.behavior in ("rewrite_live", "rewrite_live_then_fail"):
            original = kwargs.get("original_response")
            if not isinstance(original, CallToolResult):
                raise InputError("the live result is missing from kwargs")
            original.content[:] = list(self.content)
            original.structured_content = self.structured
            if self.behavior == "rewrite_live_then_fail":
                raise RuntimeError("logger exploded")
            return None
        raise InputError(f"unknown behavior {self.behavior}")


def _result(wire: Fields) -> CallToolResult:
    content = [_content(item, "content item") for item in _items(wire.get("content"), "content")]
    result = CallToolResult(content=cast("list[ContentBlock]", content))
    result.structured_content = wire.get("structured_content")
    result.is_error = wire.get("is_error") is True
    return result


def _global_loggers(value: Fields) -> list[_Scripted]:
    scripted: list[_Scripted] = []
    shared: dict[str, _Scripted] = {}
    for item in _items(value.get("global_success"), "global_success"):
        wire = _object(item, "global_success")
        name = _text(wire.get("name"), "name")
        logger = shared.get(name)
        if logger is None:
            logger = _Scripted(wire)
            shared[name] = logger
        scripted.append(logger)
    return scripted


def _build(value: Fields) -> Logging:
    dynamic = [
        _text(item, "dynamic_success")
        for item in _items(value.get("dynamic_success"), "dynamic_success")
    ]
    scripted = _global_loggers(value)
    litellm.success_callback = cast("Callbacks", list(scripted))
    logging_obj = Logging(
        model="gpt-4o",
        messages=None,
        stream=None,
        call_type="call_mcp_tool",
        start_time=START_TIME,
        litellm_call_id="call-1",
        function_id="fn-1",
        litellm_trace_id="trace-1",
        dynamic_success_callbacks=cast("Callbacks", dynamic),
        kwargs={},
    )
    result = _result(_object(value.get("result"), "result"))
    logging_obj.model_call_details["original_response"] = result
    return logging_obj


def _output(result: CallToolResult, logging_obj: Logging, error_lines: int) -> Fields:
    content = [
        cast("Json", item.model_dump(mode="json", by_alias=True, exclude_none=True))
        for item in result.content
    ]
    output: Fields = {
        "content": content,
        "structured_content": cast("Json", result.structured_content),
        "is_error": result.is_error,
        "error_lines": error_lines,
    }
    details = logging_obj.model_call_details
    if "response_cost" in details:
        output["response_cost"] = cast("Json", details["response_cost"])
    return output


def _identity(value: Json) -> Json:
    return value


def run(value: Fields) -> Fields:
    capture = _Capture()
    previous_level = verbose_logger.level
    previous_handlers = verbose_logger.handlers
    previous_global = cast("Callbacks", litellm.success_callback)
    verbose_logger.handlers = [capture]
    verbose_logger.setLevel(logging.ERROR)
    try:
        with (
            patch.object(litellm_logging, "get_litellm_params", return_value={}),
            patch.object(litellm_logging, "scrub_sensitive_keys_in_metadata", _identity),
            patch.object(
                litellm_logging, "_initialize_standard_callback_dynamic_params", return_value={}
            ),
            patch.object(litellm_logging, "get_trusted_callback_params", return_value=()),
            patch.object(Logging, "process_dynamic_callbacks", return_value=None),
            patch.object(Logging, "initialize_standard_built_in_tools_params", return_value={}),
        ):
            logging_obj = _build(value)
            final = asyncio.run(
                logging_obj.async_post_mcp_tool_call_hook(  # pyright: ignore[reportUnknownMemberType]
                    kwargs=cast("dict[str, Json]", logging_obj.model_call_details),
                    response_obj=cast(
                        "CallToolResult", logging_obj.model_call_details["original_response"]
                    ),
                    start_time=START_TIME,
                    end_time=END_TIME,
                )
            )
        return _output(final, logging_obj, capture.lines)
    finally:
        verbose_logger.handlers = previous_handlers
        verbose_logger.setLevel(previous_level)
        litellm.success_callback = previous_global
