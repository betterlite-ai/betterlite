import asyncio
import inspect
import json
import logging
from collections.abc import Callable, Mapping
from typing import ClassVar, Final

import httpx
import pytest
from completion_server import (
    BAD_KEY,
    CHAT_PATH,
    COMPLETION,
    CONTENT,
    GOOD_KEY,
    GOOD_MODEL,
    RATE_LIMITED_KEY,
    UNKNOWN_MODEL,
    CompletionServer,
    await_outcome,
    error_view,
    header_of,
    litellm_async_plain,
    litellm_main,
    litellm_module,
    litellm_plain,
    model_response,
    normalized,
    openai_module,
    raised,
)
from pydantic import BaseModel, JsonValue

import betterlite
from betterlite import _native
from betterlite._completion.arguments import timeout_value
from betterlite._completion.completion import settle_outcome
from betterlite._completion.errors import EXCEPTION_TYPES, raise_native_error
from betterlite.exceptions import (
    APIError,
    BadRequestError,
    ContextWindowExceededError,
    InternalServerError,
    RateLimitError,
    Timeout,
)
from betterlite.main import (
    TokenCountResponse,
    UnknownNativeError,
    UnsupportedCallArgument,
    acompletion,
    acount_tokens,
    acreate,
    check_valid_key,
    completion,
    valid_model,
)
from betterlite.types.utils import Message, ModelResponse

MESSAGES: Final[list[dict[str, JsonValue]]] = [{"role": "user", "content": "hi"}]
COUNTED_MODEL: Final = "gpt-3.5-turbo"
TOKEN_MESSAGES: Final[list[dict[str, JsonValue]]] = [{"role": "user", "content": "hello world"}]
NATIVE_MESSAGE: Final = "provider said no"
NATIVE_PROVIDER: Final = "openai"
NATIVE_STATUS: Final = 400
REQUEST_ID: Final = "request-1"
RATE_LIMIT_STATUS: Final = 429
CONNECT_SECONDS: Final = 0.5
READ_SECONDS: Final = 1.5
PROXY_HEADER: Final = "x-proxy-auth"
PROXY_VALUE: Final = "yes"
BROKEN_PROXY_MESSAGE: Final = "Failed to get proxy auth headers: boom"
ALIASED_MODEL: Final = "gpt-4o-mini"
LOGGER_FN_WARNING: Final = "logger_fn is not called: the completion runs in Rust"
MOCK_ID: Final = "mock"
MOCK_CREATED: Final = 5
MOCK_CONTENT: Final = "mocked"
MOCK_TEXT: Final = "hello mock"
PLAIN_ERROR_TEXT: Final = "plain bad"
TEAPOT_STATUS: Final = 418
TEAPOT_TEXT: Final = "teapot"
RATE_LIMIT_TEXT: Final = "slow"
NO_TIMEOUT: Final = 0.0
TIMEOUT_STATUS: Final = 408


class Answer(BaseModel):
    answer: str


class TeapotError(Exception):
    status_code: ClassVar[int] = TEAPOT_STATUS
    text: ClassVar[str] = TEAPOT_TEXT
    llm_provider: ClassVar[str] = "custom"


class RatelimitedError(Exception):
    status_code: ClassVar[int] = RATE_LIMIT_STATUS
    text: ClassVar[str] = RATE_LIMIT_TEXT


class ProxyAuth:
    def get_auth_headers(self) -> Mapping[str, str]:
        return {PROXY_HEADER: PROXY_VALUE}


class BrokenProxyAuth:
    def get_auth_headers(self) -> Mapping[str, str]:
        raise ValueError("boom")


MOCK_VALUES: Final[list[tuple[str | Exception, type[BaseException]]]] = [
    ("litellm.RateLimitError", RateLimitError),
    ("litellm.ContextWindowExceededError", ContextWindowExceededError),
    ("litellm.InternalServerError", InternalServerError),
    ("Exception: content_filter_policy", BadRequestError),
    ("Exception: mock_streaming_error", APIError),
    (ValueError(PLAIN_ERROR_TEXT), InternalServerError),
    (TeapotError("fake"), APIError),
    (RatelimitedError(RATE_LIMIT_TEXT), RateLimitError),
]


