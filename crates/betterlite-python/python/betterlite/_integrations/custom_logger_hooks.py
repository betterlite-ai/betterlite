from datetime import datetime

from pydantic import JsonValue

from betterlite._integrations.custom_logger_types import (
    LiteLLMLoggingObj,
    StandardAuditLogPayload,
    StandardCallbackDynamicParams,
)
from betterlite.types.litellm_params_state import JsonMap
from betterlite.types.llms.openai import AllMessageValues
from betterlite.types.prompts.init_prompts import PromptSpec


class CustomLoggerHooks:
    def log_pre_api_call(self, model: str, messages: list[JsonValue], kwargs: JsonMap) -> None: ...
    def log_post_api_call(
        self, kwargs: JsonMap, response_obj: JsonValue, start_time: datetime, end_time: datetime
    ) -> None: ...
    def log_stream_event(
        self, kwargs: JsonMap, response_obj: JsonValue, start_time: datetime, end_time: datetime
    ) -> None: ...
    def log_success_event(
        self, kwargs: JsonMap, response_obj: JsonValue, start_time: datetime, end_time: datetime
    ) -> None: ...
    def log_failure_event(
        self, kwargs: JsonMap, response_obj: JsonValue, start_time: datetime, end_time: datetime
    ) -> None: ...
    async def async_log_stream_event(
        self, kwargs: JsonMap, response_obj: JsonValue, start_time: datetime, end_time: datetime
    ) -> None: ...
    async def async_log_pre_api_call(
        self, model: str, messages: list[JsonValue], kwargs: JsonMap
    ) -> None: ...
    async def async_pre_request_hook(
        self, model: str, messages: list[JsonValue], kwargs: JsonMap
    ) -> JsonMap | None: ...
    async def async_log_success_event(
        self, kwargs: JsonMap, response_obj: JsonValue, start_time: datetime, end_time: datetime
    ) -> None: ...
    async def async_log_failure_event(
        self, kwargs: JsonMap, response_obj: JsonValue, start_time: datetime, end_time: datetime
    ) -> None: ...
    async def async_log_audit_log_event(self, audit_log: StandardAuditLogPayload) -> None: ...
    # ast-grep-ignore: no-long-names
    async def async_get_chat_completion_prompt(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L200
        self,
        model: str,
        messages: list[AllMessageValues],
        non_default_params: dict[str, JsonValue],
        prompt_id: str | None,
        prompt_variables: dict[str, JsonValue] | None,
        dynamic_callback_params: StandardCallbackDynamicParams,
        litellm_logging_obj: LiteLLMLoggingObj,
        prompt_spec: PromptSpec | None = None,
        tools: list[dict[str, JsonValue]] | None = None,
        prompt_label: str | None = None,
        prompt_version: int | None = None,
        ignore_prompt_manager_model: bool | None = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L213
        # ast-grep-ignore: no-long-names
        ignore_prompt_manager_optional_params: bool | None = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L214
    ) -> tuple[str, list[AllMessageValues], dict[str, JsonValue]]:
        _ = (
            prompt_id, prompt_variables, dynamic_callback_params, litellm_logging_obj, prompt_spec,
            tools, prompt_label, prompt_version, ignore_prompt_manager_model,
            ignore_prompt_manager_optional_params,  # ast-grep-ignore: no-long-names
        )
        return model, messages, non_default_params

    def get_chat_completion_prompt(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L224
        self,
        model: str,
        messages: list[AllMessageValues],
        non_default_params: dict[str, JsonValue],
        prompt_id: str | None,
        prompt_variables: dict[str, JsonValue] | None,
        dynamic_callback_params: StandardCallbackDynamicParams,
        prompt_spec: PromptSpec | None = None,
        prompt_label: str | None = None,
        prompt_version: int | None = None,
        ignore_prompt_manager_model: bool | None = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L235
        # ast-grep-ignore: no-long-names
        ignore_prompt_manager_optional_params: bool | None = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L236
    ) -> tuple[str, list[AllMessageValues], dict[str, JsonValue]]:
        _ = (
            prompt_id, prompt_variables, dynamic_callback_params, prompt_spec, prompt_label,
            prompt_version, ignore_prompt_manager_model,
            ignore_prompt_manager_optional_params,  # ast-grep-ignore: no-long-names
        )
        return model, messages, non_default_params
