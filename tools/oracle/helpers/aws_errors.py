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


_FAMILIES: dict[str, str] = {
    "AuthenticationError": "authentication",
    "ContextWindowExceededError": "context_window_exceeded",
    "RateLimitError": "rate_limit",
    "InternalServerError": "internal_server",
    "BadRequestError": "bad_request",
    "PermissionDeniedError": "permission_denied",
    "NotFoundError": "not_found",
    "Timeout": "timeout",
    "ServiceUnavailableError": "service_unavailable",
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


def _run(
    mapper: Mapper,
    *,
    model: str,
    custom_llm_provider: str,
    error_str: str,
    status_code: int | None,
) -> JsonValue:
    cause = SimpleNamespace(
        message=error_str,
        response=httpx.Response(
            status_code=status_code or 500,
            request=httpx.Request(method="GET", url="https://example.com"),
        ),
    )
    if status_code is not None:
        cause.status_code = status_code
    try:
        mapper(
            model=model,
            original_exception=cause,
            custom_llm_provider=custom_llm_provider,
            error_str=error_str,
            exception_type="APIError",
            exception_provider=custom_llm_provider,
            extra_information="",
        )
    except openai.APIError as error:
        family = _FAMILIES[type(error).__name__]
        provider = cast(str, getattr(error, "llm_provider", custom_llm_provider))
        context = _context(model, provider)
        result: JsonValue
        if family == "rate_limit":
            result = {
                "family": family,
                "context": context,
                "category": "vendor_rate_limit",
                "dimension": None,
                "headers": {},
            }
        elif family == "timeout":
            mapped = cast(MappedError, error)
            result = {"family": family, "context": context, "original_status": mapped.status_code}
        else:
            result = {"family": family, **context}
        return {"result": result}
    return {"result": None}


def run(argument: JsonValue) -> JsonValue:
    if not isinstance(argument, dict):
        raise ValueError("aws errors input must be an object")
    provider = argument.get("provider")
    if provider not in ("bedrock", "sagemaker"):
        raise ValueError("provider must be 'bedrock' or 'sagemaker'")
    model = argument.get("model")
    if not isinstance(model, str):
        raise ValueError("model must be a string")
    custom_llm_provider = argument.get("custom_llm_provider")
    if not isinstance(custom_llm_provider, str):
        raise ValueError("custom_llm_provider must be a string")
    error_str = argument.get("error_str")
    if not isinstance(error_str, str):
        raise ValueError("error_str must be a string")
    status_code = argument.get("status_code")
    if status_code is not None and not isinstance(status_code, int):
        raise ValueError("status_code must be an integer or null")

    bedrock_mapper = mapping._map_bedrock_exception  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    sagemaker_mapper = mapping._map_sagemaker_exception  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    mapper = cast(Mapper, bedrock_mapper if provider == "bedrock" else sagemaker_mapper)
    return _run(
        mapper,
        model=model,
        custom_llm_provider=custom_llm_provider,
        error_str=error_str,
        status_code=status_code,
    )
