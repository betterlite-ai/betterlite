import copy
import logging
import re
from collections.abc import Callable
from typing import Final, Protocol

from pydantic import JsonValue

import betterlite
from betterlite._integrations.custom_logger_types import (
    CLASSIFIER_AUDIT_FIELDS,
    SENSITIVE_DATA_MAX_DEPTH,
    InputEventCallback,
    VerboseFn,
)
from betterlite.types.utils import Choices, Message, ModelResponse

LOGGER: Final = logging.getLogger("LiteLLM")


class RedactFn(Protocol):
    def __call__(self, value: JsonValue, *, max_depth: int) -> JsonValue: ...


class ProcessFn(Protocol):
    def __call__(self, messages: list[JsonValue], *, max_depth: int) -> list[JsonValue]: ...

BASE64_INLINE_PATTERN: Final = re.compile(
    r"data:(?:application|image|audio|video)/[a-zA-Z0-9.+-]+;base64,[A-Za-z0-9+/=\s]+",
    re.MULTILINE,
)
BASE64_REDACTED: Final = "[BASE64_REDACTED]"
MAX_DEPTH_REACHED: Final = "[MAX_DEPTH_REACHED]"
REDACTED_CONTENT: Final = "redacted-by-litellm"
TRUNCATION_SUFFIX: Final = "...truncated by litellm, this logger does not support large content"
MAX_STR_LENGTH: Final = 10_000
TRUNCATED_FIELDS: Final = ("error_str", "messages", "response")


def dict_value(value: JsonValue) -> dict[str, JsonValue] | None:
    match value:
        case dict():
            return value
        case str() | bool() | int() | float() | list() | None:
            return None


def list_value(value: JsonValue) -> list[JsonValue]:
    match value:
        case list():
            return value
        case str() | bool() | int() | float() | dict() | None:
            return []


def truncate_text(text: str, max_length: int) -> str:
    if len(text) <= max_length:
        return text
    return f"{text[:max_length]}{TRUNCATION_SUFFIX}"


def truncate_field(
    field_value: JsonValue, max_length: int, truncate: Callable[[str, int], str]
) -> str | None:
    text = "" if field_value is None else str(field_value)
    return None if len(text) <= max_length else truncate(text, max_length)


def truncate_payload(
    payload: dict[str, JsonValue], truncate: Callable[[JsonValue, int], str | None]
) -> dict[str, JsonValue]:
    truncated: dict[str, JsonValue] = {}
    for field in TRUNCATED_FIELDS:
        text = truncate(payload.get(field), MAX_STR_LENGTH)
        if text is not None:
            truncated[field] = text
    return {**payload, **truncated}


def without_classifier_audit(payload: dict[str, JsonValue]) -> dict[str, JsonValue]:
    return {
        key: value for key, value in payload.items() if key not in CLASSIFIER_AUDIT_FIELDS
    }


def redact_inline_base64(text: str) -> str:
    if not BASE64_INLINE_PATTERN.search(text):
        return text
    LOGGER.debug("[CustomLogger] Redacted inline base64 string: %s...", text[:40])
    return BASE64_INLINE_PATTERN.sub(BASE64_REDACTED, text)


def redact_list_items(  # ast-grep-ignore: no-one-line-functions
    value: list[JsonValue], depth: int, max_depth: int
) -> list[JsonValue]:
    return [redact_base64_value(item, depth + 1, max_depth) for item in value]


def redact_dict_items(  # ast-grep-ignore: no-one-line-functions
    value: dict[str, JsonValue], depth: int, max_depth: int
) -> dict[str, JsonValue]:
    return {key: redact_base64_value(item, depth + 1, max_depth) for key, item in value.items()}


def redact_base64_value(
    value: JsonValue, depth: int = 0, max_depth: int = SENSITIVE_DATA_MAX_DEPTH
) -> JsonValue:
    if depth > max_depth:
        LOGGER.warning(
            "[CustomLogger] Max recursion depth %s reached while redacting base64", max_depth
        )
        return MAX_DEPTH_REACHED
    match value:
        case str():
            return redact_inline_base64(value)
        case list():
            return redact_list_items(value, depth, max_depth)
        case dict():
            return redact_dict_items(value, depth, max_depth)
        case bool() | int() | float() | None:
            return value


def is_non_text_type(value: JsonValue) -> bool:
    match value:
        case str() as kind:
            return kind != "text"
        case bool() | int() | float() | list() | dict() | None:
            return False


def should_keep_content(content: JsonValue) -> bool:
    content_dict = dict_value(content)
    if content_dict is None:
        return True
    return "file" not in content_dict and not is_non_text_type(content_dict.get("type"))


def count_content_items(payload: dict[str, JsonValue]) -> int:
    total = 0
    for message in list_value(payload.get("messages")):
        message_dict = dict_value(message)
        if message_dict is not None:
            total += len(list_value(message_dict.get("content")))
    return total


def cleaned_content(  # ast-grep-ignore: no-one-line-functions
    contents: list[JsonValue], max_depth: int, should_keep: Callable[[JsonValue], bool],
    redact: RedactFn,
) -> list[JsonValue]:
    return [redact(content, max_depth=max_depth) for content in contents if should_keep(content)]


