import asyncio
import importlib
import inspect
import io
import json
import threading
import wave
from collections.abc import Coroutine
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import ClassVar, Final, Protocol, TypeAlias

import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict, TypeIs

import betterlite
from betterlite._audio.binary import HttpxBinaryResponseContent
from betterlite._audio.main import transcription_response_from_cached_dict
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints import CallValue
from betterlite.exceptions import APIConnectionError, AuthenticationError, BadRequestError
from betterlite.main import aspeech, atranscription, speech, transcription
from betterlite.types.utils.transcription import (
    TranscriptionResponse,
    TranscriptionUsageTokensObject,
)
from betterlite.types.utils.transcription import (
    TranscriptionUsageDurationObject as DurationUsage,
)

HOST: Final = "127.0.0.1"
OK: Final = 200
UNAUTHORIZED: Final = 401
BAD_REQUEST: Final = 400
GOOD_KEY: Final = "sk-test"
BAD_KEY: Final = "bad-key"
MODEL: Final = "whisper-1"
TTS_MODEL: Final = "tts-1"
PROVIDER: Final = "openai"
TRANSCRIPTIONS_PATH: Final = "/v1/audio/transcriptions"
SPEECH_PATH: Final = "/v1/audio/speech"
API_BASE_PATH: Final = "/v1"
JSON_TYPE: Final = "application/json"
AUDIO_TYPE: Final = "audio/mpeg"
TRANSCRIPT: Final = "hello from the server"
AUTH_TEXT: Final = "Incorrect API key provided: bad-key"
AUDIO_BYTES: Final = b"\xff\xfb\x90\x00speech-bytes"
SAMPLE_RATE: Final = 8000
LARGE_SIZE: Final = 25 * 1024 * 1024
BARE_STRING_TEXT: Final = "does not accept bare str inputs"
VOICE_TEXT: Final = "'voice' is required to be passed as a string for OpenAI TTS"
DURATION_SECONDS: Final = 1.0
CACHED_DURATION: Final = 1.5
SPEED: Final = 1.5
SEGMENT: Final[dict[str, JsonValue]] = {
    "id": 0, "start": 0.0, "end": 1.0, "text": TRANSCRIPT,
}
WORD: Final[dict[str, JsonValue]] = {"word": "hello", "start": 0.0, "end": 1.0}
TOKEN_TOTAL: Final = 7
TOKENS_USAGE: Final[dict[str, JsonValue]] = {
    "type": "tokens",
    "input_tokens": 3,
    "output_tokens": 4,
    "total_tokens": TOKEN_TOTAL,
    "input_token_details": {"audio_tokens": 3, "text_tokens": 0},
}
VERBOSE_BODY: Final[dict[str, JsonValue]] = {
    "text": TRANSCRIPT,
    "task": "transcribe",
    "language": "en",
    "duration": DURATION_SECONDS,
    "segments": [SEGMENT],
    "words": [WORD],
    "usage": {"type": "duration", "seconds": DURATION_SECONDS},
}
PLAIN_BODY: Final[dict[str, JsonValue]] = {"text": TRANSCRIPT, "task": "transcribe"}
AUTH_BODY: Final[dict[str, JsonValue]] = {
    "error": {
        "message": AUTH_TEXT,
        "type": "invalid_request_error",
        "param": None,
        "code": "invalid_api_key",
    }
}

BODY: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
NUMBER: Final[TypeAdapter[float]] = TypeAdapter(float)
HIDDEN: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])

BinarySpeech: TypeAlias = (
    HttpxBinaryResponseContent | Coroutine[None, None, HttpxBinaryResponseContent]
)


class RecordedCall(TypedDict):
    path: str
    headers: dict[str, str]
    body: bytes


class ReferenceCall(Protocol):
    def __call__(self, *args: JsonValue, **kwargs: JsonValue) -> JsonValue: ...


class ReferenceModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


REFERENCE: ReferenceModule = importlib.import_module("litellm")


def parameter_rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        default: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(default.default)))
    return rows


def is_binary_pending(
    value: BinarySpeech,
) -> TypeIs[Coroutine[None, None, HttpxBinaryResponseContent]]:
    match value:
        case HttpxBinaryResponseContent():
            return False
        case Coroutine():
            return True


