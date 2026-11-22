import asyncio
import inspect
import json
import logging
from collections.abc import Awaitable, Mapping
from pathlib import Path
from typing import Final, Protocol, TypedDict

import httpx
import pytest
from completion_server import (
    BAD_KEY,
    CHAT_PATH,
    CONTENT,
    GOOD_KEY,
    GOOD_MODEL,
    LITELLM,
    LITELLM_COUNTS,
    OPENAI,
    RATE_LIMITED_KEY,
    UNKNOWN_MODEL,
    CompletionServer,
    CountView,
    FakeAdapter,
    adapter_seam,
    config_seam,
    error_view,
    header_of,
    litellm_async_plain,
    litellm_plain,
    model_response,
    normalized,
)
from pydantic import BaseModel, JsonValue, TypeAdapter

import betterlite
from betterlite import _native
from betterlite._completion import arguments
from betterlite._completion.arguments import timeout_value
from betterlite._completion.completion import settle_outcome
from betterlite._completion.errors import EXCEPTION_TYPES, raise_native_error
from betterlite._completion.stream import CompletionOutcome
from betterlite.exceptions import (
    APIError,
    BadRequestError,
    ContextWindowExceededError,
    InternalServerError,
    RateLimitError,
    Timeout,
)
from betterlite.main import (
    UnknownNativeError,
    UnsupportedCallArgument,
    acompletion,
    acount_tokens,
    acreate,
    adapter_completion,
    check_valid_key,
    completion,
    completion_with_retries,
    config_completion,
    valid_model,
)
from betterlite.types.utils import Message, ModelResponse

MESSAGES: Final[list[dict[str, JsonValue] | Message]] = [{"role": "user", "content": "hi"}]
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
OPENAI_API_ERROR: Final[type[BaseException]] = OPENAI.APIError(
    "probe", None, body=None
).__class__
ADAPTER_ID: Final = "values-adapter"
CONFIG_NAME: Final = "config.json"


class Answer(BaseModel):
    answer: str


class MockMessage(TypedDict):
    role: str
    content: str


class MockChoice(TypedDict):
    index: int
    finish_reason: str
    message: MockMessage


class MockUsage(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class MockResponseView(TypedDict):
    object: str
    model: str
    choices: list[MockChoice]
    usage: MockUsage


MOCK_RESPONSE_VIEW: Final[TypeAdapter[MockResponseView]] = TypeAdapter(MockResponseView)


class TeapotError(Exception):
    status_code: int = TEAPOT_STATUS
    text: str = TEAPOT_TEXT
    llm_provider: str = "custom"


class RatelimitedError(Exception):
    status_code: int = RATE_LIMIT_STATUS
    text: str = RATE_LIMIT_TEXT


class ProxyAuth:
    # ast-grep-ignore: no-one-line-functions
    def get_auth_headers(self) -> Mapping[str, str]:
        return {PROXY_HEADER: PROXY_VALUE}


class BrokenProxyAuth:
    # ast-grep-ignore: no-one-line-functions
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


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


def parameter_rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        default: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(default.default)))
    return rows


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


def native_cause(error: BaseException) -> _native.NativeError:
    cause = error.__cause__
    match cause:
        case _native.NativeError():
            return cause
        case object():
            raise AssertionError("the exception does not carry the native error as its cause")


async def awaited_responses(
    direct: Awaitable[CompletionOutcome],
    flagged: CompletionOutcome | Awaitable[CompletionOutcome],
    server_url: str,
) -> list[dict[str, JsonValue]]:
    direct_response = await direct
    flagged_response = await settle_outcome(flagged)
    awaited_response = await acompletion(
        model=GOOD_MODEL, messages=MESSAGES, api_base=server_url, api_key=GOOD_KEY,
    )
    return [
        model_response(direct_response).model_dump(),
        model_response(flagged_response).model_dump(),
        model_response(awaited_response).model_dump(),
    ]


def test_plain_matches_litellm() -> None:
    with CompletionServer() as server:
        response = model_response(
            completion(
                model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY,
            )
        )
        ours_call = server.calls[-1]
        reference = litellm_plain(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY,
        )
    assert normalized(response.model_dump()) == normalized(reference)
    assert ours_call["path"] == CHAT_PATH
    assert ours_call["body"].get("model") == GOOD_MODEL
    assert header_of(ours_call, "Authorization") == f"Bearer {GOOD_KEY}"


