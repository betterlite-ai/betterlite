import inspect
from typing import Annotated, Final

import httpx
import litellm.exceptions as their_errors
import pytest
from pydantic import BaseModel, BeforeValidator, ConfigDict, JsonValue

import betterlite.exceptions as our_errors
from betterlite.exceptions._http import Request as OurRequest
from betterlite.exceptions._http import Response as OurResponse

_MESSAGE: Final[str] = "invalid api key"
_CONTEXT: Final[str] = "context window of 4096 tokens exceeded"
_REJECTED: Final[str] = "rejected by the request guardrail"
_POLICY: Final[str] = "the prompt violates the content policy"
_PARAMS: Final[str] = "response_format is not supported"
_INVALID: Final[str] = "invalid request shape"
_MODEL: Final[str] = "gpt-4o"
_PROVIDER: Final[str] = "openai"
_DEBUG: Final[str] = "model=gpt-4o, provider=openai"
_HEADERS: Final[dict[str, str]] = {"x-request-id": "req_1", "retry-after": "3"}
_MIXED_HEADERS: Final[dict[str, str]] = {"X-Request-ID": "req_5", "Retry-After": "5"}
_BODY: Final[dict[str, JsonValue]] = {
    "code": "bad_code",
    "param": "messages",
    "type": "invalid_request_error",
}
_BYTES: Final[bytes] = b"raw"
_POLICY_FIELDS: Final[dict[str, JsonValue]] = {"categories": ["violence"], "score": 0.9}
_REQUEST_DATA: Final[dict[str, JsonValue]] = {
    "model": _MODEL,
    "messages": [{"role": "user", "content": "hi"}],
}
_THEIR_RESPONSE: Final[httpx.Response] = httpx.Response(
    status_code=429,
    headers=_HEADERS,
    request=httpx.Request(method="POST", url="https://api.openai.com/v1"),
)
_OUR_RESPONSE: Final[OurResponse] = OurResponse(
    status_code=429,
    request=OurRequest(method="POST", url="https://api.openai.com/v1"),
    headers=_HEADERS,
)
_THEIR_MIXED: Final[httpx.Response] = httpx.Response(
    status_code=429,
    headers=_MIXED_HEADERS,
    request=httpx.Request(method="POST", url="https://api.openai.com/v1"),
)
_OUR_MIXED: Final[OurResponse] = OurResponse(
    status_code=429,
    request=OurRequest(method="POST", url="https://api.openai.com/v1"),
    headers=_MIXED_HEADERS,
)
_THEIR_BARE: Final[httpx.Response] = httpx.Response(status_code=429)
_OUR_BARE: Final[OurResponse] = OurResponse(status_code=429)
_URL_TEXT = Annotated[str, BeforeValidator(str)]
_DEFAULT_TEXT = Annotated[str, BeforeValidator(repr)]
_DROPPED_MODULES: Final[frozenset[str]] = frozenset({"openai", "httpx"})
_MOCK_METHOD: Final[str] = "GET"
_MOCK_URL: Final[str] = "https://litellm.ai"
_BAD_REQUEST_STATUS: Final[int] = 400
_INVALID_REPR: Final[str] = f"InvalidRequestError({_INVALID!r})"
_INVALID_PARAMS: Final[list[str]] = ["message", "model", "llm_provider"]
_EMPTY_PARAM_DEFAULT: Final[str] = "<class 'inspect._empty'>"


class _RequestView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    method: str
    url: _URL_TEXT


class _ResponseView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int
    headers: dict[str, str]
    request: _RequestView


class _ParameterView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    name: str
    kind: _DEFAULT_TEXT
    default: _DEFAULT_TEXT


class _BodyView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    code: JsonValue
    param: JsonValue
    type: JsonValue


class _StatusView(BaseModel):
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
    provider_specific_fields: dict[str, JsonValue] | None = None
    request_data: dict[str, JsonValue] | None = None
    failures: tuple[dict[str, JsonValue], ...] | None = None


