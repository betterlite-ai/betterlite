import asyncio
import io
import json
import logging
from collections.abc import Mapping
from contextlib import closing
from pathlib import Path
from typing import Final

import httpx
import pytest
from endpoint_server import (
    BAD_KEY,
    GOOD_KEY,
    JSON_TYPE,
    OK,
    RESPONSES,
    SPEECH_BYTES,
    SPEECH_PATH,
    SPEECH_TYPE,
    UNAUTHORIZED,
    EndpointServer,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

import betterlite
from betterlite import exceptions
from betterlite._completion import arguments
from betterlite._completion.errors import UnknownNativeError, UnsupportedCallArgument
from betterlite._endpoints import (
    acall_native,
    call_native,
    model_from_arguments,
    proxy_headers,
    read_settings,
    to_call_payload,
)
from betterlite._native import NativeError

JSON_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
LOGGER_FN_WARNING: Final = "logger_fn is not called: the endpoint call runs in Rust"
PROXY_MESSAGE: Final = "Failed to get proxy auth headers: boom"
PROXY_HEADER: Final = "x-proxy-auth"
PROXY_VALUE: Final = "yes"
AUTHORIZATION: Final[dict[str, str]] = {"Authorization": f"Bearer {GOOD_KEY}"}
REFUSED_NAMES: Final[list[str]] = ["client", "shared_session"]
READ_BYTES: Final = 4
CONNECT_SECONDS: Final = 0.5
READ_SECONDS: Final = 1.5
WRITE_SECONDS: Final = 2.5
POOL_SECONDS: Final = 0.25
PLAIN_TIMEOUT: Final = 30
EMBEDDING_MODEL: Final = "text-embedding-3-small"
MODEL: Final = "gpt-4o"
MODEL_BODY: Final[dict[str, JsonValue]] = {"model": MODEL}
SPEECH_BODY: Final[dict[str, JsonValue]] = {"model": "tts-1"}
AUTH_ERROR_BODY: Final[dict[str, JsonValue]] = {
    "type": "AuthenticationError",
    "message": "bad key",
    "status_code": UNAUTHORIZED,
    "llm_provider": "openai",
    "model": EMBEDDING_MODEL,
}
AUTH_ERROR: Final = json.dumps(AUTH_ERROR_BODY)
UNKNOWN_ERROR_BODY: Final[dict[str, JsonValue]] = {"type": "NoSuchError", "message": "x"}
UNKNOWN_ERROR: Final = json.dumps(UNKNOWN_ERROR_BODY)
LIST_ANSWER: Final[dict[str, JsonValue]] = {"object": "list", "data": []}
TIMEOUT: Final = httpx.Timeout(
    READ_SECONDS,
    connect=CONNECT_SECONDS,
    write=WRITE_SECONDS,
    pool=POOL_SECONDS,
)


class FileMarker(TypedDict):
    name: str
    blob: int
    content_type: str | None


MARKERS: Final[TypeAdapter[dict[str, dict[str, FileMarker]]]] = TypeAdapter(
    dict[str, dict[str, FileMarker]]
)


class ErrorDetail(TypedDict):
    message: str
    type: str
    param: str | None
    code: str


class ErrorBody(TypedDict):
    error: ErrorDetail


ERROR_BODY: Final[TypeAdapter[ErrorBody]] = TypeAdapter(ErrorBody)


class FamilyError(NativeError): ...


class ProxyAuth:
    def __init__(self, headers: Mapping[str, str] | None) -> None:
        self.headers = headers

    def get_auth_headers(self) -> Mapping[str, str]:
        if self.headers is None:
            raise ValueError("boom")
        return dict(self.headers)


class FailingCall:
    def __init__(self, failure: NativeError) -> None:
        self.failure = failure

    def __call__(self, _text: str, _blobs: list[bytes]) -> str:
        raise self.failure


class AnsweringCall:
    def __init__(self, answer: str) -> None:
        self.answer = answer

    def __call__(self, _text: str, _blobs: list[bytes]) -> str:
        return self.answer


FAMILY_ERROR: Final = FamilyError(AUTH_ERROR)
FAILING: Final = FailingCall(FAMILY_ERROR)
FAILING_UNKNOWN: Final = FailingCall(FamilyError(UNKNOWN_ERROR))
FAILING_BARE: Final = FailingCall(NativeError(UNKNOWN_ERROR))
ANSWERING: Final = AnsweringCall(json.dumps(LIST_ANSWER))


async def failing_async_embedding(_text: str, _blobs: list[bytes]) -> str:
    await asyncio.sleep(0)
    raise FAMILY_ERROR


async def answering_async(_text: str, _blobs: list[bytes]) -> str:
    await asyncio.sleep(0)
    return json.dumps(LIST_ANSWER)


def test_encode_plain_values() -> None:
    expected = {"model": "m", "n": 2, "stop": None, "temperature": 0.5}
    result = to_call_payload(expected)
    assert result.text == json.dumps(expected)
    assert result.blobs == []


def test_encode_bool_stays_bool() -> None:
    body = JSON_OBJECT.validate_json(to_call_payload({"stream": True}).text)
    assert body["stream"] is True


def test_encode_bytes_file() -> None:
    result = to_call_payload({"image": b"png-bytes"})
    marker = MARKERS.validate_json(result.text)["image"]["$file"]
    assert result.blobs == [b"png-bytes"]
    assert marker == {"name": "file", "blob": 0, "content_type": None}


def test_encode_bytearray_file() -> None:
    result = to_call_payload({"image": bytearray(b"png-bytes")})
    assert result.blobs == [b"png-bytes"]


def test_encode_path_file(tmp_path: Path) -> None:
    path = tmp_path / "voice.wav"
    path.write_bytes(b"wav-bytes")
    result = to_call_payload({"file": path})
    marker = MARKERS.validate_json(result.text)["file"]["$file"]
    assert result.blobs == [b"wav-bytes"]
    assert marker["name"] == "voice.wav"


def test_encode_reader_rewinds() -> None:
    stream = io.BytesIO(b"buffer-bytes")
    stream.read(READ_BYTES)
    result = to_call_payload({"file": stream})
    marker = MARKERS.validate_json(result.text)["file"]["$file"]
    assert result.blobs == [b"buffer-bytes"]
    assert stream.tell() == READ_BYTES
    assert marker["name"] == "file"


def test_encode_handle_keeps_name(tmp_path: Path) -> None:
    path = tmp_path / "voice.wav"
    path.write_bytes(b"wav-bytes")
    with path.open("rb") as handle:
        result = to_call_payload({"file": handle})
        marker = MARKERS.validate_json(result.text)["file"]["$file"]
    assert result.blobs == [b"wav-bytes"]
    assert marker["name"] == "voice.wav"


def test_encode_tuple_names_file() -> None:
    taken = ("voices/take.wav", b"take-bytes", "audio/wav")
    result = to_call_payload({"file": taken})
    marker = MARKERS.validate_json(result.text)["file"]["$file"]
    assert result.blobs == [b"take-bytes"]
    assert marker == {"name": "voices/take.wav", "blob": 0, "content_type": "audio/wav"}


def test_encode_tuple_default_name() -> None:
    result = to_call_payload({"file": ("", b"take")})
    marker = MARKERS.validate_json(result.text)["file"]["$file"]
    assert marker == {"name": "file", "blob": 0, "content_type": None}


def test_encode_tuple_path_content(tmp_path: Path) -> None:
    path = tmp_path / "take.wav"
    path.write_bytes(b"take-bytes")
    taken = ("take.wav", path)
    result = to_call_payload({"file": taken})
    marker = MARKERS.validate_json(result.text)["file"]["$file"]
    assert result.blobs == [b"take-bytes"]
    assert marker["name"] == "take.wav"


def test_encode_nested_file_order() -> None:
    result = to_call_payload({"images": [b"one", b"two"], "meta": {"mask": b"mask"}})
    assert result.blobs == [b"one", b"two", b"mask"]
    body = JSON_OBJECT.validate_json(result.text)
    assert body["images"] == [
        {"$file": {"name": "file", "blob": 0, "content_type": None}},
        {"$file": {"name": "file", "blob": 1, "content_type": None}},
    ]
    assert body["meta"] == {"mask": {"$file": {"name": "file", "blob": 2, "content_type": None}}}


def test_encode_timeout_largest() -> None:
    body = JSON_OBJECT.validate_json(to_call_payload({"timeout": TIMEOUT}).text)
    assert body["timeout"] == WRITE_SECONDS


def test_encode_plain_timeout() -> None:
    body = JSON_OBJECT.validate_json(to_call_payload({"timeout": PLAIN_TIMEOUT}).text)
    assert body["timeout"] == PLAIN_TIMEOUT


@pytest.mark.parametrize("name", REFUSED_NAMES)
def test_refuse_client_and_session(name: str) -> None:
    with pytest.raises(UnsupportedCallArgument) as caught:
        to_call_payload({name: object()})
    assert caught.value.argument == name


def test_refuse_client_none() -> None:
    body = JSON_OBJECT.validate_json(to_call_payload({"client": None, "model": "m"}).text)
    assert body == {"model": "m"}


def test_refuse_logger_fn_once(caplog: pytest.LogCaptureFixture) -> None:
    with caplog.at_level(logging.WARNING, logger="LiteLLM"):
        result = to_call_payload({"logger_fn": print, "model": "m"})
    assert LOGGER_FN_WARNING in caplog.messages
    assert len(caplog.messages) == 1
    assert JSON_OBJECT.validate_json(result.text) == {"model": "m"}


def test_refuse_logger_fn_none(caplog: pytest.LogCaptureFixture) -> None:
    with caplog.at_level(logging.WARNING, logger="LiteLLM"):
        result = to_call_payload({"logger_fn": None})
    assert caplog.messages == []
    assert JSON_OBJECT.validate_json(result.text) == {}


def test_refuse_unknown_object_key() -> None:
    with pytest.raises(TypeError, match="mystery"):
        to_call_payload({"mystery": object()})


def test_runner_returns_json() -> None:
    answer = call_native(ANSWERING, to_call_payload({"model": EMBEDDING_MODEL}))
    assert answer == LIST_ANSWER


def test_runner_async_returns_json() -> None:
    answer = asyncio.run(acall_native(answering_async, to_call_payload({"model": MODEL})))
    assert answer == LIST_ANSWER


def test_runner_maps_native_error() -> None:
    with pytest.raises(exceptions.AuthenticationError) as caught:
        call_native(FAILING, to_call_payload({"model": EMBEDDING_MODEL}))
    assert caught.type is exceptions.AuthenticationError
    assert "bad key" in caught.value.message
    assert caught.value.status_code == UNAUTHORIZED
    assert caught.value.llm_provider == "openai"
    assert str(caught.value.__cause__) == AUTH_ERROR


def test_runner_async_native_error() -> None:
    call = acall_native(failing_async_embedding, to_call_payload({"model": MODEL}))
    with pytest.raises(exceptions.AuthenticationError):
        asyncio.run(call)


def test_runner_unknown_error_type() -> None:
    with pytest.raises(UnknownNativeError):
        call_native(FAILING_UNKNOWN, to_call_payload({"model": EMBEDDING_MODEL}))


def test_runner_error_class_filter() -> None:
    with pytest.raises(NativeError):
        call_native(FAILING_BARE, to_call_payload({"model": EMBEDDING_MODEL}), FamilyError)


def test_runner_model_positional() -> None:
    model = model_from_arguments(("gpt-4o", 1), {})
    assert model == MODEL


def test_runner_model_from_keyword() -> None:
    model = model_from_arguments((), {"model": MODEL})
    assert model == MODEL


def test_runner_model_missing_key() -> None:
    with pytest.raises(KeyError):
        model_from_arguments((), {})


def test_settings_read_per_call() -> None:
    original = betterlite.drop_params
    try:
        for wanted in (False, True):
            betterlite.drop_params = wanted
            assert read_settings(["drop_params"]) == {"drop_params": wanted}
    finally:
        betterlite.drop_params = original


def test_settings_missing_name() -> None:
    with pytest.raises(AttributeError, match="no_such_setting"):
        read_settings(["api_base", "no_such_setting"])


def test_settings_provider_keys() -> None:
    settings = read_settings(["cohere_key", "openai_key", "azure_key"])
    assert list(settings) == ["cohere_key", "openai_key", "azure_key"]


def test_settings_proxy_headers(caplog: pytest.LogCaptureFixture) -> None:
    assert proxy_headers() == {}
    arguments.proxy_auth = ProxyAuth({PROXY_HEADER: PROXY_VALUE})
    try:
        assert proxy_headers() == {PROXY_HEADER: PROXY_VALUE}
        arguments.proxy_auth = ProxyAuth(None)
        with caplog.at_level(logging.WARNING, logger="LiteLLM"):
            assert proxy_headers() == {}
        assert PROXY_MESSAGE in caplog.messages
        record = caplog.records[0]
        assert record.exc_info is not None
    finally:
        arguments.proxy_auth = None


def test_server_answers_routes() -> None:
    with closing(EndpointServer()) as server:
        for path, expected in RESPONSES.items():
            response = httpx.post(
                f"{server.url}{path}", json=MODEL_BODY, headers=AUTHORIZATION
            )
            assert response.status_code == OK
            assert response.headers["content-type"] == JSON_TYPE
            body = JSON_OBJECT.validate_json(response.content)
            assert body == expected


def test_server_speech_bytes() -> None:
    with closing(EndpointServer()) as server:
        response = httpx.post(
            f"{server.url}{SPEECH_PATH}", json=SPEECH_BODY, headers=AUTHORIZATION
        )
        assert response.status_code == OK
        assert response.headers["content-type"] == SPEECH_TYPE
        assert response.content == SPEECH_BYTES


def test_server_rejects_bad_key() -> None:
    with closing(EndpointServer()) as server:
        response = httpx.post(
            f"{server.url}/v1/embeddings",
            json=MODEL_BODY,
            headers={"Authorization": f"Bearer {BAD_KEY}"},
        )
        assert response.status_code == UNAUTHORIZED
        body = ERROR_BODY.validate_json(response.content)
        assert body["error"]["type"] == "invalid_request_error"
        assert body["error"]["code"] == "invalid_api_key"
        assert BAD_KEY in body["error"]["message"]
        assert len(server.calls) == 1


def test_server_records_and_counts() -> None:
    with closing(EndpointServer()) as server:
        for _ in range(2):
            httpx.post(f"{server.url}/v1/embeddings", json=MODEL_BODY, headers=AUTHORIZATION)
        httpx.post(f"{server.url}/v1/rerank", json=MODEL_BODY, headers=AUTHORIZATION)
        assert server.counts == {"/v1/embeddings": 2, "/v1/rerank": 1}
        paths = [record["path"] for record in server.calls]
        assert paths == ["/v1/embeddings", "/v1/embeddings", "/v1/rerank"]
        assert JSON_OBJECT.validate_json(server.calls[0]["body"]) == MODEL_BODY
        headers = {name.lower(): value for name, value in server.calls[0]["headers"].items()}
        assert headers["authorization"] == f"Bearer {GOOD_KEY}"
