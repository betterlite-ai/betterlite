from __future__ import annotations

from typing import TYPE_CHECKING, Protocol, TypeAlias, cast

import litellm
from litellm import utils
from litellm.types.utils import ModelResponse, ModelResponseStream

if TYPE_CHECKING:
    from collections.abc import Callable, Iterator, Sequence

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

MEDIUM_THINKING_BUDGET = 2048


# ruff: noqa: SLF001 - the case is about the private helpers LiteLLM routes through


class Dumpable(Protocol):
    def model_dump(self) -> Json: ...


class Utils(Protocol):
    MINIMUM_PROMPT_CACHE_TOKEN_COUNT_OVERRIDE: int | None

    def _is_gemini_model(self, model: str | None, custom_llm_provider: str | None) -> bool: ...

    def _remove_thought_signature_from_id(self, tool_call_id: str, separator: str) -> str: ...

    def _remove_thought_signatures_from_messages(
        self, messages: list[Json], separator: str
    ) -> list[Json]: ...

    def process_system_message(
        self, system_message: str, max_tokens: int, model: str | None
    ) -> tuple[Json, int]: ...

    def process_messages(
        self, messages: list[Json], max_tokens: int, model: str | None
    ) -> list[Json]: ...

    def attempt_message_addition(
        self,
        final_messages: list[Json],
        message: Json,
        available_tokens: int,
        max_tokens: int,
        model: str | None,
    ) -> list[Json]: ...

    def can_add_message(
        self, message: Json, messages: list[Json], max_tokens: int, model: str | None
    ) -> bool: ...

    def get_token_count(self, messages: list[Json], model: str | None) -> int: ...

    def shorten_message_to_fit_limit(
        self,
        message: Json,
        tokens_needed: int,
        model: str | None,
        *,
        raise_error_on_max_limit: bool = False,
    ) -> Json: ...

    def trim_messages(
        self,
        messages: list[Json],
        model: str | None = None,
        trim_ratio: float = 0.75,
        *,
        return_response_tokens: bool = False,
        max_tokens: int | None = None,
    ) -> Json: ...

    def convert_list_message_to_dict(self, messages: list[Json]) -> list[Json]: ...

    def validate_and_fix_openai_messages(self, messages: list[Json]) -> list[Json]: ...

    def validate_and_fix_openai_tools(self, tools: list[Json] | None) -> list[Json] | None: ...

    def validate_and_fix_thinking_param(self, thinking: Json) -> Json: ...

    def cleanup_none_field_in_message(self, message: Json) -> Json: ...

    def validate_chat_completion_user_messages(self, messages: list[Json]) -> list[Json]: ...

    def validate_chat_completion_tool_choice(self, tool_choice: Json, model: str = "") -> Json: ...

    def validate_openai_optional_params(self, stop: Json = None) -> Json: ...

    def is_cached_message(self, message: Json) -> bool: ...

    def get_prompt_cache_min_tokens(self, model: str) -> int: ...

    def is_prompt_caching_valid_prompt(
        self,
        model: str,
        messages: list[Json] | None,
        tools: list[Json] | None = None,
        custom_llm_provider: str | None = None,
        min_token_count: int | None = None,
    ) -> bool: ...

    def register_prompt_template(
        self,
        model: str,
        roles: Json = None,
        initial_prompt_value: str = "",
        final_prompt_value: str = "",
        tokenizer_config: Json = None,
    ) -> Json: ...

    def get_first_chars_messages(self, kwargs: Object) -> str: ...

    def get_empty_usage(self) -> Dumpable: ...

    def should_run_mock_completion(
        self, mock_response: Json, mock_tool_calls: Json, mock_timeout: Json
    ) -> bool: ...


class StreamWrapper(Protocol):
    def __init__(
        self,
        completion_stream: Iterator[ResponseModel],
        model: str,
        stream_options: Json = None,
        custom_llm_provider: str | None = None,
    ) -> None: ...

    def __iter__(self) -> Iterator[Dumpable]: ...