_CASES = [
    pytest.param(
        their_errors.AuthenticationError(message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL),
        our_errors.AuthenticationError(message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL),
        id="authentication-defaults",
    ),
    pytest.param(
        their_errors.AuthenticationError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            response=_THEIR_RESPONSE,
            litellm_debug_info=_DEBUG,
            max_retries=4,
            num_retries=2,
        ),
        our_errors.AuthenticationError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            response=_OUR_RESPONSE,
            litellm_debug_info=_DEBUG,
            max_retries=4,
            num_retries=2,
        ),
        id="authentication-response",
    ),
    pytest.param(
        their_errors.AuthenticationError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            response=_THEIR_MIXED,
        ),
        our_errors.AuthenticationError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            response=_OUR_MIXED,
        ),
        id="authentication-mixed-headers",
    ),
    pytest.param(
        their_errors.NotFoundError(message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER),
        our_errors.NotFoundError(message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER),
        id="not-found-defaults",
    ),
    pytest.param(
        their_errors.NotFoundError(
            message=_MESSAGE,
            model=_MODEL,
            llm_provider=_PROVIDER,
            response=_THEIR_RESPONSE,
            max_retries=1,
            num_retries=1,
        ),
        our_errors.NotFoundError(
            message=_MESSAGE,
            model=_MODEL,
            llm_provider=_PROVIDER,
            response=_OUR_RESPONSE,
            max_retries=1,
            num_retries=1,
        ),
        id="not-found-response",
    ),
    pytest.param(
        their_errors.BadRequestError(
            message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, body=_BODY
        ),
        our_errors.BadRequestError(
            message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, body=_BODY
        ),
        id="bad-request-minimal",
    ),
    pytest.param(
        their_errors.BadRequestError(
            message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=_THEIR_RESPONSE
        ),
        our_errors.BadRequestError(
            message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=_OUR_RESPONSE
        ),
        id="bad-request-response",
    ),
    pytest.param(
        their_errors.BadRequestError(
            message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=_THEIR_BARE
        ),
        our_errors.BadRequestError(
            message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=_OUR_BARE
        ),
        id="bad-request-bare-response",
    ),
    pytest.param(
        their_errors.ImageFetchError(message=_MESSAGE),
        our_errors.ImageFetchError(message=_MESSAGE),
        id="image-fetch-defaults",
    ),
    pytest.param(
        their_errors.ImageFetchError(
            message=_MESSAGE,
            model=_MODEL,
            llm_provider=_PROVIDER,
            response=_THEIR_RESPONSE,
            litellm_debug_info=_DEBUG,
            max_retries=2,
            num_retries=1,
            body=_BODY,
        ),
        our_errors.ImageFetchError(
            message=_MESSAGE,
            model=_MODEL,
            llm_provider=_PROVIDER,
            response=_OUR_RESPONSE,
            litellm_debug_info=_DEBUG,
            max_retries=2,
            num_retries=1,
            body=_BODY,
        ),
        id="image-fetch-full",
    ),
    pytest.param(
        their_errors.VectorStoreSearchError(
            failures=[
                {
                    "vector_store_id": "vs_one",
                    "custom_llm_provider": "pinecone",
                    "error": "not found",
                },
                {"vector_store_id": "vs_two", "custom_llm_provider": None, "error": "timed out"},
            ],
            model=_MODEL,
            llm_provider=_PROVIDER,
        ),
        our_errors.VectorStoreSearchError(
            failures=[
                {
                    "vector_store_id": "vs_one",
                    "custom_llm_provider": "pinecone",
                    "error": "not found",
                },
                {"vector_store_id": "vs_two", "custom_llm_provider": None, "error": "timed out"},
            ],
            model=_MODEL,
            llm_provider=_PROVIDER,
        ),
        id="vector-store-search",
    ),
    pytest.param(
        their_errors.UnprocessableEntityError(
            message=_MESSAGE,
            model=_MODEL,
            llm_provider=_PROVIDER,
            response=_THEIR_RESPONSE,
            litellm_debug_info=_DEBUG,
            max_retries=3,
            num_retries=1,
        ),
        our_errors.UnprocessableEntityError(
            message=_MESSAGE,
            model=_MODEL,
            llm_provider=_PROVIDER,
            response=_OUR_RESPONSE,
            litellm_debug_info=_DEBUG,
            max_retries=3,
            num_retries=1,
        ),
        id="unprocessable-entity",
    ),
    pytest.param(
        their_errors.PermissionDeniedError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            response=_THEIR_RESPONSE,
            litellm_debug_info=_DEBUG,
        ),
        our_errors.PermissionDeniedError(
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            model=_MODEL,
            response=_OUR_RESPONSE,
            litellm_debug_info=_DEBUG,
        ),
        id="permission-denied",
    ),
    pytest.param(
        their_errors.ContextWindowExceededError(
            message=_CONTEXT, model=_MODEL, llm_provider=_PROVIDER
        ),
        our_errors.ContextWindowExceededError(
            message=_CONTEXT, model=_MODEL, llm_provider=_PROVIDER
        ),
        id="context-window-defaults",
    ),
    pytest.param(
        their_errors.ContextWindowExceededError(
            message=_CONTEXT,
            model=_MODEL,
            llm_provider=_PROVIDER,
            response=_THEIR_RESPONSE,
            litellm_debug_info=_DEBUG,
        ),
        our_errors.ContextWindowExceededError(
            message=_CONTEXT,
            model=_MODEL,
            llm_provider=_PROVIDER,
            response=_OUR_RESPONSE,
            litellm_debug_info=_DEBUG,
        ),
        id="context-window-response",
    ),
    pytest.param(
        their_errors.RejectedRequestError(
            message=_REJECTED,
            model=_MODEL,
            llm_provider=_PROVIDER,
            request_data=_REQUEST_DATA,
            litellm_debug_info=_DEBUG,
        ),
        our_errors.RejectedRequestError(
            message=_REJECTED,
            model=_MODEL,
            llm_provider=_PROVIDER,
            request_data=_REQUEST_DATA,
            litellm_debug_info=_DEBUG,
        ),
        id="rejected-request",
    ),
    pytest.param(
        their_errors.ContentPolicyViolationError(
            message=_POLICY,
            model=_MODEL,
            llm_provider=_PROVIDER,
            response=_THEIR_RESPONSE,
            litellm_debug_info=_DEBUG,
            provider_specific_fields=_POLICY_FIELDS,
            body=_BODY,
        ),
        our_errors.ContentPolicyViolationError(
            message=_POLICY,
            model=_MODEL,
            llm_provider=_PROVIDER,
            response=_OUR_RESPONSE,
            litellm_debug_info=_DEBUG,
            provider_specific_fields=_POLICY_FIELDS,
            body=_BODY,
        ),
        id="content-policy",
    ),
    pytest.param(
        their_errors.UnsupportedParamsError(
            message=_PARAMS,
            llm_provider=_PROVIDER,
            model=_MODEL,
            status_code=418,
            response=_THEIR_RESPONSE,
            litellm_debug_info=_DEBUG,
            max_retries=2,
            num_retries=1,
        ),
        our_errors.UnsupportedParamsError(
            message=_PARAMS,
            llm_provider=_PROVIDER,
            model=_MODEL,
            status_code=418,
            response=_OUR_RESPONSE,
            litellm_debug_info=_DEBUG,
            max_retries=2,
            num_retries=1,
        ),
        id="unsupported-params",
    ),
    pytest.param(
        their_errors.UnsupportedParamsError(message=_PARAMS),
        our_errors.UnsupportedParamsError(message=_PARAMS),
        id="unsupported-params-minimal",
    ),
    pytest.param(
        their_errors.LiteLLMUnknownProvider(model=_MODEL),
        our_errors.LiteLLMUnknownProvider(model=_MODEL),
        id="unknown-provider",
    ),
    pytest.param(
        their_errors.LiteLLMUnknownProvider(model=_MODEL, custom_llm_provider="vertex_ai"),
        our_errors.LiteLLMUnknownProvider(model=_MODEL, custom_llm_provider="vertex_ai"),
        id="unknown-provider-named",
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
    assert _StatusView.model_validate(ours) == _StatusView.model_validate(theirs)
    their_names = _mro_names(theirs)
    our_names = _mro_names(ours)
    assert our_names[0] == their_names[0]
    assert [name for name in their_names if name in our_names] == their_names
    assert _signature_views(ours) == _signature_views(theirs)


@pytest.mark.parametrize(("theirs", "ours"), _CASES)
def test_status_error_matches(theirs: BaseException, ours: BaseException) -> None:
    _check(
        theirs,
        ours,
    )


def test_bad_request_non_dict_body() -> None:
    ours = our_errors.BadRequestError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, body=_BYTES
    )
    view = _BodyView.model_validate(ours)
    assert view.code is None
    assert view.param is None
    assert view.type is None


def test_invalid_request_pinned() -> None:
    ours = our_errors.InvalidRequestError(message=_INVALID, model=_MODEL, llm_provider=_PROVIDER)
    view = _StatusView.model_validate(ours)
    params = _signature_views(ours)
    assert str(ours) == _INVALID
    assert repr(ours) == _INVALID_REPR
    assert view.message == _INVALID
    assert view.status_code == _BAD_REQUEST_STATUS
    assert view.model == _MODEL
    assert view.llm_provider == _PROVIDER
    assert view.num_retries is None
    assert view.max_retries is None
    assert view.litellm_debug_info is None
    assert view.body is None
    assert view.code is None
    assert view.param is None
    assert view.type is None
    assert view.args == (_INVALID,)
    assert [param.name for param in params] == _INVALID_PARAMS
    assert [param.default for param in params] == [_EMPTY_PARAM_DEFAULT] * len(params)
    response = view.response
    assert response is not None
    assert response.status_code == _BAD_REQUEST_STATUS
    assert response.request.method == _MOCK_METHOD
    assert response.request.url == _MOCK_URL
    request = view.request
    assert request is not None
    assert request.method == _MOCK_METHOD
    assert request.url == _MOCK_URL
