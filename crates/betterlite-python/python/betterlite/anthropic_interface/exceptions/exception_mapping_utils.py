import json
import typing
from typing import Final, TypeGuard

from betterlite.types.utils import JsonValue

from .exceptions import AnthropicErrorDetail, AnthropicErrorResponse, AnthropicErrorType


def _is_json_object(value: JsonValue) -> TypeGuard[dict[str, JsonValue]]:
    match value:
        case {}:
            return True
        case _:
            return False


ANTHROPIC_ERROR_TYPE_MAP: Final[dict[int, AnthropicErrorType]] = {
    400: "invalid_request_error",
    401: "authentication_error",
    403: "permission_error",
    404: "not_found_error",
    413: "request_too_large",
    429: "rate_limit_error",
    500: "api_error",
    529: "overloaded_error",
}


class AnthropicExceptionMapping:
    @staticmethod
    def get_error_type(status_code: int) -> AnthropicErrorType:
        error_type = ANTHROPIC_ERROR_TYPE_MAP.get(status_code)
        if error_type is None:
            return "api_error"
        return error_type

    @staticmethod
    def create_error_response(
        status_code: int,
        message: JsonValue,
        request_id: str | None = None,
    ) -> AnthropicErrorResponse:
        error_type = AnthropicExceptionMapping.get_error_type(status_code)
        response: AnthropicErrorResponse = {
            "type": "error",
            "error": {"type": error_type, "message": message},
        }
        if request_id:
            response["request_id"] = request_id
        return response

    @staticmethod
    def extract_error_message(raw_message: str) -> JsonValue:
        parsed = AnthropicExceptionMapping._parse_json_object(raw_message)
        if parsed is None:
            return raw_message
        return AnthropicExceptionMapping._extract_message_from_dict(parsed, raw_message)

    @staticmethod
    def _parse_json_object(raw_message: str) -> dict[str, JsonValue] | None:
        try:
            parsed: JsonValue = json.loads(raw_message)
        except (ValueError, RecursionError):
            return None
        if _is_json_object(parsed):
            return parsed
        return None

    @staticmethod
    def _is_anthropic_error_dict(parsed: dict[str, JsonValue]) -> TypeGuard[AnthropicErrorResponse]:
        match parsed.get("error"):
            case {"message": _, "type": _}:
                return parsed.get("type") == "error"
            case _:
                return False

    @staticmethod
    def _extract_message_from_dict(parsed: dict[str, JsonValue], raw_message: str) -> JsonValue:
        detail = parsed.get("detail")
        if _is_json_object(detail):
            if "message" not in detail:
                return raw_message
            return detail["message"]
        aws = parsed.get("Message")
        if aws:
            return aws
        generic = parsed.get("message")
        if generic:
            return generic
        return raw_message

    @staticmethod
    def transform_to_anthropic_error(
        status_code: int,
        raw_message: str,
        request_id: str | None = None,
    ) -> AnthropicErrorResponse:
        parsed = AnthropicExceptionMapping._parse_json_object(raw_message)
        if parsed is not None and AnthropicExceptionMapping._is_anthropic_error_dict(parsed):
            if request_id and "request_id" not in parsed:
                parsed["request_id"] = request_id
            return parsed
        if parsed is None:
            return AnthropicExceptionMapping.create_error_response(
                status_code,
                raw_message,
                request_id,
            )
        message = AnthropicExceptionMapping._extract_message_from_dict(parsed, raw_message)
        return AnthropicExceptionMapping.create_error_response(status_code, message, request_id)


class AnthropicErrorSseFrame(str):
    __slots__ = ("error_response", "status_code")

    status_code: int
    error_response: AnthropicErrorResponse

    def __new__(
        cls,
        status_code: int,
        error_response: AnthropicErrorResponse,
    ) -> "typing.Self":
        frame = super().__new__(cls, f"event: error\ndata: {json.dumps(error_response)}\n\n")
        frame.status_code = status_code
        frame.error_response = error_response
        return frame

    def json_body(self, call_id: str | None) -> AnthropicErrorResponse:
        if call_id is None:
            return self.error_response
        detail: AnthropicErrorDetail = {**self.error_response["error"], "litellm_call_id": call_id}
        return {**self.error_response, "error": detail}


def anthropic_error_sse_frame(status_code: int, raw_message: str) -> AnthropicErrorSseFrame:
    error_response = AnthropicExceptionMapping.transform_to_anthropic_error(
        status_code,
        raw_message,
    )
    return AnthropicErrorSseFrame(status_code, error_response)
