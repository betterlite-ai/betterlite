from collections.abc import AsyncGenerator, Sequence
from datetime import datetime
from typing import ClassVar, Unpack

from pydantic import BaseModel, JsonValue

from betterlite._integrations import custom_logger_async_events as log_async
from betterlite._integrations import custom_logger_helpers as log_helpers
from betterlite._integrations import custom_logger_params as param_types
from betterlite._integrations.custom_logger_types import (
    LITELLM_METADATA_FIELD,
    OLD_LITELLM_METADATA_FIELD,
    SENSITIVE_DATA_MAX_DEPTH,
    AgenticLoopPlan,
    AsyncInputEventCallback,
    CallTypes,
    CallTypesLiteral,
    CustomStreamWrapper,
    DualCache,
    HTTPException,
    InputEventCallback,
    LLMResponseTypes,
    MCPPostCallResponseObject,
    PreRoutingHookResponse,
    Span,
    StandardAuditLogPayload,
    StandardLoggingPayload,
    UserAPIKeyAuth,
    VerboseFn,
)
from betterlite.types.integrations.argilla import ArgillaItem
from betterlite.types.litellm_params_state import JsonMap
from betterlite.types.llms.openai import AllMessageValues, ChatCompletionRequest
from betterlite.types.utils import (
    AdapterCompletionStreamWrapper,
    ModelResponse,
    ModelResponseStream,
)


