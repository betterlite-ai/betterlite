from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack, cast

import openai
from litellm.exceptions import APIError as LitellmApiError
from litellm.litellm_core_utils import exception_mapping_utils as mapping

JsonValue: TypeAlias = None | bool | int | float | str | list["JsonValue"] | dict[str, "JsonValue"]


class CloudflareInput(TypedDict):
    provider: Literal["cloudflare"]
    model: NotRequired[str | None]
    error_str: str


class CohereInput(TypedDict):
    provider: Literal["cohere"]
    model: NotRequired[str | None]
    error_str: str
    exception_type: str
    status_code: NotRequired[int | None]


class HuggingfaceInput(TypedDict):
    provider: Literal["huggingface"]
    model: NotRequired[str | None]
    error_str: str
    status_code: NotRequired[int | None]


class Ai21Input(TypedDict):
    provider: Literal["ai21"]
    model: NotRequired[str | None]
    message: NotRequired[str | None]
    status_code: NotRequired[int | None]


Input = CloudflareInput | CohereInput | HuggingfaceInput | Ai21Input


# stands in for the SDK exception these mappers normally introspect via getattr/hasattr
class _ProviderCauseError(Exception):
    def __init__(
        self,
        error_str: str,
        *,
        status_code: int | None = None,
        message: str | None = None,
    ) -> None:
        super().__init__(error_str)
        if status_code is not None:
            self.status_code = status_code
        if message is not None:
            self.message = message
        self._error_str = error_str

    def __str__(self) -> str:
        return self._error_str


class _MapperArguments(TypedDict):
    model: str | None
    original_exception: object
    custom_llm_provider: str
    error_str: str
    exception_type: str
    exception_provider: str
    extra_information: str


class _Mapper(Protocol):
    def __call__(self, **kwargs: Unpack[_MapperArguments]) -> None: ...


def _context(model: str | None, provider: str) -> dict[str, JsonValue]:
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


def _classified(
    family: str, model: str | None, provider: str, status: int | None = None
) -> JsonValue:
    context = _context(model, provider)
    if family == "rate_limit":
        return {
            "family": family,
            "context": context,
            "category": "vendor_rate_limit",
            "dimension": None,
            "headers": {},
        }
    if family == "timeout":
        return {"family": family, "context": context, "original_status": 408}
    if family == "api":
        return {"family": family, "context": context, "status": status}
    return {"family": family, **context}


def _unclassified() -> JsonValue:
    return {
        "family": "unclassified",
        "message": "ignored",
        "model": None,
        "provider": None,
        "max_retries": None,
        "num_retries": None,
        "body_type": None,
        "body_param": None,
        "body_code": None,
    }


_CLOUDFLARE_FAMILIES: dict[str, str] = {
    "AuthenticationError": "authentication",
    "BadRequestError": "bad_request",
}

_COHERE_FAMILIES: dict[str, str] = {
    "AuthenticationError": "authentication",
    "BadRequestError": "bad_request",
    "ContextWindowExceededError": "context_window_exceeded",
    "InternalServerError": "internal_server",
    "Timeout": "timeout",
    "RateLimitError": "rate_limit",
}

_HUGGINGFACE_FAMILIES: dict[str, str] = {
    "ContextWindowExceededError": "context_window_exceeded",
    "BadRequestError": "bad_request",
    "RateLimitError": "rate_limit",
    "AuthenticationError": "authentication",
    "Timeout": "timeout",
    "ServiceUnavailableError": "service_unavailable",
    "APIError": "api",
}

_AI21_FAMILIES: dict[str, str] = {
    "ContextWindowExceededError": "context_window_exceeded",
    "BadRequestError": "bad_request",
    "AuthenticationError": "authentication",
    "Timeout": "timeout",
    "RateLimitError": "rate_limit",
    "APIError": "api",
}


def _run_cloudflare(input_value: CloudflareInput) -> JsonValue:
    model = input_value.get("model")
    error_str = input_value["error_str"]
    cause = _ProviderCauseError(error_str, message=error_str)
    call = cast(_Mapper, mapping._map_cloudflare_exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        call(
            model=model,
            original_exception=cause,
            custom_llm_provider="cloudflare",
            error_str=error_str,
            exception_type="APIError",
            exception_provider="cloudflare",
            extra_information="",
        )
    except openai.APIError as error:
        family = _CLOUDFLARE_FAMILIES[type(error).__name__]
        return {"result": _classified(family, model, "cloudflare")}
    return {"result": None}


def _run_cohere(input_value: CohereInput) -> JsonValue:
    model = input_value.get("model")
    error_str = input_value["error_str"]
    exception_type = input_value["exception_type"]
    status_code = input_value.get("status_code")
    cause = _ProviderCauseError(error_str, status_code=status_code, message=error_str)
    call = cast(_Mapper, mapping._map_cohere_exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        call(
            model=model,
            original_exception=cause,
            custom_llm_provider="cohere",
            error_str=error_str,
            exception_type=exception_type,
            exception_provider="cohere",
            extra_information="",
        )
    except openai.APIError as error:
        return {"result": _classified(_COHERE_FAMILIES[type(error).__name__], model, "cohere")}
    except _ProviderCauseError:
        return {"result": _unclassified()}
    return {"result": None}


def _run_huggingface(input_value: HuggingfaceInput) -> JsonValue:
    model = input_value.get("model")
    error_str = input_value["error_str"]
    status_code = input_value.get("status_code")
    cause = _ProviderCauseError(error_str, status_code=status_code, message=error_str)
    call = cast(_Mapper, mapping._map_huggingface_exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        call(
            model=model,
            original_exception=cause,
            custom_llm_provider="huggingface",
            error_str=error_str,
            exception_type="APIError",
            exception_provider="huggingface",
            extra_information="",
        )
    except openai.APIError as error:
        family = _HUGGINGFACE_FAMILIES[type(error).__name__]
        status = cast(LitellmApiError, error).status_code if family == "api" else None
        return {"result": _classified(family, model, "huggingface", status)}
    return {"result": None}


def _run_ai21(input_value: Ai21Input) -> JsonValue:
    model = input_value.get("model")
    message = input_value.get("message")
    status_code = input_value.get("status_code")
    cause = _ProviderCauseError(message or "", status_code=status_code, message=message or "")
    call = cast(_Mapper, mapping._map_ai21_exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        call(
            model=model,
            original_exception=cause,
            custom_llm_provider="ai21",
            error_str=message or "",
            exception_type="APIError",
            exception_provider="ai21",
            extra_information="",
        )
    except openai.APIError as error:
        family = _AI21_FAMILIES[type(error).__name__]
        status = cast(LitellmApiError, error).status_code if family == "api" else None
        return {"result": _classified(family, model, "ai21", status)}
    return {"result": None}


def run(input_value: Input) -> JsonValue:
    if input_value["provider"] == "cloudflare":
        return _run_cloudflare(input_value)
    if input_value["provider"] == "cohere":
        return _run_cohere(input_value)
    if input_value["provider"] == "huggingface":
        return _run_huggingface(input_value)
    return _run_ai21(input_value)