class ResponseIterator(Protocol):
    def __init__(
        self, model_response: Dumpable, *, convert_to_delta: bool = False
    ) -> None: ...

    def __iter__(self) -> Iterator[Dumpable]: ...


class ResponseListIterator(Protocol):
    def __init__(
        self, model_responses: Sequence[Dumpable], delay: float | None = None
    ) -> None: ...

    def __iter__(self) -> Iterator[Dumpable]: ...


class ResponseModel(Protocol):
    def __init__(self, **fields: Json) -> None: ...

    def model_dump(self) -> Json: ...


U = cast(Utils, utils)
WRAPPER = cast("type[StreamWrapper]", utils.TextCompletionStreamWrapper)
ITERATOR = cast("type[ResponseIterator]", utils.ModelResponseIterator)
LIST_ITERATOR = cast("type[ResponseListIterator]", utils.ModelResponseListIterator)
MODEL_RESPONSE = cast("type[ResponseModel]", ModelResponse)
STREAM_CHUNK = cast("type[ResponseModel]", ModelResponseStream)


def text(value: Json) -> str:
    if not isinstance(value, str):
        raise ValueError(f"expected a string, got {type(value).__name__}")
    return value


def optional_text(value: Json) -> str | None:
    return None if value is None else text(value)


def integer(value: Json) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"expected an integer, got {type(value).__name__}")
    return value


def object_of(value: Json) -> Object:
    if not isinstance(value, dict):
        raise ValueError(f"expected an object, got {type(value).__name__}")
    return value


def list_of(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"expected a list, got {type(value).__name__}")
    return value


def objects_of(value: Json) -> list[Object]:
    return [object_of(item) for item in list_of(value)]


def number(value: Json) -> float:
    if isinstance(value, bool) or not isinstance(value, int | float):
        raise ValueError(f"expected a number, got {type(value).__name__}")
    return float(value)


def call_field(call: Object, name: str) -> Json:
    if name not in call:
        raise ValueError(f"the call has no {name}")
    return call[name]


def run(input_value: dict[str, Json]) -> Json:
    # an unmapped model name makes get_llm_provider print to stdout, where the answer goes
    litellm.suppress_debug_info = True
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    return [run_call(object_of(call)) for call in calls]


def run_call(call: Object) -> Json:
    function = text(call.get("function"))
    for handlers in GROUPS:
        handler = handlers.get(function)
        if handler is not None:
            return handler(call)
    raise ValueError(f"unknown function: {function}")


def is_gemini_model(call: Object) -> Json:
    model = optional_text(call.get("model"))
    provider = optional_text(call.get("custom_llm_provider"))
    return U._is_gemini_model(model, provider)  # pyright: ignore[reportPrivateUsage]


def strip_signature_from_id(call: Object) -> Json:
    return U._remove_thought_signature_from_id(  # pyright: ignore[reportPrivateUsage]
        text(call_field(call, "tool_call_id")), text(call_field(call, "separator"))
    )


def strip_signatures_from_messages(call: Object) -> Json:
    return U._remove_thought_signatures_from_messages(  # pyright: ignore[reportPrivateUsage]
        list_of(call_field(call, "messages")), text(call_field(call, "separator"))
    )


def process_system_message(call: Object) -> Json:
    event, remaining = U.process_system_message(
        text(call_field(call, "system_message")),
        integer(call_field(call, "max_tokens")),
        optional_text(call.get("model")),
    )
    return [event, remaining]


def process_messages(call: Object) -> Json:
    return U.process_messages(
        list_of(call_field(call, "messages")),
        integer(call_field(call, "max_tokens")),
        optional_text(call.get("model")),
    )


def attempt_message_addition(call: Object) -> Json:
    return U.attempt_message_addition(
        list_of(call_field(call, "final_messages")),
        call_field(call, "message"),
        integer(call_field(call, "available_tokens")),
        integer(call_field(call, "max_tokens")),
        optional_text(call.get("model")),
    )


