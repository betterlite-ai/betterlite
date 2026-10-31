from types import SimpleNamespace
from typing import Protocol, TypeAlias, TypedDict, Unpack, cast

import httpx
import openai
from litellm.litellm_core_utils import exception_mapping_utils as mapping

JsonValue: TypeAlias = None | bool | int | float | str | list["JsonValue"] | dict[str, "JsonValue"]


class MapperArguments(TypedDict):
    model: str
    original_exception: object
    custom_llm_provider: str
    error_str: str
    exception_type: str
    exception_provider: str
    extra_information: str


class Mapper(Protocol):
    def __call__(self, **kwargs: Unpack[MapperArguments]) -> None: ...


class MappedError(Protocol):
    status_code: int


_ANTHROPIC_FAMILIES: dict[str, str] = {
    "ContextWindowExceededError": "context_window_exceeded",
    "InternalServerError": "internal_server",
    "AuthenticationError": "authentication",
    "ContentPolicyViolationError": "content_policy_violation",
    "BadRequestError": "bad_request",
    "PermissionDeniedError": "permission_denied",
    "NotFoundError": "not_found",
    "Timeout": "timeout",
    "RateLimitError": "rate_limit",
    "BadGatewayError": "bad_gateway",
    "ServiceUnavailableError": "service_unavailable",
}

_REPLICATE_FAMILIES: dict[str, str] = {
    "AuthenticationError": "authentication",
    "ContextWindowExceededError": "context_window_exceeded",
    "BadRequestError": "bad_request",
    "RateLimitError": "rate_limit",
    "UnprocessableEntityError": "unprocessable_entity",
    "Timeout": "timeout",
    "ServiceUnavailableError": "service_unavailable",
    "APIError": "api",
}


def _context(model: str, provider: str) -> dict[str, JsonValue]:
    return {
        "message": "ignored",
        "model": model,
        "provider": provider,
        "max_retries": None,
        "num_retries": None,
        "body_type": None,
        "body_param": None,
        "body_code": None,
    }


def _response(status_code: int) -> httpx.Response:
    request = httpx.Request(method="POST", url="https://example.com/v1/")
    return httpx.Response(status_code=status_code, request=request)


def _result_for(
    error: openai.APIError, families: dict[str, str], context: dict[str, JsonValue]
) -> JsonValue:
    family = families[type(error).__name__]
    if family == "rate_limit":
        return {
            "family": family,
            "context": context,
            "category": "vendor_rate_limit",
            "dimension": None,
            "headers": {},
        }
    if family == "timeout":
        mapped = cast(MappedError, error)
        return {"family": family, "context": context, "original_status": mapped.status_code}
    if family == "api":
        mapped = cast(MappedError, error)
        return {"family": family, "context": context, "status": mapped.status_code}
    return {"family": family, **context}


def _cause(error_str: str, status_code: int | None) -> SimpleNamespace:
    if status_code is None:
        return SimpleNamespace(message=error_str)
    return SimpleNamespace(
        message=error_str, status_code=status_code, response=_response(status_code)
    )


def _run_anthropic(model: str, error_str: str, status_code: int | None) -> JsonValue:
    cause = _cause(error_str, status_code)
    call = cast(Mapper, mapping._map_anthropic_exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        call(
            model=model,
            original_exception=cause,
            custom_llm_provider="anthropic",
            error_str=error_str,
            exception_type="APIError",
            exception_provider="anthropic",
            extra_information="",
        )
    except openai.APIError as error:
        return {"result": _result_for(error, _ANTHROPIC_FAMILIES, _context(model, "anthropic"))}
    return {"result": None}


def _run_replicate(
    model: str, error_str: str, exception_type: str, status_code: int | None
) -> JsonValue:
    cause = _cause(error_str, status_code)
    call = cast(Mapper, mapping._map_replicate_exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        call(
            model=model,
            original_exception=cause,
            custom_llm_provider="replicate",
            error_str=error_str,
            exception_type=exception_type,
            exception_provider="replicate",
            extra_information="",
        )
    except openai.APIError as error:
        return {"result": _result_for(error, _REPLICATE_FAMILIES, _context(model, "replicate"))}
    raise AssertionError("_map_replicate_exception must always raise")


def run(argument: JsonValue) -> JsonValue:
    if not isinstance(argument, dict):
        raise ValueError("anthropic/replicate errors input must be an object")
    provider = argument.get("provider")
    model = argument.get("model")
    if not isinstance(model, str):
        raise ValueError("model must be a string")
    error_str = argument.get("error_str")
    if not isinstance(error_str, str):
        raise ValueError("error_str must be a string")
    status_code = argument.get("status_code")
    if status_code is not None and not isinstance(status_code, int):
        raise ValueError("status_code must be an integer or null")

    if provider == "anthropic":
        return _run_anthropic(model, error_str, status_code)
    if provider == "replicate":
        exception_type = argument.get("exception_type")
        if not isinstance(exception_type, str):
            raise ValueError("exception_type must be a string")
        return _run_replicate(model, error_str, exception_type, status_code)
    raise ValueError("provider must be anthropic or replicate")