def parameter_rows(function: Callable[..., object]) -> list[tuple[str, object, str]]:
    return [
        (parameter.name, parameter.kind, repr(parameter.default))
        for parameter in inspect.signature(function).parameters.values()
    ]


def test_completion_plain_matches_litellm() -> None:
    with CompletionServer() as server:
        ours = model_response(
            COMPLETION(model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY)
        )
        ours_call = server.calls[-1]
        reference = litellm_plain(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY
        )
    assert normalized(ours.model_dump()) == normalized(reference)
    assert ours_call["path"] == CHAT_PATH
    assert ours_call["body"]["model"] == GOOD_MODEL
    assert header_of(ours_call, "Authorization") == f"Bearer {GOOD_KEY}"


def test_completion_signature_matches_litellm() -> None:
    module = litellm_module()
    assert parameter_rows(completion) == parameter_rows(module.completion)
    assert parameter_rows(acompletion) == parameter_rows(module.acompletion)
    assert not inspect.iscoroutinefunction(completion)
    assert not inspect.iscoroutinefunction(acreate)
    assert inspect.iscoroutinefunction(acompletion)


def test_completion_coroutine_paths_match_litellm() -> None:
    with CompletionServer() as server:
        direct = acreate(model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY)
        assert inspect.isawaitable(direct)
        flagged = completion(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY,
            acompletion=True,
        )
        assert inspect.isawaitable(flagged)
        direct_response = model_response(asyncio.run(await_outcome(direct)))
        flagged_response = model_response(asyncio.run(settle_outcome(flagged)))
        awaited_response = model_response(
            asyncio.run(
                await_outcome(
                    acompletion(
                        model=GOOD_MODEL, messages=MESSAGES, api_base=server.url,
                        api_key=GOOD_KEY,
                    )
                )
            )
        )
        reference = asyncio.run(
            litellm_async_plain(
                model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY
            )
        )
    expected = normalized(reference)
    for response in (direct_response, flagged_response, awaited_response):
        assert normalized(response.model_dump()) == expected


def test_completion_python_values(caplog: pytest.LogCaptureFixture) -> None:
    client_error = raised(
        lambda: COMPLETION(model=GOOD_MODEL, messages=MESSAGES, client=object()),
        UnsupportedCallArgument,
    )
    assert isinstance(client_error, UnsupportedCallArgument)
    assert client_error.argument == "client"
    session_error = raised(
        lambda: COMPLETION(model=GOOD_MODEL, messages=MESSAGES, shared_session=object()),
        UnsupportedCallArgument,
    )
    assert isinstance(session_error, UnsupportedCallArgument)
    assert session_error.argument == "shared_session"
    timeout = httpx.Timeout(READ_SECONDS, connect=CONNECT_SECONDS)
    assert timeout_value(timeout) == {
        "connect": CONNECT_SECONDS, "read": READ_SECONDS,
        "write": READ_SECONDS, "pool": READ_SECONDS,
    }
    with CompletionServer() as server:
        with caplog.at_level(logging.WARNING, logger="LiteLLM"):
            logged = model_response(
                COMPLETION(
                    model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY,
                    logger_fn=print,
                )
            )
        assert LOGGER_FN_WARNING in caplog.messages
        timed = model_response(
            COMPLETION(
                model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY,
                timeout=timeout,
            )
        )
        formatted = model_response(
            COMPLETION(
                model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY,
                response_format=Answer,
            )
        )
        formatted_call = server.calls[-1]
        litellm_plain(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY,
            response_format=Answer,
        )
        reference_call = server.calls[-1]
        ours_format = formatted_call["body"]["response_format"]
        assert ours_format == reference_call["body"]["response_format"]
        message = Message(role="user", content="hi")
        message_dump = message.model_dump()
        messaged = model_response(
            COMPLETION(model=GOOD_MODEL, messages=[message], api_base=server.url, api_key=GOOD_KEY)
        )
        sent = server.calls[-1]["body"]["messages"][0]
        assert sent["role"] == message_dump["role"]
        assert sent["content"] == message_dump["content"]
        mock = ModelResponse(
            id=MOCK_ID, created=MOCK_CREATED, model=GOOD_MODEL,
            choices=[{"index": 0, "message": {"role": "assistant", "content": MOCK_CONTENT}}],
        )
        mocked = model_response(
            COMPLETION(model=GOOD_MODEL, messages=MESSAGES, mock_response=mock)
        )
    assert logged.choices[0].message.content == CONTENT
    assert timed.choices[0].message.content == CONTENT
    assert formatted.choices[0].message.content == CONTENT
    assert messaged.choices[0].message.content == CONTENT
    assert mocked.id == MOCK_ID
    assert mocked.created == MOCK_CREATED
    assert mocked.choices[0].message.content == MOCK_CONTENT


