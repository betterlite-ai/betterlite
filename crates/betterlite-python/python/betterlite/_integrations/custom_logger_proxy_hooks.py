from collections.abc import AsyncGenerator, Sequence
from typing import Unpack

from pydantic import BaseModel, JsonValue

from betterlite._integrations import custom_logger_params as param_types
from betterlite._integrations.custom_logger_types import (
    CallTypes,
    CallTypesLiteral,
    CustomStreamWrapper,
    DualCache,
    HTTPException,
    LLMResponseTypes,
    PreRoutingHookResponse,
    Span,
    StandardLoggingPayload,
    UserAPIKeyAuth,
)
from betterlite.types.integrations.argilla import ArgillaItem
from betterlite.types.litellm_params_state import JsonMap
from betterlite.types.llms.openai import AllMessageValues, ChatCompletionRequest
from betterlite.types.utils import (
    AdapterCompletionStreamWrapper,
    ModelResponse,
    ModelResponseStream,
)


class CustomLoggerProxyHooks:
    async def async_pre_routing_hook(
        self, **params: Unpack[param_types.PreRoutingParams]
    ) -> PreRoutingHookResponse | None: ...
    async def async_filter_deployments(
        self,
        model: str,
        healthy_deployments: list[dict[str, JsonValue]],
        messages: list[AllMessageValues] | None,
        request_kwargs: JsonMap | None = None,
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
        self,
        request_data: JsonMap,
        response: LLMResponseTypes,
        call_type: CallTypes | None,
    ) -> LLMResponseTypes | None: ...
    async def async_post_call_failure_deployment_hook(  # ast-grep-ignore: no-long-names
        self,
        request_data: JsonMap,
        exception: Exception,
        call_type: CallTypes | None,
        fallback_depth: int | None = None,
    ) -> None: ...
    async def async_post_call_streaming_deployment_hook(  # ast-grep-ignore: no-long-names
        self,
        request_data: JsonMap,
        response_chunk: JsonValue,
        call_type: CallTypes | None,
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
        self,
        logged_item: ArgillaItem,
        standard_logging_payload: StandardLoggingPayload | None,
    ) -> ArgillaItem | None:
        raise NotImplementedError("async_dataset_hook not implemented")

    async def async_pre_call_hook(
        self,
        user_api_key_dict: UserAPIKeyAuth,
        cache: DualCache,
        data: JsonMap,
        call_type: CallTypesLiteral,
    ) -> Exception | str | dict[str, JsonValue] | None: ...
    async def async_filter_listed_models(
        self, user_api_key_dict: UserAPIKeyAuth, model_names: Sequence[str]
    ) -> Sequence[str]:
        _ = user_api_key_dict
        return model_names

    async def async_post_call_response_headers_hook(  # ast-grep-ignore: no-long-names
        self,
        data: JsonMap,
        user_api_key_dict: UserAPIKeyAuth,
        response: JsonValue,
        request_headers: dict[str, str] | None = None,
        litellm_call_info: JsonMap | None = None,
    ) -> dict[str, str] | None: ...
    async def async_post_call_failure_hook(
        self,
        request_data: JsonMap,
        original_exception: Exception,
        user_api_key_dict: UserAPIKeyAuth,
        traceback_str: str | None = None,
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
        self,
        user_api_key_dict: UserAPIKeyAuth,
        response: CustomStreamWrapper,
        request_data: JsonMap,
    ) -> AsyncGenerator[ModelResponseStream, None]:
        _ = (user_api_key_dict, request_data)
        async for item in response:
            yield item