def wav_bytes() -> bytes:
    buffer = io.BytesIO()
    with wave.open(buffer, "wb") as handle:
        handle.setnchannels(1)
        handle.setsampwidth(1)
        handle.setframerate(SAMPLE_RATE)
        handle.writeframes(b"\x80" * SAMPLE_RATE)
    return buffer.getvalue()


class AudioServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]
    transcription_body: dict[str, JsonValue]
    speech_body: bytes
    error_status: int | None
    error_body: dict[str, JsonValue]

    def __init__(self) -> None:
        super().__init__((HOST, 0), AudioHandler)
        self.calls = []
        self.transcription_body = dict(VERBOSE_BODY)
        self.speech_body = AUDIO_BYTES
        self.error_status = None
        self.error_body = {}
        AudioHandler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}{API_BASE_PATH}"

    def record(self, path: str, headers: dict[str, str], body: bytes) -> None:
        self.calls.append({"path": path, "headers": headers, "body": body})

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class AudioHandler(BaseHTTPRequestHandler):
    owner: ClassVar[AudioServer]

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        raw = b"" if declared is None else self.rfile.read(int(declared))
        self.owner.record(self.path, dict(self.headers.items()), raw)
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply_json(UNAUTHORIZED, dict(AUTH_BODY))
            return
        if self.owner.error_status is not None:
            self.reply_json(self.owner.error_status, self.owner.error_body)
            return
        if self.path.endswith("/audio/speech"):
            self.reply_bytes(self.owner.speech_body, AUDIO_TYPE)
            return
        self.reply_json(OK, dict(self.owner.transcription_body))

    def reply_json(self, status: int, payload: dict[str, JsonValue]) -> None:
        self.reply_bytes(json.dumps(payload).encode(), JSON_TYPE, status)

    def reply_bytes(self, encoded: bytes, kind: str, status: int = OK) -> None:
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("x-request-id", "audio-request")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def header_of(call: RecordedCall, name: str) -> str | None:
    for header, value in call["headers"].items():
        if header.lower() == name.lower():
            return value
    return None


def hidden_of(response: TranscriptionResponse) -> dict[str, JsonValue]:
    private: dict[str, JsonValue] | None = response.__pydantic_private__
    assert private is not None
    return HIDDEN.validate_python(private["_hidden_params"])


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(transcription)) == parameter_rows(
        inspect.signature(REFERENCE.transcription)
    )
    assert parameter_rows(inspect.signature(atranscription)) == parameter_rows(
        inspect.signature(REFERENCE.atranscription)
    )
    assert parameter_rows(inspect.signature(speech)) == parameter_rows(
        inspect.signature(REFERENCE.speech)
    )
    assert parameter_rows(inspect.signature(aspeech)) == parameter_rows(
        inspect.signature(REFERENCE.aspeech)
    )
    assert not inspect.iscoroutinefunction(transcription)
    assert inspect.iscoroutinefunction(atranscription)
    assert not inspect.iscoroutinefunction(speech)
    assert inspect.iscoroutinefunction(aspeech)