def mock_error(value: str | Exception, expected: type[BaseException]) -> BaseException:
    return raised(
        lambda: COMPLETION(model=GOOD_MODEL, messages=MESSAGES, mock_response=value), expected
    )


def litellm_mock_error(value: str | Exception) -> BaseException:
    return raised(
        lambda: litellm_plain(model=GOOD_MODEL, messages=MESSAGES, mock_response=value),
        openai_module().APIError,
    )


def test_completion_mock_errors_match_litellm() -> None:
    for value, expected in MOCK_VALUES:
        ours = mock_error(value, expected)
        reference = litellm_mock_error(value)
        assert error_view(ours) == error_view(reference)


def mock_error_identity(error: APIError) -> BaseException:
    return raised(
        lambda: COMPLETION(model=GOOD_MODEL, messages=MESSAGES, mock_response=error), APIError
    )


def test_completion_mock_raises_supplied_api_errors() -> None:
    ours_error = APIError(
        status_code=TEAPOT_STATUS, message=TEAPOT_TEXT, llm_provider=NATIVE_PROVIDER,
        model=GOOD_MODEL,
    )
    assert mock_error_identity(ours_error) is ours_error
    openai_error = openai_module().APIError("boom", None, body=None)
    reference = raised(
        lambda: COMPLETION(model=GOOD_MODEL, messages=MESSAGES, mock_response=openai_error),
        openai_module().APIError,
    )
    assert reference is openai_error


def test_completion_mock_text_and_timeout_match_litellm() -> None:
    ours = model_response(
        COMPLETION(model=GOOD_MODEL, messages=MESSAGES, mock_response=MOCK_TEXT)
    )
    reference = litellm_plain(
        model=GOOD_MODEL, messages=MESSAGES, mock_response=MOCK_TEXT
    )
    assert normalized(ours.model_dump()) == normalized(reference)
    timeout_error = raised(
        lambda: COMPLETION(
            model=GOOD_MODEL, messages=MESSAGES, mock_timeout=True, timeout=NO_TIMEOUT
        ),
        Timeout,
    )
    reference_error = raised(
        lambda: litellm_plain(
            model=GOOD_MODEL, messages=MESSAGES, mock_timeout=True, timeout=NO_TIMEOUT
        ),
        openai_module().APIError,
    )
    assert error_view(timeout_error) == error_view(reference_error)


def test_completion_mock_timeout_async() -> None:
    pending = COMPLETION(
        model=GOOD_MODEL, messages=MESSAGES, mock_timeout=True, timeout=NO_TIMEOUT,
        acompletion=True,
    )
    assert inspect.isawaitable(pending)
    error = raised(lambda: asyncio.run(settle_outcome(pending)), Timeout)
    assert isinstance(error, Timeout)
    assert error.status_code == TIMEOUT_STATUS