def can_add_message(call: Object) -> Json:
    return U.can_add_message(
        call_field(call, "message"),
        list_of(call_field(call, "messages")),
        integer(call_field(call, "max_tokens")),
        optional_text(call.get("model")),
    )


def get_token_count(call: Object) -> Json:
    return U.get_token_count(
        list_of(call_field(call, "messages")), optional_text(call.get("model"))
    )


def shorten_message_to_fit_limit(call: Object) -> Json:
    return U.shorten_message_to_fit_limit(
        call_field(call, "message"),
        integer(call_field(call, "tokens_needed")),
        optional_text(call.get("model")),
        raise_error_on_max_limit=call.get("raise_error_on_max_limit") is True,
    )


def trim_messages(call: Object) -> Json:
    trim_ratio = call.get("trim_ratio")
    max_tokens = call.get("max_tokens")
    return U.trim_messages(
        list_of(call_field(call, "messages")),
        model=optional_text(call.get("model")),
        trim_ratio=0.75 if trim_ratio is None else number(trim_ratio),
        return_response_tokens=call.get("return_response_tokens") is True,
        max_tokens=None if max_tokens is None else integer(max_tokens),
    )


def convert_list_message_to_dict(call: Object) -> Json:
    return U.convert_list_message_to_dict(list_of(call_field(call, "messages")))


def fix_openai_messages(call: Object) -> Json:
    return U.validate_and_fix_openai_messages(list_of(call_field(call, "messages")))


def validate_and_fix_openai_tools(call: Object) -> Json:
    tools = call.get("tools")
    return U.validate_and_fix_openai_tools(None if tools is None else list_of(tools))


def fix_thinking_param(call: Object) -> Json:
    return U.validate_and_fix_thinking_param(call_field(call, "thinking"))


def cleanup_none_field_in_message(call: Object) -> Json:
    return U.cleanup_none_field_in_message(call_field(call, "message"))


def validate_openai_user_messages(call: Object) -> Json:
    return U.validate_chat_completion_user_messages(list_of(call_field(call, "messages")))


def validate_tool_choice(call: Object) -> Json:
    return U.validate_chat_completion_tool_choice(call_field(call, "tool_choice"))


def validate_optional_params(call: Object) -> Json:
    litellm.disable_stop_sequence_limit = call.get("disable_stop_sequence_limit") is True
    return U.validate_openai_optional_params(call.get("stop"))


def is_cached_message(call: Object) -> Json:
    litellm.disable_anthropic_gemini_context_caching_transform = (
        call.get("disable_transform") is True
    )
    return U.is_cached_message(call_field(call, "message"))


def get_prompt_cache_min_tokens(call: Object) -> Json:
    override = call.get("minimum_token_override")
    U.MINIMUM_PROMPT_CACHE_TOKEN_COUNT_OVERRIDE = None if override is None else integer(override)
    return U.get_prompt_cache_min_tokens(text(call_field(call, "model")))


def is_prompt_caching_valid_prompt(call: Object) -> Json:
    messages = call.get("messages")
    tools = call.get("tools")
    min_token_count = call.get("min_token_count")
    return U.is_prompt_caching_valid_prompt(
        text(call_field(call, "model")),
        None if messages is None else list_of(messages),
        None if tools is None else list_of(tools),
        optional_text(call.get("custom_llm_provider")),
        None if min_token_count is None else integer(min_token_count),
    )


def text_completion_stream(call: Object) -> Json:
    include_usage = call.get("include_usage")
    options: Json = None if include_usage is None else {"include_usage": include_usage is True}
    chunks = [STREAM_CHUNK(**object_of(item)) for item in list_of(call_field(call, "chunks"))]
    stream = WRAPPER(iter(chunks), "", options)
    return [chunk.model_dump() for chunk in stream]


