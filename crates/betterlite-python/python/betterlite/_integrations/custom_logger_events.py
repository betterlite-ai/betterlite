import logging
import traceback
from datetime import datetime
from typing import Final

from pydantic import JsonValue

from betterlite._integrations.custom_logger_types import (
    AgenticLoopPlan,
    AsyncEventCallback,
    AsyncInputEventCallback,
    BaseAnthropicMessagesConfig,
    EventCallback,
    InputEventCallback,
    LiteLLMLoggingObj,
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

    def log_event(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L557
        self,
        kwargs: dict[str, JsonValue],
        response_obj: JsonValue,
        start_time: datetime,
        end_time: datetime,
        print_verbose: VerboseFn,
        callback_func: EventCallback,
    ) -> None:
        try:
            kwargs["log_event_type"] = "post_api_call"
            _ = callback_func(kwargs, response_obj, start_time, end_time)
        except Exception:
            print_verbose(f"Custom Logger Error - {traceback.format_exc()}")
            LOGGER.debug("Custom Logger Error", exc_info=True)

    async def async_log_event(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L570
        self,
        kwargs: dict[str, JsonValue],
        response_obj: JsonValue,
        start_time: datetime,
        end_time: datetime,
        print_verbose: VerboseFn,
        callback_func: AsyncEventCallback,
    ) -> None:
        try:
            kwargs.update({"log_event_type": "post_api_call"})
            _ = await callback_func(kwargs, response_obj, start_time, end_time)
        except Exception:
            print_verbose("Custom Logger Error - " + traceback.format_exc())
            LOGGER.debug("Custom Logger Error", exc_info=True)

    async def async_post_mcp_tool_call_hook(
        self,
        kwargs: JsonMap,
        response_obj: MCPPostCallResponseObject,
        start_time: datetime,
        end_time: datetime,
    ) -> MCPPostCallResponseObject | None: ...
    async def async_should_run_agentic_loop(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L606
        self,
        response: JsonValue,
        model: str,
        messages: list[dict[str, JsonValue]],
        tools: list[dict[str, JsonValue]] | None,
        stream: bool,  # noqa: FBT001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L612
        custom_llm_provider: str,
        kwargs: dict[str, JsonValue],
    ) -> tuple[bool, dict[str, JsonValue]]:
        _ = (response, model, messages, tools, stream, custom_llm_provider, kwargs)
        return False, {}

    async def async_run_agentic_loop(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L664
        self,
        tools: dict[str, JsonValue],
        model: str,
        messages: list[dict[str, JsonValue]],
        response: JsonValue,
        # ast-grep-ignore: no-long-names
        anthropic_messages_provider_config: BaseAnthropicMessagesConfig | None,
        # ast-grep-ignore: no-long-names
        anthropic_messages_optional_request_params: dict[str, JsonValue],
        logging_obj: LiteLLMLoggingObj,
        stream: bool,  # noqa: FBT001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L673
        kwargs: dict[str, JsonValue],
    ) -> JsonValue: ...
    async def async_build_agentic_loop_plan(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L733
        self,
        tools: dict[str, JsonValue],
        model: str,
        messages: list[dict[str, JsonValue]],
        response: JsonValue,
        # ast-grep-ignore: no-long-names
        anthropic_messages_provider_config: BaseAnthropicMessagesConfig | None,
        # ast-grep-ignore: no-long-names
        anthropic_messages_optional_request_params: dict[str, JsonValue],
        logging_obj: LiteLLMLoggingObj,
        stream: bool,  # noqa: FBT001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L742
        kwargs: dict[str, JsonValue],
    ) -> AgenticLoopPlan:
        _ = (
            tools, model, messages, response, logging_obj, stream, kwargs,
            anthropic_messages_provider_config,  # ast-grep-ignore: no-long-names
            anthropic_messages_optional_request_params,  # ast-grep-ignore: no-long-names
        )
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
    async def async_should_run_chat_completion_agentic_loop(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L792
        self,
        response: JsonValue,
        model: str,
        messages: list[dict[str, JsonValue]],
        tools: list[dict[str, JsonValue]] | None,
        stream: bool,  # noqa: FBT001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L798
        custom_llm_provider: str,
        kwargs: dict[str, JsonValue],
    ) -> tuple[bool, dict[str, JsonValue]]:
        _ = (response, model, messages, tools, stream, custom_llm_provider, kwargs)
        return False, {}

    # ast-grep-ignore: no-long-names
    async def async_run_chat_completion_agentic_loop(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L807
        self,
        tools: dict[str, JsonValue],
        model: str,
        messages: list[dict[str, JsonValue]],
        response: JsonValue,
        optional_params: dict[str, JsonValue],
        logging_obj: LiteLLMLoggingObj,
        stream: bool,  # noqa: FBT001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L815
        kwargs: dict[str, JsonValue],
    ) -> JsonValue: ...
    # ast-grep-ignore: no-long-names
    async def async_build_chat_completion_agentic_loop_plan(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L822
        self,
        tools: dict[str, JsonValue],
        model: str,
        messages: list[dict[str, JsonValue]],
        response: JsonValue,
        optional_params: dict[str, JsonValue],
        logging_obj: LiteLLMLoggingObj,
        stream: bool,  # noqa: FBT001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L830
        kwargs: dict[str, JsonValue],
    ) -> AgenticLoopPlan:
        _ = (tools, model, messages, response, optional_params, logging_obj, stream, kwargs)
        return AgenticLoopPlan(run_agentic_loop=False)
