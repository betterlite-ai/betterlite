import asyncio
import time
from http import HTTPStatus
from typing import Final, NoReturn, Protocol, runtime_checkable

from pydantic import JsonValue

from betterlite import exceptions
from betterlite._completion.arguments import JSON, CallValue, is_response_format_class
from betterlite.exceptions._http import Request
from betterlite.types.utils import ModelResponse

# the request httpx request LiteLLM builds beside a mock exception
MOCK_REQUEST_URL: Final = "https://api.openai.com/v1/"


@runtime_checkable
class _StatusCodeProvider(Protocol):
    status_code: int


@runtime_checkable
class _TextProvider(Protocol):
    text: str


@runtime_checkable
class _ProviderName(Protocol):
    llm_provider: str


# openai.APIError without importing openai: betterlite's mirror plays that base for our classes
def _is_api_error(error: Exception) -> bool:
    if isinstance(error, exceptions.APIError):  # ast-grep-ignore: no-isinstance
        return True
    return any(
        base.__module__ == "openai" and base.__name__ == "APIError"
        for base in error.__class__.__mro__
    )


def provider_name(value: CallValue) -> str:
    # LiteLLM's `custom_llm_provider or "openai"`: an absent or empty name is openai
    if is_response_format_class(value):
        return "openai"
    match value:
        case str() if len(value) > 0:
            return value
        case object():
            return "openai"


def _mapped_mock(error: exceptions.MockException, model: str, provider: str) -> NoReturn:
    # the completion boundary maps a mock exception by status, as LiteLLM's exception_type does
    name = "OpenAI" if provider == "openai" else f"{provider[0].upper()}{provider[1:]}"
    text = f"{name}Exception - {error.message}"
    status = error.status_code
    if status in (HTTPStatus.REQUEST_TIMEOUT, HTTPStatus.GATEWAY_TIMEOUT):
        raise exceptions.Timeout(f"Timeout Error: {text}", model, provider)
    if status in (HTTPStatus.BAD_REQUEST, HTTPStatus.UNPROCESSABLE_ENTITY):
        raise exceptions.BadRequestError(text, model, provider)
    if status == HTTPStatus.UNAUTHORIZED:
        raise exceptions.AuthenticationError(
            f"AuthenticationError: {text}", llm_provider=provider, model=model
        )
    if status == HTTPStatus.NOT_FOUND:
        raise exceptions.NotFoundError(f"NotFoundError: {text}", model, provider)
    if status == HTTPStatus.TOO_MANY_REQUESTS:
        raise exceptions.RateLimitError(
            f"RateLimitError: {text}", llm_provider=provider, model=model
        )
    if status == HTTPStatus.INTERNAL_SERVER_ERROR:
        raise exceptions.InternalServerError(
            f"InternalServerError: {text}", llm_provider=provider, model=model
        )
    if status == HTTPStatus.BAD_GATEWAY:
        raise exceptions.BadGatewayError(
            f"BadGatewayError: {text}", llm_provider=provider, model=model
        )
    if status == HTTPStatus.SERVICE_UNAVAILABLE:
        raise exceptions.ServiceUnavailableError(
            f"ServiceUnavailableError: {text}", llm_provider=provider, model=model
        )
    raise exceptions.APIError(
        status, f"APIError: {text}", provider, model, request=error.request
    )


def raise_mock_exception(error: Exception, model: str, provider: str) -> NoReturn:
    # _handle_mock_potential_exceptions: an APIError is raised as is, anything else is a mock error
    if _is_api_error(error):
        raise error
    # ast-grep-ignore: no-isinstance
    status = error.status_code if isinstance(error, _StatusCodeProvider) else 500
    # ast-grep-ignore: no-isinstance
    message = error.text if isinstance(error, _TextProvider) else str(error)
    # ast-grep-ignore: no-isinstance
    llm_provider = error.llm_provider if isinstance(error, _ProviderName) else provider
    mock = exceptions.MockException(
        status_code=status,
        message=message,
        llm_provider=llm_provider,
        model=model,
        request=Request(method="POST", url=MOCK_REQUEST_URL),
    )
    _mapped_mock(mock, model, provider)


def raise_text_mock(text: str, model: str, provider: str) -> None:
    # the four failure strings mock_completion turns into errors before any request is built
    if text == "litellm.RateLimitError":
        raise exceptions.RateLimitError("this is a mock rate limit error", provider, model)
    if text == "litellm.ContextWindowExceededError":
        raise exceptions.ContextWindowExceededError(
            "this is a mock context window exceeded error", model, provider
        )
    if text == "litellm.InternalServerError":
        raise exceptions.InternalServerError(
            "this is a mock internal server error", provider, model
        )
    if text.startswith("Exception: content_filter_policy"):
        _mapped_mock(
            exceptions.MockException(
                status_code=400,
                message=text,
                llm_provider="azure",
                model=model,
                request=Request(method="POST", url=MOCK_REQUEST_URL),
            ),
            model,
            provider,
        )
    if text.startswith("Exception: mock_streaming_error"):
        _mapped_mock(
            exceptions.MockException(
                status_code=529,
                message="This is a mock error raised mid-stream",
                llm_provider="anthropic",
                model=model,
                request=Request(method="POST", url=MOCK_REQUEST_URL),
            ),
            model,
            provider,
        )


def mock_response_value(value: CallValue, model: str, provider: str) -> JsonValue:
    # a ModelResponse becomes the JSON a mock object is built from; an Exception is raised
    if is_response_format_class(value):
        return JSON.validate_python(value)
    match value:
        case ModelResponse():
            # the Rust mock object rejects unknown keys, so the dump leaves its nulls out
            return JSON.validate_python(value.model_dump(exclude_none=True))
        case Exception():
            raise_mock_exception(value, model, provider)
        case str():
            raise_text_mock(value, model, provider)
            return value
        case object():
            return JSON.validate_python(value)


def _connect_seconds(timeout: JsonValue) -> float | None:
    # a mock timeout sleeps the connect phase of a phases mapping, and a number is its own delay
    match timeout:
        case int() | float():
            return float(timeout)
        case dict():
            connect = timeout.get("connect")
            match connect:
                case int() | float():
                    return float(connect)
                case object():
                    return None
        case object():
            return None


def raise_mock_timeout(timeout_value: JsonValue, model: str) -> NoReturn:
    # _handle_mock_timeout: sleep the timeout first, then raise the Timeout LiteLLM raises
    seconds = _connect_seconds(timeout_value)
    if seconds is not None:
        time.sleep(seconds)
    raise exceptions.Timeout("This is a mock timeout error", model, "openai")


async def araise_mock_timeout(timeout_value: JsonValue, model: str) -> NoReturn:
    # the awaitable call's own timeout mock, so the event loop is not blocked
    seconds = _connect_seconds(timeout_value)
    if seconds is not None:
        await asyncio.sleep(seconds)
    raise exceptions.Timeout("This is a mock timeout error", model, "openai")