def test_completion_settings_take_effect_at_call_time(
    monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    with CompletionServer() as server:
        monkeypatch.setattr(betterlite, "proxy_auth", ProxyAuth())
        model_response(
            COMPLETION(model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY)
        )
        assert header_of(server.calls[-1], PROXY_HEADER) == PROXY_VALUE
        monkeypatch.setattr(betterlite, "proxy_auth", BrokenProxyAuth())
        with caplog.at_level(logging.WARNING, logger="LiteLLM"):
            model_response(
                COMPLETION(
                    model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY
                )
            )
        assert BROKEN_PROXY_MESSAGE in caplog.messages
        monkeypatch.setattr(betterlite, "model_alias_map", {GOOD_MODEL: ALIASED_MODEL})
        aliased = model_response(
            COMPLETION(model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY)
        )
        aliased_call = server.calls[-1]
    assert aliased_call["body"]["model"] == ALIASED_MODEL
    assert aliased.model == ALIASED_MODEL


def native_payload(kind: str) -> str:
    payload: dict[str, JsonValue] = {
        "type": kind,
        "message": NATIVE_MESSAGE,
        "status_code": NATIVE_STATUS,
        "llm_provider": NATIVE_PROVIDER,
        "model": GOOD_MODEL,
        "body": {"error": {"message": NATIVE_MESSAGE}},
        "headers": {"x-request-id": REQUEST_ID},
        "num_retries": 0,
        "max_retries": 0,
    }
    return json.dumps(payload)


def raised_native(kind: str, expected: type[BaseException]) -> BaseException:
    native = _native.NativeError(native_payload(kind))
    error = raised(lambda: raise_native_error(native), expected)
    assert error.__cause__ is native
    return error


def test_completion_errors_each_native_type_raises_its_class() -> None:
    for kind, expected in EXCEPTION_TYPES.items():
        error = raised_native(kind, expected)
        assert isinstance(error, expected)


def test_completion_errors_carry_native_fields() -> None:
    rate_limited = raised_native("RateLimitError", RateLimitError)
    assert isinstance(rate_limited, RateLimitError)
    assert rate_limited.message == f"litellm.RateLimitError: {NATIVE_MESSAGE}"
    assert rate_limited.status_code == RATE_LIMIT_STATUS
    assert rate_limited.llm_provider == NATIVE_PROVIDER
    assert rate_limited.model == GOOD_MODEL
    bad_request = raised_native("BadRequestError", BadRequestError)
    assert isinstance(bad_request, BadRequestError)
    assert bad_request.message == f"litellm.BadRequestError: {NATIVE_MESSAGE}"
    assert bad_request.status_code == NATIVE_STATUS
    assert bad_request.model == GOOD_MODEL


def test_completion_errors_unknown_type_raises_unknown_native_error() -> None:
    payload_text = native_payload("MysteryError")
    native = _native.NativeError(payload_text)
    error = raised(lambda: raise_native_error(native), UnknownNativeError)
    assert isinstance(error, UnknownNativeError)
    assert error.payload == payload_text
    assert error.__cause__ is native


def test_completion_errors_provider_status_maps_to_its_class() -> None:
    with CompletionServer() as server:
        error = raised(
            lambda: COMPLETION(
                model=GOOD_MODEL, messages=MESSAGES, api_base=server.url,
                api_key=RATE_LIMITED_KEY,
            ),
            RateLimitError,
        )
    assert isinstance(error, RateLimitError)
    assert error.status_code == RATE_LIMIT_STATUS
    assert isinstance(error.__cause__, _native.NativeError)


def test_completion_probe_key_and_model(monkeypatch: pytest.MonkeyPatch) -> None:
    with CompletionServer() as server:
        monkeypatch.setenv("OPENAI_API_BASE", server.url)
        monkeypatch.setenv("OPENAI_API_KEY", GOOD_KEY)
        assert check_valid_key(GOOD_MODEL, GOOD_KEY)
        assert not check_valid_key(GOOD_MODEL, BAD_KEY)
        valid_model(GOOD_MODEL)
        error = raised(lambda: valid_model(UNKNOWN_MODEL), BadRequestError)
    assert isinstance(error, BadRequestError)
    assert error.status_code == NATIVE_STATUS
    assert error.model == UNKNOWN_MODEL


def test_completion_probe_token_count_matches_litellm() -> None:
    with CompletionServer() as server:
        ours: TokenCountResponse = asyncio.run(
            acount_tokens(model=COUNTED_MODEL, messages=TOKEN_MESSAGES)
        )
        reference = asyncio.run(
            litellm_main().acount_tokens(
                model=COUNTED_MODEL, messages=TOKEN_MESSAGES, api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
    assert ours.total_tokens == reference.total_tokens
    assert ours.request_model == reference.request_model
    assert ours.model_used == reference.model_used
    assert ours.tokenizer_type == reference.tokenizer_type
