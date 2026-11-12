from collections.abc import Mapping
from typing import Literal, TypeAlias, TypedDict

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

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
    provider_specific_fields: Mapping[str, Json]
    litellm_call_id: str


class AnthropicErrorDetail(OptionalDetailFields):
    type: AnthropicErrorType
    message: Json


class OptionalResponseFields(TypedDict, total=False):
    request_id: str


class AnthropicErrorResponse(OptionalResponseFields):
    type: Literal["error"]
    error: AnthropicErrorDetail