def test_plain_missing_model() -> None:
    with pytest.raises(ValueError, match="model param not passed in") as ours_caught:
        completion(model=None, messages=MESSAGES)
    with pytest.raises(ValueError, match="model param not passed in") as reference_caught:
        LITELLM.completion(model=None, messages=MESSAGES)
    assert str(ours_caught.value) == str(reference_caught.value)


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(completion)) == parameter_rows(
        inspect.signature(LITELLM.completion)
    )
    assert parameter_rows(inspect.signature(acompletion)) == parameter_rows(
        inspect.signature(LITELLM.acompletion)
    )
    assert not inspect.iscoroutinefunction(completion)
    assert not inspect.iscoroutinefunction(acreate)
    assert inspect.iscoroutinefunction(acompletion)


def test_coroutine_paths() -> None:
    with CompletionServer() as server:
        direct = acreate(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY,
        )
        assert inspect.isawaitable(direct)
        flagged = completion(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server.url,
            api_key=GOOD_KEY, acompletion=True,
        )
        assert inspect.isawaitable(flagged)
        responses = asyncio.run(awaited_responses(direct, flagged, server.url))
        reference = asyncio.run(
            litellm_async_plain(
                model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY,
            )
        )
    expected = normalized(reference)
    for response in responses:
        assert normalized(response) == expected


def assert_response_format_matches(
    server: CompletionServer, bodies: list[dict[str, JsonValue]]
) -> None:
    litellm_plain(
        model=GOOD_MODEL, messages=MESSAGES, api_base=server.url,
        api_key=GOOD_KEY, response_format=Answer,
    )
    expected = server.calls[-1]["body"].get("response_format")
    assert expected is not None
    for body in bodies:
        assert body.get("response_format") == expected


