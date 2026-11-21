from datetime import datetime
from typing import Unpack

from pydantic import JsonValue

from betterlite._integrations import custom_logger_params as param_types
from betterlite._integrations.custom_logger_types import StandardAuditLogPayload
from betterlite.types.litellm_params_state import JsonMap
from betterlite.types.llms.openai import AllMessageValues


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
    # ast-grep-ignore: no-long-names, no-one-line-functions
    async def async_get_chat_completion_prompt(
        self, **params: Unpack[param_types.AsyncChatPromptParams]
    ) -> tuple[str, list[AllMessageValues], dict[str, JsonValue]]:
        return params["model"], params["messages"], params["non_default_params"]

    def get_chat_completion_prompt(  # ast-grep-ignore: no-one-line-functions
        self, **params: Unpack[param_types.ChatPromptParams]
    ) -> tuple[str, list[AllMessageValues], dict[str, JsonValue]]:
        return params["model"], params["messages"], params["non_default_params"]
