import inspect
from typing import Annotated, Final

import httpx
import litellm.exceptions as their_errors
import pytest
from pydantic import BaseModel, BeforeValidator, ConfigDict, Field, JsonValue

import betterlite.exceptions as our_errors
from betterlite.exceptions._http import Request as OurRequest
from betterlite.exceptions._http import Response as OurResponse

_MESSAGE: Final[str] = "upstream timed out"
_MID: Final[str] = "stream failed after the first chunk"
_MODEL: Final[str] = "gpt-4o"
_PROVIDER: Final[str] = "openai"
_DEBUG: Final[str] = "model=gpt-4o, provider=openai"
_URL: Final[str] = "https://api.openai.com/v1"
_HEADERS: Final[dict[str, str]] = {"x-request-id": "req_2", "retry-after": "9"}
_BODY: Final[dict[str, JsonValue]] = {
    "code": "internal_code",
    "param": "temperature",
    "type": "server_error",
}
_DETAIL: Final[dict[str, JsonValue]] = {"code": "quota_exceeded"}
_RATE_BODY: Final[dict[str, JsonValue]] = {
    "code": "original_code",
    "param": "top_p",
    "type": "original_type",
}
_CONTENT: Final[bytes] = b"slow down"
_BYTES: Final[bytes] = b"raw"
_LATIN_CONTENT: Final[bytes] = b"r\xe9sum\xe9"
_LATIN_HEADERS: Final[dict[str, str]] = {
    "content-type": "text/plain; charset=latin-1",
    "x-request-id": "req_3",
}
_BROKEN_CONTENT: Final[bytes] = b"\xff"
_UNKNOWN_HEADERS: Final[dict[str, str]] = {
    "content-type": "text/plain; charset=x-unknown",
    "x-request-id": "req_4",
}
_THEIR_TARGET: Final[httpx.Response] = httpx.Response(
    status_code=429,
    headers=_HEADERS,
    request=httpx.Request(method="POST", url=_URL),
)
_OUR_TARGET: Final[OurResponse] = OurResponse(
    status_code=429,
    request=OurRequest(method="POST", url=_URL),
    headers=_HEADERS,
)
_THEIR_RATE: Final[httpx.Response] = httpx.Response(
    status_code=429,
    headers=_HEADERS,
    content=_CONTENT,
    request=httpx.Request(method="POST", url=_URL),
)
_OUR_RATE: Final[OurResponse] = OurResponse(
    status_code=429,
    request=OurRequest(method="POST", url=_URL),
    headers=_HEADERS,
    content=_CONTENT,
)
_THEIR_LATIN: Final[httpx.Response] = httpx.Response(
    status_code=429,
    headers=_LATIN_HEADERS,
    content=_LATIN_CONTENT,
    request=httpx.Request(method="POST", url=_URL),
)
_OUR_LATIN: Final[OurResponse] = OurResponse(
    status_code=429,
    request=OurRequest(method="POST", url=_URL),
    headers=_LATIN_HEADERS,
    content=_LATIN_CONTENT,
)
_THEIR_BROKEN: Final[httpx.Response] = httpx.Response(
    status_code=429,
    headers=_HEADERS,
    content=_BROKEN_CONTENT,
    request=httpx.Request(method="POST", url=_URL),
)
_OUR_BROKEN: Final[OurResponse] = OurResponse(
    status_code=429,
    request=OurRequest(method="POST", url=_URL),
    headers=_HEADERS,
    content=_BROKEN_CONTENT,
)
_THEIR_UNKNOWN: Final[httpx.Response] = httpx.Response(
    status_code=429,
    headers=_UNKNOWN_HEADERS,
    content=_BROKEN_CONTENT,
    request=httpx.Request(method="POST", url=_URL),
)
_OUR_UNKNOWN: Final[OurResponse] = OurResponse(
    status_code=429,
    request=OurRequest(method="POST", url=_URL),
    headers=_UNKNOWN_HEADERS,
    content=_BROKEN_CONTENT,
)
_URL_TEXT = Annotated[str, BeforeValidator(str)]
_DEFAULT_TEXT = Annotated[str, BeforeValidator(repr)]
_DROPPED_MODULES: Final[frozenset[str]] = frozenset({"openai", "httpx"})
_OPENAI_PROVIDER: Final[str] = "openai"
_OPENAI_TEXT: Final[str] = ""
_OPENAI_REPR: Final[str] = "OpenAIError()"
_OPENAI_PARAMS: Final[list[str]] = ["original_exception"]
_OPENAI_MRO: Final[list[str]] = ["OpenAIError", "Exception", "BaseException", "object"]
_NONE_TEXT: Final[str] = "None"


