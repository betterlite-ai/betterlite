from collections.abc import Mapping
from typing import Literal, TypedDict

from betterlite.types.utils import JsonValue

AnthropicErrorType = Literal[
    "invalid_request_error",
    "authentication_error",
    "permission_error",
    "not_found_error",
    "request_too_large",
    "rate_limit_error",
    "api_error",
    "overloaded_error",
]


class OptionalDetailFields(TypedDict, total=False):
    provider_specific_fields: Mapping[str, JsonValue]
    litellm_call_id: str


class AnthropicErrorDetail(OptionalDetailFields):
    type: AnthropicErrorType
    message: JsonValue


class OptionalResponseFields(TypedDict, total=False):
    request_id: str


class AnthropicErrorResponse(OptionalResponseFields):
    type: Literal["error"]
    error: AnthropicErrorDetail