def process_messages(
    messages: list[JsonValue], max_depth: int, keep: Callable[[JsonValue], bool],
    redact: RedactFn,
) -> list[JsonValue]:
    filtered: list[JsonValue] = []
    for message in messages:
        message_dict = dict_value(message)
        if message_dict is None:
            continue
        contents = message_dict.get("content")
        match contents:
            case list() as content_list:
                message_dict["content"] = cleaned_content(content_list, max_depth, keep, redact)
            case str() | bool() | int() | float() | dict() | None:
                message_dict["content"] = redact(contents, max_depth=max_depth)
        for key, value in list(message_dict.items()):
            if key != "content":
                message_dict[key] = redact(value, max_depth=max_depth)
        filtered.append(message_dict)
    return filtered


def strip_base64(
    payload: dict[str, JsonValue],
    max_depth: int,
    process: ProcessFn,
) -> dict[str, JsonValue]:
    messages = list_value(payload.get("messages"))
    LOGGER.debug("[CustomLogger] Stripping base64 from %s messages", len(messages))
    if messages:
        payload["messages"] = process(messages, max_depth=max_depth)
    LOGGER.debug(
        "[CustomLogger] Completed base64 strip; retained %s content items",
        count_content_items(payload),
    )
    return payload


def redacted_model_response() -> JsonValue:  # ast-grep-ignore: no-one-line-functions
    return ModelResponse(choices=[Choices(message=Message(content=REDACTED_CONTENT))]).model_dump()


def redact_content_text(content_item: JsonValue) -> None:
    content_dict = dict_value(content_item)
    if content_dict is not None and "text" in content_dict:
        content_dict["text"] = REDACTED_CONTENT


def redact_output_item(output_item: JsonValue) -> None:
    output_dict = dict_value(output_item)
    if output_dict is not None:
        for content_item in list_value(output_dict.get("content")):
            redact_content_text(content_item)


def redacted_responses_api(response: dict[str, JsonValue]) -> dict[str, JsonValue]:
    copied = copy.deepcopy(response)
    for output_item in list_value(copied.get("output")):
        redact_output_item(output_item)
    return copied


def redacted_response_value(response: JsonValue) -> JsonValue:
    response_dict = dict_value(response)
    if response_dict is not None and "output" in response_dict:
        return redacted_responses_api(response_dict)
    return redacted_model_response()


def redact_messages_and_response(
    payload: dict[str, JsonValue], excluded: tuple[str, ...]
) -> None:
    if "messages" not in excluded and payload.get("messages") is not None:
        payload["messages"] = [Message(content=REDACTED_CONTENT).model_dump()]
    if "response" not in excluded and payload.get("response") is not None:
        payload["response"] = redacted_response_value(payload["response"])


def copy_standard_logging_object(
    payload: dict[str, JsonValue], excluded_fields: list[str] | None, *,
    turn_off: bool, redacts_itself: bool,
) -> dict[str, JsonValue]:
    excluded: tuple[str, ...] = () if excluded_fields is None else tuple(excluded_fields)
    copied: dict[str, JsonValue] = {
        key: value for key, value in payload.items()
        if key not in excluded and not (turn_off and key in CLASSIFIER_AUDIT_FIELDS)
    }
    if turn_off and not redacts_itself:
        redact_messages_and_response(copied, excluded)
    return copied


def redact_details(
    model_call_details: dict[str, JsonValue], *, turn_off: bool, redacts_itself: bool,
) -> dict[str, JsonValue]:
    excluded_fields: list[str] | None = betterlite.standard_logging_payload_excluded_fields
    if not turn_off and not excluded_fields:
        return model_call_details
    raw_object = model_call_details.get("standard_logging_object")
    if raw_object is None:
        return dict(model_call_details)
    standard_logging_object = dict_value(raw_object)
    if standard_logging_object is None:
        raise TypeError("standard_logging_object must be a dict")
    copied = copy_standard_logging_object(
        standard_logging_object, excluded_fields, turn_off=turn_off,
        redacts_itself=redacts_itself,
    )
    params = dict_value(model_call_details.get("litellm_params"))
    request = None if params is None else dict_value(params.get("proxy_server_request"))
    if turn_off and params is not None and request is not None:
        cleaned = without_classifier_audit(request)
        return {
            **model_call_details,
            "litellm_params": {**params, "proxy_server_request": cleaned},
            "standard_logging_object": copied,
        }
    return {**model_call_details, "standard_logging_object": copied}


def add_input_event_fields(
    kwargs: dict[str, JsonValue], model: str, messages: list[JsonValue]
) -> dict[str, JsonValue]:
    kwargs["model"] = model
    kwargs["messages"] = messages
    kwargs["log_event_type"] = "pre_api_call"
    return kwargs


def run_input_event(
    callback_func: InputEventCallback, model: str, messages: list[JsonValue],
    kwargs: dict[str, JsonValue], print_verbose: VerboseFn,
) -> None:
    _ = callback_func(add_input_event_fields(kwargs, model, messages))
    print_verbose(f"Custom Logger - model call details: {kwargs}")
