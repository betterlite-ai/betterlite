import logging
import traceback
from datetime import datetime
from typing import Final, Unpack

from pydantic import JsonValue

from betterlite._integrations import custom_logger_params as param_types
from betterlite._integrations.custom_logger_types import (
    AgenticLoopPlan,
    AsyncInputEventCallback,
    InputEventCallback,
    MCPPostCallResponseObject,
    VerboseFn,
)
from betterlite.types.litellm_params_state import JsonMap

LOGGER: Final = logging.getLogger("LiteLLM")


class CustomLoggerEvents:
    def log_input_event(
        self,
        model: str,
        messages: list[JsonValue],
        kwargs: dict[str, JsonValue],
        print_verbose: VerboseFn,
        callback_func: InputEventCallback,
    ) -> None:
        try:
            kwargs["model"] = model
            kwargs["messages"] = messages
            kwargs["log_event_type"] = "pre_api_call"
            _ = callback_func(kwargs)
            print_verbose(f"Custom Logger - model call details: {kwargs}")
        except Exception:
            print_verbose(f"Custom Logger Error - {traceback.format_exc()}")
            LOGGER.debug("Custom Logger Error", exc_info=True)

    async def async_log_input_event(
        self,
        model: str,
        messages: list[JsonValue],
        kwargs: dict[str, JsonValue],
        print_verbose: VerboseFn,
        callback_func: AsyncInputEventCallback,
    ) -> None:
        try:
            kwargs.update({"model": model, "messages": messages, "log_event_type": "pre_api_call"})
            _ = await callback_func(kwargs)
            print_verbose("Custom Logger - model call details: " + str(kwargs))
        except Exception:
            print_verbose("Custom Logger Error - " + traceback.format_exc())
            LOGGER.debug("Custom Logger Error", exc_info=True)

    def log_event(self, **params: Unpack[param_types.EventParams]) -> None:
        try:
            params["kwargs"]["log_event_type"] = "post_api_call"
            _ = params["callback_func"](
                params["kwargs"], params["response_obj"], params["start_time"], params["end_time"]
            )
        except Exception:
            params["print_verbose"](f"Custom Logger Error - {traceback.format_exc()}")
            LOGGER.debug("Custom Logger Error", exc_info=True)

    async def async_log_event(self, **params: Unpack[param_types.AsyncEventParams]) -> None:
        try:
            params["kwargs"].update({"log_event_type": "post_api_call"})
            _ = await params["callback_func"](
                params["kwargs"], params["response_obj"], params["start_time"], params["end_time"]
            )
        except Exception:
            params["print_verbose"]("Custom Logger Error - " + traceback.format_exc())
            LOGGER.debug("Custom Logger Error", exc_info=True)

    async def async_post_mcp_tool_call_hook(
        self,
        kwargs: JsonMap,
        response_obj: MCPPostCallResponseObject,
        start_time: datetime,
        end_time: datetime,
    ) -> MCPPostCallResponseObject | None: ...
    async def async_should_run_agentic_loop(
        self, **params: Unpack[param_types.AgenticGateParams]
    ) -> tuple[bool, dict[str, JsonValue]]:
        _ = params
        return False, {}

    async def async_run_agentic_loop(
        self, **params: Unpack[param_types.AgenticRunParams]
    ) -> JsonValue: ...
    async def async_build_agentic_loop_plan(
        self, **params: Unpack[param_types.AgenticRunParams]
    ) -> AgenticLoopPlan:
        _ = params
        return AgenticLoopPlan(run_agentic_loop=False)

    # ast-grep-ignore: no-long-names
    async def async_post_agentic_loop_response_hook(
        self, response: JsonValue, plan: AgenticLoopPlan, kwargs: JsonMap
    ) -> JsonValue:
        _ = (plan, kwargs)
        return response

    async def async_agentic_loop_cleanup_hook(  # ast-grep-ignore: no-long-names
        self, plan: AgenticLoopPlan, kwargs: JsonMap
    ) -> None: ...
    # ast-grep-ignore: no-long-names
    async def async_should_run_chat_completion_agentic_loop(
        self, **params: Unpack[param_types.AgenticGateParams]
    ) -> tuple[bool, dict[str, JsonValue]]:
        _ = params
        return False, {}

    async def async_run_chat_completion_agentic_loop(  # ast-grep-ignore: no-long-names
        self, **params: Unpack[param_types.ChatAgenticRunParams]
    ) -> JsonValue: ...
    # ast-grep-ignore: no-long-names
    async def async_build_chat_completion_agentic_loop_plan(
        self, **params: Unpack[param_types.ChatAgenticRunParams]
    ) -> AgenticLoopPlan:
        _ = params
        return AgenticLoopPlan(run_agentic_loop=False)