class _RequestView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    method: str
    url: _URL_TEXT


class _ResponseView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int
    headers: dict[str, str]
    text: str
    request: _RequestView


class _ParameterView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    name: str
    kind: _DEFAULT_TEXT
    default: _DEFAULT_TEXT


class _OpenAIView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    llm_provider: str
    args: tuple[JsonValue, ...]


class _ServerView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    message: str | None = None
    status_code: int | None = None
    llm_provider: str | None = None
    model: str | None = None
    num_retries: int | None = None
    max_retries: int | None = None
    litellm_debug_info: str | None = None
    code: JsonValue
    param: JsonValue
    type: JsonValue = None
    body: JsonValue | bytes = None
    headers: dict[str, str] | None = None
    args: tuple[JsonValue, ...] = ()
    response: _ResponseView | None = None
    request: _RequestView | None = None
    request_id: str | None = None
    category: str | None = None
    rate_limit_type: str | None = None
    detail: JsonValue = None
    raw_response: str | None = None
    schema_text: str | None = Field(default=None, validation_alias="schema")
    generated_content: str | None = None
    is_pre_first_chunk: bool | None = None


_CASES = [
    pytest.param(
        their_errors.Timeout(message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER),
        our_errors.Timeout(message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER),
        id="timeout-defaults",
    ),
    pytest.param(
        their_errors.Timeout(
            message=_MESSAGE,
            model=_MODEL,
            llm_provider=_PROVIDER,
            litellm_debug_info=_DEBUG,
            max_retries=5,
            num_retries=3,
            headers=_HEADERS,
            exception_status_code=429,
            response=_THEIR_TARGET,
        ),
        our_errors.Timeout(
            message=_MESSAGE,
            model=_MODEL,
            llm_provider=_PROVIDER,
            litellm_debug_info=_DEBUG,
            max_retries=5,
            num_retries=3,
            headers=_HEADERS,
            exception_status_code=429,
            response=_OUR_TARGET,
        ),
        id="timeout-full",
    ),
    pytest.param(
        their_errors.Timeout(
            message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, headers=_HEADERS
        ),
        our_errors.Timeout(
            message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, headers=_HEADERS
        ),
        id="timeout-given-headers",
    ),
    pytest.param(
        their_errors.RateLimitError(message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL),
        our_errors.RateLimitError(message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL),
        id="rate-limit-defaults",
    ),
    pytest.param(
        their_errors.RateLimitError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            response=_THEIR_RATE,
            litellm_debug_info=_DEBUG,
            max_retries=6,
            num_retries=2,
            category="litellm_rate_limit",
            rate_limit_type="tokens",
            headers={"retry-after": "7", "x-limit": "10"},
            detail=_DETAIL,
            body=_RATE_BODY,
        ),
        our_errors.RateLimitError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            response=_OUR_RATE,
            litellm_debug_info=_DEBUG,
            max_retries=6,
            num_retries=2,
            category="litellm_rate_limit",
            rate_limit_type="tokens",
            headers={"retry-after": "7", "x-limit": "10"},
            detail=_DETAIL,
            body=_RATE_BODY,
        ),
        id="rate-limit-full",
    ),
    pytest.param(
        their_errors.RateLimitError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            category=their_errors.RateLimitErrorCategory.LITELLM_BATCH_RATE_LIMIT,
            rate_limit_type=their_errors.RateLimitType.BUDGET,
        ),
        our_errors.RateLimitError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            category=our_errors.RateLimitErrorCategory.LITELLM_BATCH_RATE_LIMIT,
            rate_limit_type=our_errors.RateLimitType.BUDGET,
        ),
        id="rate-limit-enum",
    ),
    pytest.param(
        their_errors.RateLimitError(
            message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, response=_THEIR_LATIN
        ),
        our_errors.RateLimitError(
            message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, response=_OUR_LATIN
        ),
        id="rate-limit-latin1",
    ),
    pytest.param(
        their_errors.RateLimitError(
            message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, response=_THEIR_BROKEN
        ),
        our_errors.RateLimitError(
            message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, response=_OUR_BROKEN
        ),
        id="rate-limit-replace",
    ),
    pytest.param(
        their_errors.RateLimitError(
            message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, response=_THEIR_UNKNOWN
        ),
        our_errors.RateLimitError(
            message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, response=_OUR_UNKNOWN
        ),
        id="rate-limit-unknown-charset",
    ),
    pytest.param(
        their_errors.ServiceUnavailableError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            response=_THEIR_TARGET,
            litellm_debug_info=_DEBUG,
            max_retries=4,
            num_retries=2,
        ),
        our_errors.ServiceUnavailableError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            response=_OUR_TARGET,
            litellm_debug_info=_DEBUG,
            max_retries=4,
            num_retries=2,
        ),
        id="service-unavailable",
    ),
    pytest.param(
        their_errors.BadGatewayError(message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL),
        our_errors.BadGatewayError(message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL),
        id="bad-gateway",
    ),
    pytest.param(
        their_errors.InternalServerError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            response=_THEIR_TARGET,
            litellm_debug_info=_DEBUG,
            max_retries=2,
            num_retries=1,
            body=_BODY,
        ),
        our_errors.InternalServerError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            response=_OUR_TARGET,
            litellm_debug_info=_DEBUG,
            max_retries=2,
            num_retries=1,
            body=_BODY,
        ),
        id="internal-server",
    ),
    pytest.param(
        their_errors.InternalServerError(
            message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, body=_BYTES
        ),
        our_errors.InternalServerError(
            message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, body=_BYTES
        ),
        id="internal-server-bytes-body",
    ),
    pytest.param(
        their_errors.APIError(
            status_code=502, message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, body=_BODY
        ),
        our_errors.APIError(
            status_code=502, message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, body=_BODY
        ),
        id="api-error-default-request",
    ),
    pytest.param(
        their_errors.APIError(
            status_code=418,
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            request=httpx.Request(method="PUT", url="https://example.test/put"),
            litellm_debug_info=_DEBUG,
            max_retries=1,
            num_retries=1,
        ),
        our_errors.APIError(
            status_code=418,
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            request=OurRequest(method="PUT", url="https://example.test/put"),
            litellm_debug_info=_DEBUG,
            max_retries=1,
            num_retries=1,
        ),
        id="api-error-given-request",
    ),
    pytest.param(
        their_errors.APIConnectionError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            request=httpx.Request(method="PUT", url="https://example.test/ignored"),
            litellm_debug_info=_DEBUG,
            max_retries=2,
            num_retries=1,
        ),
        our_errors.APIConnectionError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            request=OurRequest(method="PUT", url="https://example.test/ignored"),
            litellm_debug_info=_DEBUG,
            max_retries=2,
            num_retries=1,
        ),
        id="api-connection",
    ),
    pytest.param(
        their_errors.APIResponseValidationError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            litellm_debug_info=_DEBUG,
            max_retries=1,
            num_retries=1,
        ),
        our_errors.APIResponseValidationError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            litellm_debug_info=_DEBUG,
            max_retries=1,
            num_retries=1,
        ),
        id="api-response-validation",
    ),
    pytest.param(
        their_errors.JSONSchemaValidationError(
            model=_MODEL,
            llm_provider=_PROVIDER,
            raw_response="not json",
            schema='{"type": "object"}',
        ),
        our_errors.JSONSchemaValidationError(
            model=_MODEL,
            llm_provider=_PROVIDER,
            raw_response="not json",
            schema='{"type": "object"}',
        ),
        id="json-schema",
    ),
    pytest.param(
        their_errors.MockException(
            status_code=418, message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL
        ),
        our_errors.MockException(
            status_code=418, message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL
        ),
        id="mock-exception",
    ),
    pytest.param(
        their_errors.MockException(
            status_code=500,
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            request=httpx.Request(method="GET", url="https://example.test/get"),
            litellm_debug_info=_DEBUG,
            max_retries=3,
            num_retries=1,
        ),
        our_errors.MockException(
            status_code=500,
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            request=OurRequest(method="GET", url="https://example.test/get"),
            litellm_debug_info=_DEBUG,
            max_retries=3,
            num_retries=1,
        ),
        id="mock-exception-request",
    ),
    pytest.param(
        their_errors.MidStreamFallbackError(message=_MID, model=_MODEL, llm_provider=_PROVIDER),
        our_errors.MidStreamFallbackError(message=_MID, model=_MODEL, llm_provider=_PROVIDER),
        id="mid-stream-defaults",
    ),
    pytest.param(
        their_errors.MidStreamFallbackError(
            message=_MID,
            model=_MODEL,
            llm_provider=_PROVIDER,
            response=_THEIR_TARGET,
            litellm_debug_info=_DEBUG,
            max_retries=2,
            num_retries=1,
            generated_content="partial answer",
            is_pre_first_chunk=True,
        ),
        our_errors.MidStreamFallbackError(
            message=_MID,
            model=_MODEL,
            llm_provider=_PROVIDER,
            response=_OUR_TARGET,
            litellm_debug_info=_DEBUG,
            max_retries=2,
            num_retries=1,
            generated_content="partial answer",
            is_pre_first_chunk=True,
        ),
        id="mid-stream-full",
    ),
    pytest.param(
        their_errors.MidStreamFallbackError(
            message=_MID,
            model=_MODEL,
            llm_provider=_PROVIDER,
            original_exception=their_errors.Timeout(
                message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER
            ),
        ),
        our_errors.MidStreamFallbackError(
            message=_MID,
            model=_MODEL,
            llm_provider=_PROVIDER,
            original_exception=our_errors.Timeout(
                message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER
            ),
        ),
        id="mid-stream-timeout",
    ),
    pytest.param(
        their_errors.MidStreamFallbackError(
            message=_MID,
            model=_MODEL,
            llm_provider=_PROVIDER,
            original_exception=their_errors.RateLimitError(
                message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL
            ),
        ),
        our_errors.MidStreamFallbackError(
            message=_MID,
            model=_MODEL,
            llm_provider=_PROVIDER,
            original_exception=our_errors.RateLimitError(
                message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL
            ),
        ),
        id="mid-stream-rate-limit",
    ),
    pytest.param(
        their_errors.MidStreamFallbackError(
            message=_MID,
            model=_MODEL,
            llm_provider=_PROVIDER,
            original_exception=their_errors.BudgetExceededError(current_cost=1.5, max_budget=2.0),
        ),
        our_errors.MidStreamFallbackError(
            message=_MID,
            model=_MODEL,
            llm_provider=_PROVIDER,
            original_exception=our_errors.BudgetExceededError(current_cost=1.5, max_budget=2.0),
        ),
        id="mid-stream-budget",
    ),
    pytest.param(
        their_errors.MidStreamFallbackError(
            message=_MID,
            model=_MODEL,
            llm_provider=_PROVIDER,
            original_exception=RuntimeError("connection reset"),
        ),
        our_errors.MidStreamFallbackError(
            message=_MID,
            model=_MODEL,
            llm_provider=_PROVIDER,
            original_exception=RuntimeError("connection reset"),
        ),
        id="mid-stream-plain",
    ),
]