def model_response_iterator(call: Object) -> Json:
    response = MODEL_RESPONSE(**object_of(call_field(call, "response")))
    stream = ITERATOR(response, convert_to_delta=call.get("convert_to_delta") is True)
    return [chunk.model_dump() for chunk in stream]


def model_response_list_iterator(call: Object) -> Json:
    responses = [
        MODEL_RESPONSE(**object_of(item))
        for item in list_of(call_field(call, "responses"))
    ]
    delay = call.get("delay_seconds")
    stream = LIST_ITERATOR(responses, None if delay is None else number(delay))
    return [chunk.model_dump() for chunk in stream]


def register_prompt_template(call: Object) -> Json:
    litellm.custom_prompt_dict = {}
    litellm.known_tokenizer_config = {}  # pyright: ignore[reportPrivateImportUsage]
    U.register_prompt_template(
        text(call_field(call, "model")),
        call_field(call, "roles"),
        text(call.get("initial_prompt_value", "")),
        text(call.get("final_prompt_value", "")),
        call.get("tokenizer_config"),
    )
    return {
        "custom": cast("Json", litellm.custom_prompt_dict),  # pyright: ignore[reportPrivateImportUsage]
        "known": cast("Json", litellm.known_tokenizer_config),  # pyright: ignore[reportPrivateImportUsage]
    }


def get_first_chars_messages(call: Object) -> Json:
    return U.get_first_chars_messages({"messages": call.get("messages")})


def get_empty_usage(_call: Object) -> Json:
    return U.get_empty_usage().model_dump()


def should_run_mock_completion(call: Object) -> Json:
    return U.should_run_mock_completion(
        call.get("mock_response"),
        call.get("mock_tool_calls"),
        call.get("mock_timeout"),
    )


THOUGHT: dict[str, Callable[[Object], Json]] = {
    "is_gemini_model": is_gemini_model,
    "strip_signature_from_id": strip_signature_from_id,
    "strip_signatures_from_messages": strip_signatures_from_messages,
}

TRIM: dict[str, Callable[[Object], Json]] = {
    "process_system_message": process_system_message,
    "process_messages": process_messages,
    "attempt_message_addition": attempt_message_addition,
    "can_add_message": can_add_message,
    "get_token_count": get_token_count,
    "shorten_message_to_fit_limit": shorten_message_to_fit_limit,
    "trim_messages": trim_messages,
}

VALIDATORS: dict[str, Callable[[Object], Json]] = {
    "convert_list_message_to_dict": convert_list_message_to_dict,
    "fix_openai_messages": fix_openai_messages,
    "validate_and_fix_openai_tools": validate_and_fix_openai_tools,
    "fix_thinking_param": fix_thinking_param,
    "cleanup_none_field_in_message": cleanup_none_field_in_message,
    "validate_openai_user_messages": validate_openai_user_messages,
    "validate_tool_choice": validate_tool_choice,
    "validate_optional_params": validate_optional_params,
}

CACHE: dict[str, Callable[[Object], Json]] = {
    "is_cached_message": is_cached_message,
    "get_prompt_cache_min_tokens": get_prompt_cache_min_tokens,
    "is_prompt_caching_valid_prompt": is_prompt_caching_valid_prompt,
}

STREAMS: dict[str, Callable[[Object], Json]] = {
    "text_completion_stream": text_completion_stream,
    "model_response_iterator": model_response_iterator,
    "model_response_list_iterator": model_response_list_iterator,
}

SMALL: dict[str, Callable[[Object], Json]] = {
    "register_prompt_template": register_prompt_template,
    "get_first_chars_messages": get_first_chars_messages,
    "get_empty_usage": get_empty_usage,
    "should_run_mock_completion": should_run_mock_completion,
}

GROUPS: tuple[dict[str, Callable[[Object], Json]], ...] = (
    THOUGHT,
    TRIM,
    VALIDATORS,
    CACHE,
    STREAMS,
    SMALL,
)
