import logging
from collections.abc import Callable
from typing import Final

from pydantic import JsonValue

from betterlite._integrations.custom_logger_base64_values import CustomLoggerBase64Values
from betterlite._integrations.custom_logger_types import (
    SENSITIVE_DATA_MAX_DEPTH,
    StandardLoggingPayload,
)

LOGGER: Final = logging.getLogger("LiteLLM")


class CustomLoggerBase64(CustomLoggerBase64Values):
    async def _strip_base64_from_messages(
        self, payload: StandardLoggingPayload, max_depth: int = SENSITIVE_DATA_MAX_DEPTH
    ) -> StandardLoggingPayload:
        return strip_base64(payload, max_depth, self._process_messages)

    # ast-grep-ignore: no-long-names
    def _strip_base64_from_messages_sync(
        self, payload: StandardLoggingPayload, max_depth: int = SENSITIVE_DATA_MAX_DEPTH
    ) -> StandardLoggingPayload:
        return strip_base64(payload, max_depth, self._process_messages)

    def _process_messages(
        self, messages: list[JsonValue], max_depth: int = SENSITIVE_DATA_MAX_DEPTH
    ) -> list[JsonValue]:
        filtered_messages: list[JsonValue] = []
        for message in messages:
            match message:
                case dict():
                    message_payload: dict[str, JsonValue] = message
                    contents = message_payload.get("content")
                    match contents:
                        case list():
                            content_items: list[JsonValue] = contents
                            message_payload["content"] = [
                                self._redact_base64(value=content_item, max_depth=max_depth)
                                for content_item in content_items
                                if self._should_keep_content(content_item)
                            ]
                        case str() | bool() | int() | float() | dict() | None:
                            message_payload["content"] = self._redact_base64(
                                value=contents, max_depth=max_depth
                            )
                    for key, value in list(message_payload.items()):
                        if key != "content":
                            message_payload[key] = self._redact_base64(
                                value=value, max_depth=max_depth
                            )
                    filtered_messages.append(message_payload)
                case str() | bool() | int() | float() | list() | None:
                    continue
        return filtered_messages


def strip_base64(
    payload: dict[str, JsonValue],
    max_depth: int,
    process: Callable[[list[JsonValue], int], list[JsonValue]],
) -> dict[str, JsonValue]:
    messages: list[JsonValue] = []
    raw_messages = payload.get("messages")
    match raw_messages:
        case list():
            message_list: list[JsonValue] = raw_messages
            messages = message_list
            LOGGER.debug("[CustomLogger] Stripping base64 from %s messages", len(messages))
        case str() | bool() | int() | float() | dict() | None:
            LOGGER.debug("[CustomLogger] Stripping base64 from %s messages", 0)
    if messages:
        processed_messages = process(messages, max_depth)
        payload["messages"] = processed_messages
        messages = processed_messages
    LOGGER.debug(
        "[CustomLogger] Completed base64 strip; retained %s content items",
        sum(content_item_count(message) for message in messages),
    )
    return payload


def content_item_count(message: JsonValue) -> int:
    match message:
        case dict():
            message_payload: dict[str, JsonValue] = message
            contents = message_payload.get("content")
            match contents:
                case list() as content_items:
                    return len(content_items)
                case str() | bool() | int() | float() | dict() | None:
                    return 0
        case str() | bool() | int() | float() | list() | None:
            return 0