class CustomLogger:
    server_fulfilled_tool_names: ClassVar[frozenset[str]] = frozenset()
    enforces_request_content: bool = False
    def __init__(
        self, *, turn_off_message_logging: bool = False, message_logging: bool = True,
        **kwargs: JsonValue,
    ) -> None:
        _ = kwargs
        self.message_logging = message_logging
        self.turn_off_message_logging = turn_off_message_logging
    @staticmethod
    def get_callback_env_vars(callback_name: str | None = None) -> list[str]:
        # this build has no Python callback registry (B189, B256), so no name resolves
        _ = callback_name
        return []
    def log_pre_api_call(self, model: str, messages: list[JsonValue], kwargs: JsonMap) -> None: ...
    def log_post_api_call(self, kwargs: JsonMap, response_obj: JsonValue,
        start_time: datetime, end_time: datetime) -> None: ...
    def log_stream_event(self, kwargs: JsonMap, response_obj: JsonValue,
        start_time: datetime, end_time: datetime) -> None: ...
    def log_success_event(self, kwargs: JsonMap, response_obj: JsonValue,
        start_time: datetime, end_time: datetime) -> None: ...
    def log_failure_event(self, kwargs: JsonMap, response_obj: JsonValue,
        start_time: datetime, end_time: datetime) -> None: ...
    async def async_log_stream_event(self, kwargs: JsonMap,
        response_obj: JsonValue, start_time: datetime, end_time: datetime) -> None: ...
    async def async_log_pre_api_call(self, model: str, messages: list[JsonValue],
        kwargs: JsonMap) -> None: ...
    async def async_pre_request_hook(self, model: str, messages: list[JsonValue],
        kwargs: JsonMap) -> JsonMap | None: ...
    async def async_log_success_event(self, kwargs: JsonMap, response_obj: JsonValue,
        start_time: datetime, end_time: datetime) -> None: ...
    async def async_log_failure_event(self, kwargs: JsonMap, response_obj: JsonValue,
        start_time: datetime, end_time: datetime) -> None: ...
    async def async_log_audit_log_event(self, audit_log: StandardAuditLogPayload) -> None: ...
    # ast-grep-ignore: no-long-names, no-one-line-functions
    async def async_get_chat_completion_prompt(
        self, **params: Unpack[param_types.AsyncChatPromptParams]
    ) -> tuple[str, list[AllMessageValues], dict[str, JsonValue]]:
        return params["model"], params["messages"], params["non_default_params"]
    def get_chat_completion_prompt(  # ast-grep-ignore: no-one-line-functions
        self, **params: Unpack[param_types.ChatPromptParams]
    ) -> tuple[str, list[AllMessageValues], dict[str, JsonValue]]:
        return params["model"], params["messages"], params["non_default_params"]
    async def async_pre_routing_hook(
        self, **params: Unpack[param_types.PreRoutingParams]
    ) -> PreRoutingHookResponse | None: ...
    async def async_filter_deployments(
        self, model: str, healthy_deployments: list[dict[str, JsonValue]],
        messages: list[AllMessageValues] | None, request_kwargs: JsonMap | None = None,
        parent_otel_span: Span | None = None,
    ) -> list[dict[str, JsonValue]]:
        _ = (model, messages, request_kwargs, parent_otel_span)
        return healthy_deployments
    async def async_pre_call_deployment_hook(
        self, kwargs: JsonMap, call_type: CallTypes | None
    ) -> dict[str, JsonValue] | None: ...
    async def async_pre_call_check(
        self, deployment: JsonMap, parent_otel_span: Span | None
    ) -> dict[str, JsonValue] | None: ...
    def pre_call_check(self, deployment: JsonMap) -> dict[str, JsonValue] | None: ...
    async def async_post_call_success_deployment_hook(  # ast-grep-ignore: no-long-names
        self, request_data: JsonMap, response: LLMResponseTypes, call_type: CallTypes | None,
    ) -> LLMResponseTypes | None: ...
    async def async_post_call_failure_deployment_hook(  # ast-grep-ignore: no-long-names
        self, request_data: JsonMap, exception: Exception,
        call_type: CallTypes | None, fallback_depth: int | None = None,
    ) -> None: ...
    async def async_post_call_streaming_deployment_hook(  # ast-grep-ignore: no-long-names
        self, request_data: JsonMap, response_chunk: JsonValue, call_type: CallTypes | None,
    ) -> JsonValue | None: ...
    async def log_model_group_rate_limit_error(  # ast-grep-ignore: no-long-names
        self, exception: Exception, original_model_group: str | None, kwargs: JsonMap
    ) -> None: ...
    async def log_success_fallback_event(
        self, original_model_group: str, kwargs: JsonMap, original_exception: Exception
    ) -> None: ...
    async def log_failure_fallback_event(
        self, original_model_group: str, kwargs: JsonMap, original_exception: Exception
    ) -> None: ...
    def translate_completion_input_params(  # ast-grep-ignore: no-long-names
        self, kwargs: JsonMap
    ) -> ChatCompletionRequest | None: ...
    def translate_completion_output_params(  # ast-grep-ignore: no-long-names
        self, response: ModelResponse
    ) -> BaseModel | None: ...
    def translate_completion_output_params_streaming(  # ast-grep-ignore: no-long-names
        self, completion_stream: JsonValue
    ) -> AdapterCompletionStreamWrapper | None: ...
    async def async_dataset_hook(  # ast-grep-ignore: no-one-line-functions
        self, logged_item: ArgillaItem,
        standard_logging_payload: StandardLoggingPayload | None,
    ) -> ArgillaItem | None:
        raise NotImplementedError("async_dataset_hook not implemented")
    async def async_pre_call_hook(
        self, user_api_key_dict: UserAPIKeyAuth, cache: DualCache,
        data: JsonMap, call_type: CallTypesLiteral,
    ) -> Exception | str | dict[str, JsonValue] | None: ...
    async def async_filter_listed_models(
        self, user_api_key_dict: UserAPIKeyAuth, model_names: Sequence[str]
    ) -> Sequence[str]:
        _ = user_api_key_dict
        return model_names
    async def async_post_call_response_headers_hook(  # ast-grep-ignore: no-long-names
        self, data: JsonMap, user_api_key_dict: UserAPIKeyAuth,
        response: JsonValue, request_headers: dict[str, str] | None = None,
        litellm_call_info: JsonMap | None = None,
    ) -> dict[str, str] | None: ...
    async def async_post_call_failure_hook(
        self, request_data: JsonMap, original_exception: Exception,
        user_api_key_dict: UserAPIKeyAuth, traceback_str: str | None = None,
    ) -> HTTPException | None: ...
    async def async_post_call_success_hook(
        self, data: JsonMap, user_api_key_dict: UserAPIKeyAuth, response: LLMResponseTypes
    ) -> JsonValue: ...
    async def async_logging_hook(
        self, kwargs: dict[str, JsonValue], result: JsonValue, call_type: str
    ) -> tuple[dict[str, JsonValue], JsonValue]:
        _ = call_type
        return kwargs, result
    def logging_hook(
        self, kwargs: dict[str, JsonValue], result: JsonValue, call_type: str
    ) -> tuple[dict[str, JsonValue], JsonValue]:
        _ = call_type
        return kwargs, result
    async def async_moderation_hook(
        self, data: JsonMap, user_api_key_dict: UserAPIKeyAuth, call_type: CallTypesLiteral
    ) -> JsonValue: ...
    async def async_post_call_streaming_hook(
        self, user_api_key_dict: UserAPIKeyAuth, response: str
    ) -> JsonValue: ...
    async def async_post_call_streaming_iterator_hook(  # ast-grep-ignore: no-long-names
        self, user_api_key_dict: UserAPIKeyAuth, response: CustomStreamWrapper,
        request_data: JsonMap,
    ) -> AsyncGenerator[ModelResponseStream, None]:
        _ = (user_api_key_dict, request_data)
        async for item in response:
            yield item
    def log_input_event(  # ast-grep-ignore: no-one-line-functions
        self, model: str, messages: list[JsonValue], kwargs: dict[str, JsonValue],
        print_verbose: VerboseFn, callback_func: InputEventCallback,
    ) -> None:
        log_helpers.run_input_event(callback_func, model, messages, kwargs, print_verbose)
    async def async_log_input_event(  # ast-grep-ignore: no-one-line-functions
        self, model: str, messages: list[JsonValue], kwargs: dict[str, JsonValue],
        print_verbose: VerboseFn, callback_func: AsyncInputEventCallback,
    ) -> None:
        await log_async.run_async_input(callback_func, model, messages, kwargs, print_verbose)
    def log_event(self, **params: Unpack[param_types.EventParams]) -> None:
        params["kwargs"]["log_event_type"] = "post_api_call"
        _ = params["callback_func"](
            params["kwargs"], params["response_obj"], params["start_time"], params["end_time"]
        )
    async def async_log_event(self, **params: Unpack[param_types.AsyncEventParams]) -> None:
        params["kwargs"]["log_event_type"] = "post_api_call"
        _ = await params["callback_func"](
            params["kwargs"], params["response_obj"], params["start_time"], params["end_time"]
        )
    async def async_post_mcp_tool_call_hook(
        self, kwargs: JsonMap, response_obj: MCPPostCallResponseObject,
        start_time: datetime, end_time: datetime,
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
    # ast-grep-ignore: no-long-names, no-one-line-functions
    def truncate_standard_logging_payload_content(
        self, standard_logging_object: StandardLoggingPayload
    ) -> StandardLoggingPayload:
        return log_helpers.truncate_payload(standard_logging_object, self._truncate_field)
    # ast-grep-ignore: no-one-line-functions
    def _truncate_field(self, field_value: JsonValue, max_length: int) -> str | None:
        return log_helpers.truncate_field(field_value, max_length, self._truncate_text)
    # ast-grep-ignore: no-one-line-functions
    def _truncate_text(self, text: str, max_length: int) -> str:
        return log_helpers.truncate_text(text, max_length)
    def _select_metadata_field(self, request_kwargs: JsonMap | None = None) -> str | None:
        if request_kwargs is None:
            return None
        if LITELLM_METADATA_FIELD in request_kwargs:
            return LITELLM_METADATA_FIELD
        return OLD_LITELLM_METADATA_FIELD
    def redacts_messages_itself(self) -> bool:  # ast-grep-ignore: no-one-line-functions
        return False
    def redact_standard_logging_payload_from_model_call_details(  # ast-grep-ignore: no-long-names
        self, model_call_details: dict[str, JsonValue]
    ) -> dict[str, JsonValue]:
        return log_helpers.redact_details(
            model_call_details, turn_off=self.turn_off_message_logging,
            redacts_itself=self.redacts_messages_itself(),
        )
    # ast-grep-ignore: no-long-names
    async def get_proxy_server_request_from_cold_storage_with_object_key(
        self, object_key: str
    ) -> JsonMap | None: ...
    def handle_callback_failure(self, callback_name: str) -> None:
        log_helpers.LOGGER.debug(
            "No callback with increment_callback_logging_failure method found for %s. %s",
            callback_name, "Ensure 'prometheus' is in your callbacks config.",
        )
    async def _strip_base64_from_messages(  # ast-grep-ignore: no-one-line-functions
        self, payload: StandardLoggingPayload, max_depth: int = SENSITIVE_DATA_MAX_DEPTH
    ) -> StandardLoggingPayload:
        return log_helpers.strip_base64(payload, max_depth, self._process_messages)
    def _strip_base64_from_messages_sync(  # ast-grep-ignore: no-long-names, no-one-line-functions
        self, payload: StandardLoggingPayload, max_depth: int = SENSITIVE_DATA_MAX_DEPTH
    ) -> StandardLoggingPayload:
        return log_helpers.strip_base64(payload, max_depth, self._process_messages)
    def _redact_base64(  # ast-grep-ignore: no-one-line-functions
        self, value: JsonValue, depth: int = 0, max_depth: int = SENSITIVE_DATA_MAX_DEPTH
    ) -> JsonValue:
        return log_helpers.redact_base64_value(value, depth, max_depth)
    # ast-grep-ignore: no-one-line-functions
    def _should_keep_content(self, content: JsonValue) -> bool:
        return log_helpers.should_keep_content(content)
    def _process_messages(
        self, messages: list[JsonValue], max_depth: int = SENSITIVE_DATA_MAX_DEPTH
    ) -> list[JsonValue]:
        return log_helpers.process_messages(
            messages, max_depth, self._should_keep_content, self._redact_base64
        )
