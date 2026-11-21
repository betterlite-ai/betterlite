from datetime import datetime
from typing import NotRequired, TypedDict

from pydantic import JsonValue

from betterlite._integrations.custom_logger_types import (
    AsyncEventCallback,
    BaseAnthropicMessagesConfig,
    EventCallback,
    LiteLLMLoggingObj,
    StandardCallbackDynamicParams,
    VerboseFn,
)
from betterlite.types.llms.openai import AllMessageValues
from betterlite.types.prompts.init_prompts import PromptSpec


class AsyncChatPromptParams(TypedDict):
    model: str
    messages: list[AllMessageValues]
    non_default_params: dict[str, JsonValue]
    prompt_id: str | None
    prompt_variables: dict[str, JsonValue] | None
    dynamic_callback_params: StandardCallbackDynamicParams
    litellm_logging_obj: LiteLLMLoggingObj
    prompt_spec: NotRequired[PromptSpec | None]
    tools: NotRequired[list[dict[str, JsonValue]] | None]
    prompt_label: NotRequired[str | None]
    prompt_version: NotRequired[int | None]
    ignore_prompt_manager_model: NotRequired[bool | None]
    # ast-grep-ignore: no-long-names
    ignore_prompt_manager_optional_params: NotRequired[bool | None]


class ChatPromptParams(TypedDict):
    model: str
    messages: list[AllMessageValues]
    non_default_params: dict[str, JsonValue]
    prompt_id: str | None
    prompt_variables: dict[str, JsonValue] | None
    dynamic_callback_params: StandardCallbackDynamicParams
    prompt_spec: NotRequired[PromptSpec | None]
    prompt_label: NotRequired[str | None]
    prompt_version: NotRequired[int | None]
    ignore_prompt_manager_model: NotRequired[bool | None]
    # ast-grep-ignore: no-long-names
    ignore_prompt_manager_optional_params: NotRequired[bool | None]


class AgenticGateParams(TypedDict):
    response: JsonValue
    model: str
    messages: list[dict[str, JsonValue]]
    tools: list[dict[str, JsonValue]] | None
    stream: bool
    custom_llm_provider: str
    kwargs: dict[str, JsonValue]


class AgenticRunParams(TypedDict):
    tools: dict[str, JsonValue]
    model: str
    messages: list[dict[str, JsonValue]]
    response: JsonValue
    # ast-grep-ignore: no-long-names
    anthropic_messages_provider_config: BaseAnthropicMessagesConfig | None

    # ast-grep-ignore: no-long-names
    anthropic_messages_optional_request_params: dict[str, JsonValue]
    logging_obj: LiteLLMLoggingObj
    stream: bool
    kwargs: dict[str, JsonValue]


class ChatAgenticRunParams(TypedDict):
    tools: dict[str, JsonValue]
    model: str
    messages: list[dict[str, JsonValue]]
    response: JsonValue
    optional_params: dict[str, JsonValue]
    logging_obj: LiteLLMLoggingObj
    stream: bool
    kwargs: dict[str, JsonValue]


class EventParams(TypedDict):
    kwargs: dict[str, JsonValue]
    response_obj: JsonValue
    start_time: datetime
    end_time: datetime
    print_verbose: VerboseFn
    callback_func: EventCallback


class AsyncEventParams(TypedDict):
    kwargs: dict[str, JsonValue]
    response_obj: JsonValue
    start_time: datetime
    end_time: datetime
    print_verbose: VerboseFn
    callback_func: AsyncEventCallback


class PreRoutingParams(TypedDict):
    model: str
    request_kwargs: dict[str, JsonValue]
    messages: NotRequired[list[dict[str, JsonValue]] | None]
    input: NotRequired[str | list[JsonValue] | None]
    specific_deployment: NotRequired[bool | None]