def _mro_names(error: BaseException) -> list[str]:
    classes = type(error).__mro__
    return [cls.__name__ for cls in classes if cls.__module__.split(".")[0] not in _DROPPED_MODULES]


def _signature_views(error: BaseException) -> list[_ParameterView]:
    signature = inspect.signature(type(error))
    return [_ParameterView.model_validate(parameter) for parameter in signature.parameters.values()]


def _check(theirs: BaseException, ours: BaseException) -> None:
    assert str(ours) == str(theirs)
    assert repr(ours) == repr(theirs)
    assert _ServerView.model_validate(ours) == _ServerView.model_validate(theirs)
    their_names = _mro_names(theirs)
    our_names = _mro_names(ours)
    assert our_names[0] == their_names[0]
    assert [name for name in their_names if name in our_names] == their_names
    assert _signature_views(ours) == _signature_views(theirs)


@pytest.mark.parametrize(("theirs", "ours"), _CASES)
def test_server_error_matches(theirs: BaseException, ours: BaseException) -> None:
    _check(
        theirs,
        ours,
    )


def test_openai_error_pinned() -> None:
    ours = our_errors.OpenAIError()
    ignored = our_errors.OpenAIError(original_exception=ValueError("bad output"))
    view = _OpenAIView.model_validate(ours)
    params = _signature_views(ours)
    assert str(ours) == _OPENAI_TEXT
    assert repr(ours) == _OPENAI_REPR
    assert view.llm_provider == _OPENAI_PROVIDER
    assert view.args == ()
    assert [param.name for param in params] == _OPENAI_PARAMS
    assert [param.default for param in params] == [_NONE_TEXT]
    our_names = _mro_names(ours)
    assert [name for name in _OPENAI_MRO if name in our_names] == _OPENAI_MRO
    assert _OpenAIView.model_validate(ignored) == view
    assert str(ignored) == _OPENAI_TEXT