def test_upload_nameless_file() -> None:
    with closing(AudioServer()) as server:
        response = transcription(
            model=MODEL, file=wav_bytes(), api_base=server.url, api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    body = call["body"]
    assert call["path"] == TRANSCRIPTIONS_PATH
    assert header_of(call, "authorization") == f"Bearer {GOOD_KEY}"
    assert b'name="file"' in body
    assert b'filename="audio.wav"' in body
    assert b"Content-Type: audio/wav" in body
    assert b'name="model"' in body
    assert b"whisper-1" in body
    assert b'name="response_format"' in body
    assert b"verbose_json" in body
    assert response.text == TRANSCRIPT


def test_upload_named_file() -> None:
    with closing(AudioServer()) as server:
        response = transcription(
            model=MODEL,
            file=("clip.wav", wav_bytes(), "audio/x-custom"),
            language="en",
            prompt="a prompt",
            temperature=0,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    body = call["body"]
    assert b'filename="clip.wav"' in body
    assert b"Content-Type: audio/x-custom" in body
    assert b'name="language"' in body
    assert b'name="prompt"' in body
    assert b'name="temperature"' in body
    assert NUMBER.validate_python(response["duration"]) == DURATION_SECONDS
    assert response["language"] == "en"
    hidden = hidden_of(response)
    assert hidden["model"] == MODEL
    assert hidden["custom_llm_provider"] == PROVIDER
    assert "audio_transcription_duration" not in hidden


def test_upload_large_file_once() -> None:
    large = b"\x00" * LARGE_SIZE
    with closing(AudioServer()) as server:
        transcription(
            model=MODEL, file=large, api_base=server.url, api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    assert call["body"].count(large) == 1
    assert len(call["body"]) < len(large) + 4096


def test_transcription_hidden() -> None:
    with closing(AudioServer()) as server:
        response = transcription(
            model=MODEL, file=wav_bytes(), api_base=server.url, api_key=GOOD_KEY,
        )
    hidden = hidden_of(response)
    assert hidden["model"] == MODEL
    assert hidden["custom_llm_provider"] == PROVIDER
    assert BODY.validate_python(hidden["headers"])["x-request-id"] == "audio-request"
    additional = BODY.validate_python(hidden["additional_headers"])
    assert additional["llm_provider-x-request-id"] == "audio-request"
    assert NUMBER.validate_python(response["_response_ms"]) >= 0
    assert NUMBER.validate_python(hidden["_response_ms"]) >= 0


def test_transcription_usage() -> None:
    with closing(AudioServer()) as server:
        response = transcription(
            model=MODEL, file=wav_bytes(), api_base=server.url, api_key=GOOD_KEY,
        )
    match response.usage:
        case DurationUsage() as usage:
            assert usage.seconds == DURATION_SECONDS
        case object():
            pytest.fail("usage is not a duration object")


def test_transcription_token_usage() -> None:
    with closing(AudioServer()) as server:
        server.transcription_body = {"text": TRANSCRIPT, "usage": dict(TOKENS_USAGE)}
        response = transcription(
            model=MODEL, file=wav_bytes(), api_base=server.url, api_key=GOOD_KEY,
        )
    match response.usage:
        case TranscriptionUsageTokensObject() as usage:
            assert usage.total_tokens == TOKEN_TOTAL
        case object():
            pytest.fail("usage is not a tokens object")


def test_transcription_duration() -> None:
    with closing(AudioServer()) as server:
        server.transcription_body = dict(PLAIN_BODY)
        response = transcription(
            model=MODEL, file=wav_bytes(), api_base=server.url, api_key=GOOD_KEY,
        )
    hidden = hidden_of(response)
    assert NUMBER.validate_python(hidden["audio_transcription_duration"]) == DURATION_SECONDS


def test_transcription_cached_dict() -> None:
    cached: dict[str, JsonValue] = {
        "text": TRANSCRIPT, "duration": CACHED_DURATION, "unknown": 1,
    }
    response = transcription_response_from_cached_dict(cached)
    assert response.text == TRANSCRIPT
    assert NUMBER.validate_python(response["duration"]) == CACHED_DURATION
    assert "unknown" not in response.model_dump()


def test_atranscription_positional() -> None:
    with closing(AudioServer()) as server:
        server.transcription_body = dict(PLAIN_BODY)
        response = asyncio.run(
            atranscription(
                "whisper-1", ("clip.wav", wav_bytes()),
                api_base=server.url, api_key=GOOD_KEY,
            )
        )
        call = server.calls[-1]
    hidden = hidden_of(response)
    assert b'filename="clip.wav"' in call["body"]
    assert NUMBER.validate_python(response["_response_ms"]) >= 0
    assert NUMBER.validate_python(hidden["_response_ms"]) >= 0
    assert "audio_transcription_duration" not in hidden


def test_atranscription_awaitable() -> None:
    with closing(AudioServer()) as server:
        server.transcription_body = dict(PLAIN_BODY)
        pending = atranscription(
            model=MODEL, file=wav_bytes(), api_base=server.url, api_key=GOOD_KEY,
        )
        assert inspect.isawaitable(pending)
        response = asyncio.run(pending)
    assert response.text == TRANSCRIPT
    hidden = hidden_of(response)
    assert NUMBER.validate_python(hidden["_response_ms"]) >= 0
    assert NUMBER.validate_python(hidden["audio_transcription_duration"]) == DURATION_SECONDS


def test_speech_returns_bytes() -> None:
    with closing(AudioServer()) as server:
        response = speech(
            model=TTS_MODEL,
            input="hello",
            voice="alloy",
            response_format="mp3",
            speed=SPEED,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        call = server.calls[-1]
    assert not is_binary_pending(response)
    body = BODY.validate_json(call["body"])
    assert call["path"] == SPEECH_PATH
    assert body["model"] == TTS_MODEL
    assert body["input"] == "hello"
    assert body["voice"] == "alloy"
    assert body["response_format"] == "mp3"
    assert body["speed"] == SPEED
    assert header_of(call, "authorization") == f"Bearer {GOOD_KEY}"
    assert response.content == AUDIO_BYTES
    assert response.status_code == OK
    assert response.headers["x-request-id"] == "audio-request"


def test_speech_binary_methods(tmp_path: Path) -> None:
    with closing(AudioServer()) as server:
        response = speech(
            model=TTS_MODEL, input="hello", voice="alloy",
            api_base=server.url, api_key=GOOD_KEY,
        )
    assert not is_binary_pending(response)
    assert response.read() == AUDIO_BYTES
    assert b"".join(response.iter_bytes(4)) == AUDIO_BYTES
    assert response.text == AUDIO_BYTES.decode("utf-8", "replace")
    target = tmp_path / "reply.mp3"
    response.stream_to_file(target)
    assert target.read_bytes() == AUDIO_BYTES


def test_speech_json_reply() -> None:
    with closing(AudioServer()) as server:
        server.speech_body = b'{"ok": true}'
        response = speech(
            model=TTS_MODEL, input="hello", voice="alloy",
            api_base=server.url, api_key=GOOD_KEY,
        )
    assert not is_binary_pending(response)
    assert BODY.validate_python(response.json())["ok"] is True


def test_speech_aspeech_pending() -> None:
    with closing(AudioServer()) as server:
        pending = speech(
            model=TTS_MODEL, input="hello", voice="alloy", aspeech=True,
            api_base=server.url, api_key=GOOD_KEY,
        )
        assert is_binary_pending(pending)
        response = asyncio.run(pending)
    assert response.content == AUDIO_BYTES


def test_aspeech_reads_model() -> None:
    with closing(AudioServer()) as server:
        response = asyncio.run(
            aspeech("tts-1", "hello", "alloy", api_base=server.url, api_key=GOOD_KEY)
        )
        body = BODY.validate_json(server.calls[-1]["body"])
    assert body["model"] == "tts-1"
    assert response.content == AUDIO_BYTES


def test_aspeech_requires_model() -> None:
    with pytest.raises(KeyError, match="model"):
        asyncio.run(aspeech())


def test_transcription_auth_error() -> None:
    with closing(AudioServer()) as server, pytest.raises(AuthenticationError) as caught:
        transcription(
            model=MODEL, file=wav_bytes(), api_base=server.url, api_key=BAD_KEY,
        )
    error = caught.value
    assert error.status_code == UNAUTHORIZED
    assert error.model == MODEL
    assert error.llm_provider == PROVIDER
    assert AUTH_TEXT in error.message
    assert error.type == "invalid_request_error"
    assert error.code == "invalid_api_key"
    cause = error.__cause__
    assert cause is not None
    assert cause.__class__.__name__ == "NativeAudioError"


def test_speech_auth_error() -> None:
    with closing(AudioServer()) as server, pytest.raises(AuthenticationError) as caught:
        speech(
            model=TTS_MODEL, input="hello", voice="alloy",
            api_base=server.url, api_key=BAD_KEY,
        )
    assert caught.value.status_code == UNAUTHORIZED
    assert AUTH_TEXT in caught.value.message
    assert caught.value.type == "invalid_request_error"
    assert caught.value.code == "invalid_api_key"


def test_speech_voice_error() -> None:
    with closing(AudioServer()) as server, pytest.raises(BadRequestError, match=VOICE_TEXT):
        speech(model=TTS_MODEL, input="hello", api_base=server.url, api_key=GOOD_KEY)
    assert server.calls == []


def test_speech_empty_voice() -> None:
    with closing(AudioServer()) as server:
        speech(
            model=TTS_MODEL, input="hello", voice="",
            api_base=server.url, api_key=GOOD_KEY,
        )
        body = BODY.validate_json(server.calls[-1]["body"])
    assert body["voice"] == ""


def test_speech_bad_provider_error() -> None:
    with pytest.raises(Exception, match="Unable to map the custom llm provider=vertex_ai"):
        speech(model="vertex_ai/tts", input="hello", voice="alloy")


def test_unsupported_transcription() -> None:
    with pytest.raises(APIConnectionError, match="transcription is not supported for vertex_ai"):
        transcription(model="vertex_ai/whisper", file=wav_bytes())


def test_transcription_bare_string() -> None:
    with pytest.raises(ValueError, match=BARE_STRING_TEXT):
        transcription(model=MODEL, file="clip.wav")


def test_client_argument_error() -> None:
    with pytest.raises(UnsupportedCallArgument) as caught:
        speech(model=TTS_MODEL, input="hello", voice="alloy", client="injected")
    assert caught.value.argument == "client"


def test_speech_missing_key_error(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    previous_key, previous_openai = betterlite.api_key, betterlite.openai_key
    betterlite.api_key = None
    betterlite.openai_key = None
    try:
        with pytest.raises(APIConnectionError) as caught:
            speech(model=TTS_MODEL, input="hello", voice="alloy")
    finally:
        betterlite.api_key, betterlite.openai_key = previous_key, previous_openai
    assert "api_key client option must be set" in caught.value.message


@pytest.mark.parametrize(("field", "value", "message"), [
    ("model", None, "model parameter is required"),
    ("model", 42, "model must be text"),
    ("file", None, "file is required"),
    ("file", "clip.wav", BARE_STRING_TEXT),
    ("timeout", "soon", "timeout must be a number"),
])
def test_transcription_local_error(field: str, value: CallValue, message: str) -> None:
    values: dict[str, CallValue] = {"model": MODEL, "file": wav_bytes(), "api_key": GOOD_KEY}
    values[field] = value
    for mode in ("sync", "async"):
        with pytest.raises(ValueError, match=message) as caught:
            invoke_transcription(mode, values)
        cause = caught.value.__cause__
        assert cause is not None
        assert cause.__class__.__name__ == "NativeAudioError"


def invoke_transcription(mode: str, values: dict[str, CallValue]) -> None:
    if mode == "sync":
        transcription(**values)
    else:
        asyncio.run(atranscription(**values))


def invoke_audio(operation: str, server: AudioServer) -> None:
    match operation:
        case "transcription":
            transcription(MODEL, wav_bytes(), api_base=server.url, api_key=GOOD_KEY)
        case "atranscription":
            asyncio.run(atranscription(
                MODEL, wav_bytes(), api_base=server.url, api_key=GOOD_KEY,
            ))
        case "speech":
            speech(TTS_MODEL, "hello", "alloy", api_base=server.url, api_key=GOOD_KEY)
        case "aspeech":
            asyncio.run(aspeech(
                TTS_MODEL, "hello", "alloy", api_base=server.url, api_key=GOOD_KEY,
            ))
        case object():
            pytest.fail("unknown audio operation")


@pytest.mark.parametrize("operation", ["transcription", "atranscription", "speech", "aspeech"])
def test_audio_provider_body_error(operation: str) -> None:
    with closing(AudioServer()) as server:
        server.error_status = BAD_REQUEST
        server.error_body = {"error": {
            "message": "Invalid file", "type": "invalid_request_error",
            "param": "file", "code": "invalid_file",
        }}
        with pytest.raises(BadRequestError) as caught:
            invoke_audio(operation, server)
    assert caught.value.type == "invalid_request_error"
    assert caught.value.param == "file"
    assert caught.value.code == "invalid_file"
    assert BODY.validate_python(caught.value.body)["code"] == "invalid_file"


def test_speech_sdk_identity_headers(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("OPENAI_ORG_ID", raising=False)
    monkeypatch.delenv("OPENAI_PROJECT_ID", raising=False)
    monkeypatch.setenv("OPENAI_ORGANIZATION", "org-env")
    monkeypatch.setenv("OPENAI_PROJECT", "project-env")
    monkeypatch.setattr(betterlite, "organization", "org-setting")
    monkeypatch.setattr(betterlite, "project", "project-setting")
    with closing(AudioServer()) as server:
        speech(
            TTS_MODEL, "hello", "alloy", organization="org-a", project="project-a",
            api_base=server.url, api_key=GOOD_KEY,
        )
        assert header_of(server.calls[-1], "openai-organization") is None
        assert header_of(server.calls[-1], "openai-project") is None
        monkeypatch.setenv("OPENAI_ORG_ID", "org-sdk")
        monkeypatch.setenv("OPENAI_PROJECT_ID", "project-sdk")
        asyncio.run(aspeech(
            TTS_MODEL, "hello", "alloy", organization="org-a", project="project-a",
            api_base=server.url, api_key=GOOD_KEY,
        ))
        assert header_of(server.calls[-1], "openai-organization") == "org-sdk"
        assert header_of(server.calls[-1], "openai-project") == "project-sdk"
