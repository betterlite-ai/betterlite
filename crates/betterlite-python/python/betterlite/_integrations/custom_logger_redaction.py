from typing import Final

from pydantic import JsonValue

import betterlite
from betterlite._integrations.custom_logger_redaction_response import redacted_response
from betterlite._integrations.custom_logger_types import (
    CLASSIFIER_AUDIT_FIELDS,
    LITELLM_METADATA_FIELD,
    OLD_LITELLM_METADATA_FIELD,
    REDACTED_CONTENT,
    StandardLoggingPayload,
)
from betterlite.types.litellm_params_state import JsonMap
from betterlite.types.utils import Message

TRUNCATION_SUFFIX: Final = "...truncated by litellm, this logger does not support large content"
MAX_STR_LENGTH: Final = 10_000
TRUNCATED_FIELDS: Final = ("error_str", "messages", "response")


def redacted_logging_object(
    standard_payload: dict[str, JsonValue],
    excluded_fields: list[str] | None,
    *,
    turn_off: bool,
    redacts_itself: bool,
) -> dict[str, JsonValue]:
    excluded: tuple[str, ...] = () if excluded_fields is None else tuple(excluded_fields)
    copied: dict[str, JsonValue] = {
        key: value
        for key, value in standard_payload.items()
        if key not in excluded and not (turn_off and key in CLASSIFIER_AUDIT_FIELDS)
    }
    if not turn_off or redacts_itself:
        return copied
    if "messages" not in excluded and copied.get("messages") is not None:
        copied["messages"] = [Message(content=REDACTED_CONTENT).model_dump()]
    if "response" not in excluded and copied.get("response") is not None:
        copied["response"] = redacted_response(copied["response"])
    return copied


def redacted_litellm_params(
    params: dict[str, JsonValue], *, turn_off: bool
) -> dict[str, JsonValue]:
    if not turn_off:
        return {}
    request_value = params.get("proxy_server_request")
    match request_value:
        case dict():
            request: dict[str, JsonValue] = request_value
            return {
                "litellm_params": {
                    **params,
                    "proxy_server_request": {
                        key: value
                        for key, value in request.items()
                        if key not in CLASSIFIER_AUDIT_FIELDS
                    },
                }
            }
        case str() | bool() | int() | float() | list() | None:
            return {}


class CustomLoggerRedaction:
    turn_off_message_logging: bool

    # ast-grep-ignore: no-long-names
    def truncate_standard_logging_payload_content(
        self, standard_logging_object: StandardLoggingPayload
    ) -> StandardLoggingPayload:
        candidates = {
            field: self._truncate_field(
                field_value=standard_logging_object.get(field), max_length=MAX_STR_LENGTH
            )
            for field in TRUNCATED_FIELDS
        }
        truncated_fields = {field: text for field, text in candidates.items() if text is not None}
        return {**standard_logging_object, **truncated_fields}

    def _truncate_field(self, field_value: JsonValue, max_length: int) -> str | None:
        text = "" if field_value is None else str(field_value)
        return None if len(text) <= max_length else self._truncate_text(text, max_length)

    def _truncate_text(self, text: str, max_length: int) -> str:
        if len(text) <= max_length:
            return text
        return f"{text[:max_length]}{TRUNCATION_SUFFIX}"

    def _select_metadata_field(self, request_kwargs: JsonMap | None = None) -> str | None:
        if request_kwargs is None:
            return None
        if LITELLM_METADATA_FIELD in request_kwargs:
            return LITELLM_METADATA_FIELD
        return OLD_LITELLM_METADATA_FIELD

    def redacts_messages_itself(self) -> bool:  # ast-grep-ignore: no-one-line-functions
        return False

    # ast-grep-ignore: no-long-names
    def redact_standard_logging_payload_from_model_call_details(
        self, model_call_details: dict[str, JsonValue]
    ) -> dict[str, JsonValue]:
        excluded_fields = betterlite.standard_logging_payload_excluded_fields
        if not self.turn_off_message_logging and not excluded_fields:
            return model_call_details
        raw_object = model_call_details.get("standard_logging_object")
        if raw_object is None:
            return dict(model_call_details)
        match raw_object:
            case dict():
                standard_payload: dict[str, JsonValue] = raw_object
                litellm_params_value = model_call_details.get("litellm_params")
                params_redaction: dict[str, JsonValue] = {}
                match litellm_params_value:
                    case dict():
                        params: dict[str, JsonValue] = litellm_params_value
                        params_redaction = redacted_litellm_params(
                            params, turn_off=self.turn_off_message_logging
                        )
                    case str() | bool() | int() | float() | list() | None:
                        pass
                return {
                    **model_call_details,
                    **params_redaction,
                    "standard_logging_object": redacted_logging_object(
                        standard_payload,
                        excluded_fields,
                        turn_off=self.turn_off_message_logging,
                        redacts_itself=self.redacts_messages_itself(),
                    ),
                }
            case str() | bool() | int() | float() | list():
                raise TypeError("standard_logging_object must be a dict")
