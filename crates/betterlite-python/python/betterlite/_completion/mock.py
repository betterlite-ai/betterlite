import asyncio
import time
from typing import Final, NoReturn, Protocol, runtime_checkable

from pydantic import JsonValue

from betterlite._completion.arguments import JSON, CallValue, TimeoutValues, timeout_seconds
from betterlite.exceptions import (
    APIError, AuthenticationError, BadGatewayError, BadRequestError,
    ContextWindowExceededError, InternalServerError, MockException, NotFoundError,
    RateLimitError, ServiceUnavailableError, Timeout,
)
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


def _is_api_error(error: Exception) -> bool:
    # openai.APIError without importing openai; betterlite's mirror plays that base for our classes
    if isinstance(error, APIError):
        return True
    return any(
        base.__module__ == "openai" and base.__name__ == "APIError"
        for base in type(error).__mro__
    )


def provider_name(value: CallValue) -> str:
    # LiteLLM's `custom_llm_provider or "openai"`: an absent or empty name is openai
    match value:
        case str() if len(value) > 0:
            return value
        case object():
            return "openai"


def _exception_provider(provider: str) -> str:
    if provider == "openai":
        return "OpenAIException"
    return f"{provider[0].upper()}{provider[1:]}Exception"


def _mapped_mock(error: MockException, model: str, provider: str) -> NoReturn:
    # the completion boundary maps a mock exception by status, as LiteLLM's exception_type does
    text = f"{_exception_provider(provider)} - {error.message}"
    status = error.status_code
    if status in (408, 504):
        raise Timeout(f"Timeout Error: {text}", model, provider)
    if status in (400, 422):
        raise BadRequestError(text, model, provider)
    if status == 401:
        raise AuthenticationError(
            f"AuthenticationError: {text}", llm_provider=provider, model=model
        )
    if status == 404:
        raise NotFoundError(f"NotFoundError: {text}", model, provider)
    if status == 429:
        raise RateLimitError(f"RateLimitError: {text}", llm_provider=provider, model=model)
    if status == 500:
        raise InternalServerError(
            f"InternalServerError: {text}", llm_provider=provider, model=model
        )
    if status == 502:
        raise BadGatewayError(f"BadGatewayError: {text}", llm_provider=provider, model=model)
    if status == 503:
        raise ServiceUnavailableError(
            f"ServiceUnavailableError: {text}", llm_provider=provider, model=model
        )
    raise APIError(status, f"APIError: {text}", provider, model, request=error.request)


def raise_mock_exception(error: Exception, model: str, provider: str) -> NoReturn:
    # _handle_mock_potential_exceptions: an APIError is raised as is, anything else is a mock error
    if _is_api_error(error):
        raise error
    status = error.status_code if isinstance(error, _StatusCodeProvider) else 500
    message = error.text if isinstance(error, _TextProvider) else str(error)
    llm_provider = error.llm_provider if isinstance(error, _ProviderName) else provider
    mock = MockException(
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
        raise RateLimitError("this is a mock rate limit error", provider, model)
    if text == "litellm.ContextWindowExceededError":
        raise ContextWindowExceededError(
            "this is a mock context window exceeded error", model, provider
        )
    if text == "litellm.InternalServerError":
        raise InternalServerError("this is a mock internal server error", provider, model)
    if text.startswith("Exception: content_filter_policy"):
        _mapped_mock(
            MockException(
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
            MockException(
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
    match value:
        case ModelResponse():
            return JSON.validate_python(value.model_dump())
        case Exception():
            raise_mock_exception(value, model, provider)
        case str():
            raise_text_mock(value, model, provider)
            return value
        case object():
            return JSON.validate_python(value)


def raise_mock_timeout(timeout: float | int | str | TimeoutValues, model: str) -> NoReturn:
    # _handle_mock_timeout: sleep the timeout first, then raise the Timeout LiteLLM raises
    seconds = timeout_seconds(timeout)
    if seconds is not None:
        time.sleep(seconds)
    raise Timeout("This is a mock timeout error", model, "openai")


async def araise_mock_timeout(timeout: float | int | str | TimeoutValues, model: str) -> NoReturn:
    # the awaitable call's own timeout mock, so the event loop is not blocked
    seconds = timeout_seconds(timeout)
    if seconds is not None:
        await asyncio.sleep(seconds)
    raise Timeout("This is a mock timeout error", model, "openai")
