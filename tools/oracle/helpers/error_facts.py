from collections.abc import Callable
from types import SimpleNamespace
from typing import Protocol, TypeAlias, TypedDict, Unpack, cast

import openai
from litellm.litellm_core_utils import exception_mapping_utils as mapping

JsonValue: TypeAlias = None | bool | int | float | str | list["JsonValue"] | dict[str, "JsonValue"]


class StatusArguments(TypedDict):
    model: str
    original_exception: object
    custom_llm_provider: str
    error_str: str
    exception_provider: str
    extra_information: str


class StatusMapper(Protocol):
    def __call__(self, **kwargs: Unpack[StatusArguments]) -> None: ...


class MappedError(Protocol):
    status_code: int
    message: str


def _status(argument: dict[str, JsonValue], text: str) -> JsonValue:
    cause = SimpleNamespace(
        status_code=argument.get("status"),
        status_code_is_synthesized=argument.get("status_code_is_synthesized", False),
    )
    call = cast(StatusMapper, mapping._map_exception_by_status)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        call(model="model", original_exception=cause, custom_llm_provider="openai",
             error_str=text, exception_provider=str(argument.get("provider", "Provider")),
             extra_information="")
    except openai.APIError as error:
        mapped = cast(MappedError, error)
        families = {
            "AuthenticationError": "authentication", "PermissionDeniedError": "permission_denied",
            "NotFoundError": "not_found", "Timeout": "timeout", "RateLimitError": "rate_limit",
            "InternalServerError": "internal_server", "BadGatewayError": "bad_gateway",
            "ServiceUnavailableError": "service_unavailable", "BadRequestError": "bad_request",
            "APIError": "api",
        }
        return {"family": families[type(error).__name__],
                "status_code": mapped.status_code,
                "message": mapped.message, "response_present": False}
    return None


def run(argument: JsonValue) -> JsonValue:
    if not isinstance(argument, dict):
        raise ValueError("error facts input must be an object")
    kind = argument.get("kind")
    text_value = argument.get("text", "")
    if not isinstance(text_value, str):
        raise ValueError("text must be a string")
    result: JsonValue
    if kind == "rate_limit":
        status = argument.get("status")
        result = mapping.ExceptionCheckers.is_error_str_rate_limit(
            text_value, status if isinstance(status, int) else None)
    elif kind == "context_window":
        result = mapping.ExceptionCheckers.is_error_str_context_window_exceeded(text_value)
    elif kind == "body_code":
        result = mapping._get_body_error_code(text_value)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    elif kind == "message":
        extract = cast(Callable[[object], str | None], mapping.get_error_message)
        result = extract(SimpleNamespace(body=argument.get("body")))
    elif kind == "status":
        result = _status(argument, text_value)
    else:
        raise ValueError("unknown error facts operation")
    return {"result": result}