def test_python_values(caplog: pytest.LogCaptureFixture) -> None:
    with pytest.raises(UnsupportedCallArgument) as client_caught:
        completion(model=GOOD_MODEL, messages=MESSAGES, client=object())
    assert client_caught.value.argument == "client"
    with pytest.raises(UnsupportedCallArgument) as session_caught:
        completion(model=GOOD_MODEL, messages=MESSAGES, shared_session=object())
    assert session_caught.value.argument == "shared_session"
    timeout = httpx.Timeout(READ_SECONDS, connect=CONNECT_SECONDS)
    assert timeout_value(timeout) == {
        "connect": CONNECT_SECONDS, "read": READ_SECONDS,
        "write": READ_SECONDS, "pool": READ_SECONDS,
    }
    with CompletionServer() as server:
        with caplog.at_level(logging.WARNING, logger="LiteLLM"):
            logged = model_response(
                completion(
                    model=GOOD_MODEL, messages=MESSAGES, api_base=server.url,
                    api_key=GOOD_KEY, logger_fn=print,
                )
            )
        assert LOGGER_FN_WARNING in caplog.messages
        timed = model_response(
            completion(
                model=GOOD_MODEL, messages=MESSAGES, api_base=server.url,
                api_key=GOOD_KEY, timeout=timeout,
            )
        )
        formatted = model_response(
            completion(
                model=GOOD_MODEL, messages=MESSAGES, api_base=server.url,
                api_key=GOOD_KEY, response_format=Answer,
            )
        )
        assert_response_format_matches(server, [server.calls[-1]["body"]])
        message = Message(role="user", content="hi")
        message_dump = message.model_dump()
        message_values: list[dict[str, JsonValue] | Message] = [message]
        messaged = model_response(
            completion(
                model=GOOD_MODEL, messages=message_values, api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        sent = server.calls[-1]["body"].get("messages")
        assert sent is not None
        assert sent[0]["role"] == message_dump["role"]
        assert sent[0]["content"] == message_dump["content"]
        mock = ModelResponse(
            id=MOCK_ID, created=MOCK_CREATED, model=GOOD_MODEL,
            choices=[{"index": 0, "message": {"role": "assistant", "content": MOCK_CONTENT}}],
        )
        mocked = model_response(
            completion(model=GOOD_MODEL, messages=MESSAGES, mock_response=mock)
        )
        ignored_logging = model_response(
            completion(
                model=GOOD_MODEL, messages=MESSAGES, mock_response=MOCK_TEXT,
                litellm_logging_obj=object(),
            )
        )
    assert logged.choices[0].message.content == CONTENT
    assert timed.choices[0].message.content == CONTENT
    assert formatted.choices[0].message.content == CONTENT
    assert messaged.choices[0].message.content == CONTENT
    assert mocked.id == MOCK_ID
    assert mocked.created == MOCK_CREATED
    assert mocked.choices[0].message.content == MOCK_CONTENT
    assert ignored_logging.choices[0].message.content == MOCK_TEXT


def test_python_values_calls() -> None:
    # the class conversion sits in completion_from_arguments, so acreate and the retry loop share it
    with CompletionServer() as server:
        awaited = acreate(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server.url,
            api_key=GOOD_KEY, response_format=Answer,
        )
        asyncio.run(settle_outcome(awaited))
        awaited_body = server.calls[-1]["body"]
        completion_with_retries(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server.url,
            api_key=GOOD_KEY, response_format=Answer,
        )
        retried_body = server.calls[-1]["body"]
        assert_response_format_matches(server, [awaited_body, retried_body])


def test_python_values_entries(tmp_path: Path) -> None:
    # _json_values converts the class before config_completion and adapter_completion look at values
    adapter = FakeAdapter()
    with CompletionServer() as server:
        config_file = tmp_path / CONFIG_NAME
        config: dict[str, JsonValue] = {
            "model": GOOD_MODEL, "api_base": server.url, "api_key": GOOD_KEY,
        }
        config_file.write_text(json.dumps(config))
        with config_seam(config_file):
            configured = model_response(
                config_completion(messages=MESSAGES, response_format=Answer)
            )
        configured_body = server.calls[-1]["body"]
        with adapter_seam(adapter, ADAPTER_ID):
            adapted = model_response(
                adapter_completion(
                    adapter_id=ADAPTER_ID, model=GOOD_MODEL, messages=MESSAGES,
                    api_base=server.url, api_key=GOOD_KEY, response_format=Answer,
                )
            )
        adapter_body = server.calls[-1]["body"]
        assert_response_format_matches(server, [configured_body, adapter_body])
    assert configured.choices[0].message.content == CONTENT
    assert adapted.choices[0].message.content == CONTENT


def test_mock_errors_match_litellm() -> None:
    for value, expected in MOCK_VALUES:
        with pytest.raises(expected) as ours_caught:
            model_response(
                completion(model=GOOD_MODEL, messages=MESSAGES, mock_response=value)
            )
        with pytest.raises(OPENAI_API_ERROR) as reference_caught:
            litellm_plain(model=GOOD_MODEL, messages=MESSAGES, mock_response=value)
        assert error_view(ours_caught.value) == error_view(reference_caught.value)


def test_mock_raises_supplied() -> None:
    ours_error = APIError(
        status_code=TEAPOT_STATUS, message=TEAPOT_TEXT, llm_provider=NATIVE_PROVIDER,
        model=GOOD_MODEL,
    )
    with pytest.raises(APIError) as ours_caught:
        completion(model=GOOD_MODEL, messages=MESSAGES, mock_response=ours_error)
    assert ours_caught.value is ours_error
    openai_error = OPENAI.APIError("boom", None, body=None)
    with pytest.raises(openai_error.__class__) as supplied_caught:
        completion(model=GOOD_MODEL, messages=MESSAGES, mock_response=openai_error)
    assert supplied_caught.value is openai_error


def test_mock_text_matches_litellm() -> None:
    # LiteLLM's mock answer has no provider_specific_fields; the native answer always writes it
    ours = MOCK_RESPONSE_VIEW.validate_python(
        model_response(
            completion(model=GOOD_MODEL, messages=MESSAGES, mock_response=MOCK_TEXT)
        ).model_dump()
    )
    reference = MOCK_RESPONSE_VIEW.validate_python(
        litellm_plain(model=GOOD_MODEL, messages=MESSAGES, mock_response=MOCK_TEXT)
    )
    assert ours["object"] == reference["object"]
    assert ours["model"] == reference["model"]
    assert ours["usage"] == reference["usage"]
    ours_rows = [
        (choice["index"], choice["finish_reason"], choice["message"]["role"],
         choice["message"]["content"])
        for choice in ours["choices"]
    ]
    reference_rows = [
        (choice["index"], choice["finish_reason"], choice["message"]["role"],
         choice["message"]["content"])
        for choice in reference["choices"]
    ]
    assert ours_rows == reference_rows


def test_mock_timeout_matches() -> None:
    with pytest.raises(Timeout) as ours_caught:
        completion(
            model=GOOD_MODEL, messages=MESSAGES, mock_timeout=True, timeout=NO_TIMEOUT
        )
    with pytest.raises(OPENAI_API_ERROR) as reference_caught:
        litellm_plain(
            model=GOOD_MODEL, messages=MESSAGES, mock_timeout=True, timeout=NO_TIMEOUT
        )
    assert error_view(ours_caught.value) == error_view(reference_caught.value)


def test_mock_timeout_async() -> None:
    pending = completion(
        model=GOOD_MODEL, messages=MESSAGES, mock_timeout=True, timeout=NO_TIMEOUT,
        acompletion=True,
    )
    assert inspect.isawaitable(pending)
    with pytest.raises(Timeout) as caught:
        asyncio.run(settle_outcome(pending))
    assert caught.value.status_code == TIMEOUT_STATUS


def plain_response(server: CompletionServer) -> ModelResponse:
    return model_response(
        completion(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server.url,
            api_key=GOOD_KEY,
        )
    )


def test_settings_take_effect(caplog: pytest.LogCaptureFixture) -> None:
    with CompletionServer() as server:
        arguments.proxy_auth = ProxyAuth()
        try:
            proxied = plain_response(server)
            assert header_of(server.calls[-1], PROXY_HEADER) == PROXY_VALUE
            arguments.proxy_auth = BrokenProxyAuth()
            with caplog.at_level(logging.WARNING, logger="LiteLLM"):
                broken = plain_response(server)
            assert BROKEN_PROXY_MESSAGE in caplog.messages
        finally:
            arguments.proxy_auth = None
        betterlite.model_alias_map = {GOOD_MODEL: ALIASED_MODEL}
        try:
            aliased = plain_response(server)
            aliased_call = server.calls[-1]
            assert aliased_call["body"].get("model") == ALIASED_MODEL
            assert aliased.model == ALIASED_MODEL
        finally:
            betterlite.model_alias_map = {}
    assert proxied.choices[0].message.content == CONTENT
    assert broken.choices[0].message.content == CONTENT


def test_errors_each_native_type() -> None:
    for kind, expected in EXCEPTION_TYPES.items():
        native = _native.NativeError(native_payload(kind))
        with pytest.raises(expected) as caught:
            raise_native_error(native)
        assert caught.type is expected
        assert caught.value.__cause__ is native


def test_errors_native_fields() -> None:
    with pytest.raises(RateLimitError) as rate_limited_caught:
        raise_native_error(_native.NativeError(native_payload("RateLimitError")))
    assert rate_limited_caught.value.message == f"litellm.RateLimitError: {NATIVE_MESSAGE}"
    assert rate_limited_caught.value.status_code == RATE_LIMIT_STATUS
    assert rate_limited_caught.value.llm_provider == NATIVE_PROVIDER
    assert rate_limited_caught.value.model == GOOD_MODEL
    with pytest.raises(BadRequestError) as bad_request_caught:
        raise_native_error(_native.NativeError(native_payload("BadRequestError")))
    assert bad_request_caught.value.message == f"litellm.BadRequestError: {NATIVE_MESSAGE}"
    assert bad_request_caught.value.status_code == NATIVE_STATUS
    assert bad_request_caught.value.model == GOOD_MODEL


def test_errors_unknown_type() -> None:
    payload_text = native_payload("MysteryError")
    native = _native.NativeError(payload_text)
    with pytest.raises(UnknownNativeError) as caught:
        raise_native_error(native)
    assert caught.value.payload == payload_text
    assert caught.value.__cause__ is native


def test_errors_provider_status() -> None:
    with CompletionServer() as server, pytest.raises(RateLimitError) as caught:
        completion(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server.url,
            api_key=RATE_LIMITED_KEY,
        )
    error = caught.value
    assert error.status_code == RATE_LIMIT_STATUS
    assert "RateLimitError" in str(native_cause(error))


def test_probe_key_and_model(monkeypatch: pytest.MonkeyPatch) -> None:
    with CompletionServer() as server:
        monkeypatch.setenv("OPENAI_API_BASE", server.url)
        monkeypatch.setenv("OPENAI_API_KEY", GOOD_KEY)
        assert check_valid_key(GOOD_MODEL, GOOD_KEY)
        assert not check_valid_key(GOOD_MODEL, BAD_KEY)
        valid_model(GOOD_MODEL)
        with pytest.raises(BadRequestError) as caught:
            valid_model(UNKNOWN_MODEL)
    assert caught.value.status_code == NATIVE_STATUS
    assert caught.value.model == UNKNOWN_MODEL


async def counted_reference(server_url: str) -> CountView:
    return await LITELLM_COUNTS.acount_tokens(
        model=COUNTED_MODEL, messages=TOKEN_MESSAGES, api_base=server_url, api_key=GOOD_KEY,
    )


def test_probe_token_count() -> None:
    with CompletionServer() as server:
        ours = asyncio.run(acount_tokens(model=COUNTED_MODEL, messages=TOKEN_MESSAGES))
        reference = asyncio.run(counted_reference(server.url))
    assert ours.total_tokens == reference.total_tokens
    assert ours.request_model == reference.request_model
    assert ours.model_used == reference.model_used
    assert ours.tokenizer_type == reference.tokenizer_type
