import asyncio
import importlib
import inspect
import json
import threading
from collections.abc import Coroutine
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Protocol, runtime_checkable

import pytest
from batch_server import parameter_rows
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints.payload import CallValue
from betterlite._native import NativeRealtimeError
from betterlite.realtime_api import (
    BaseLLMException,
    acreate_realtime_client_secret,
    acreate_realtime_transcription_session,
    arealtime_calls,
)

HOST: Final = "127.0.0.1"
OK: Final = 200
UNAUTHORIZED: Final = 401
NOT_FOUND: Final = 404
GOOD_KEY: Final = "sk-test"
BAD_KEY: Final = "bad-key"
EPHEMERAL_KEY: Final = "ek-test"
MODEL: Final = "openai/gpt-4o-realtime-preview"
RESOLVED_MODEL: Final = "gpt-4o-realtime-preview"
TRANSCRIPTION_MODEL: Final = "openai/gpt-realtime-whisper"
NESTED_MODEL: Final = "gpt-realtime-whisper"
INSTRUCTIONS: Final = "be brief"
CLIENT_SECRET_PATH: Final = "/v1/realtime/client_secrets"
TRANSCRIPTION_PATH: Final = "/v1/realtime/transcription_sessions"
CALLS_PATH: Final = "/v1/realtime/calls"
EXPIRES_AT: Final = 1710000000
SECONDS: Final = 60
SESSION_ID: Final = "sess-1"
CLIENT_SECRET_VALUE: Final = "ek-client-secret"
TRANSCRIPTION_VALUE: Final = "ek-transcription-secret"
AUTH_TEXT: Final = '{"error":{"message":"Incorrect API key provided: bad-key"}}'
SDP_OFFER: Final = b"v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\n"
SDP_ANSWER: Final = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n"
CLIENT_SECRET_BODY: Final[dict[str, JsonValue]] = {
    "value": CLIENT_SECRET_VALUE,
    "expires_at": EXPIRES_AT,
    "session": {"id": SESSION_ID, "type": "realtime", "model": RESOLVED_MODEL},
}
TRANSCRIPTION_BODY: Final[dict[str, JsonValue]] = {
    "client_secret": {"value": TRANSCRIPTION_VALUE, "expires_at": EXPIRES_AT},
}
CLIENT_SECRET_REQUEST: Final[dict[str, JsonValue]] = {
    "expires_after": {"anchor": "created_at", "seconds": SECONDS},
    "session": {"type": "realtime", "instructions": INSTRUCTIONS},
}
TRANSCRIPTION_REQUEST: Final[dict[str, JsonValue]] = {
    "input_audio_transcription": {"model": NESTED_MODEL},
}
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
DUMP: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class RecordedCall(TypedDict):
    path: str
    headers: dict[str, str]
    body: bytes


class NativeFailure(TypedDict):
    type: str
    message: str
    status_code: int


FAILURE: Final[TypeAdapter[NativeFailure]] = TypeAdapter(NativeFailure)


class ReferenceAnswer(Protocol):
    def json(self) -> JsonValue: ...

    @property
    def text(self) -> str: ...


class ReferenceCall(Protocol):
    def __call__(self, **kwargs: CallValue) -> Coroutine[None, None, ReferenceAnswer]: ...


class ReferenceModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


class ErrorModule(Protocol):
    def __getattr__(self, name: str) -> type[BaseException]: ...


@runtime_checkable
class ErrorFields(Protocol):
    status_code: int
    message: str


OURS: ReferenceModule = importlib.import_module("betterlite.realtime_api")
REFERENCE_SECRET: ReferenceModule = importlib.import_module("litellm")
REFERENCE_TRANSCRIPTION: ReferenceModule = importlib.import_module("litellm")
REFERENCE_CALLS: ReferenceModule = importlib.import_module("litellm")
REFERENCE_ERRORS: ErrorModule = importlib.import_module(
    "litellm.llms.base_llm.chat.transformation"
)


class RealtimeServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]
    thread: threading.Thread

    def __init__(self) -> None:
        super().__init__((HOST, 0), RealtimeHandler)
        self.calls = []
        RealtimeHandler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}"

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class RealtimeHandler(BaseHTTPRequestHandler):
    owner: ClassVar[RealtimeServer]

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        body = b"" if declared is None else self.rfile.read(int(declared))
        self.owner.calls.append(
            {"path": self.path, "headers": dict(self.headers.items()), "body": body}
        )
        expected = EPHEMERAL_KEY if self.path == CALLS_PATH else GOOD_KEY
        if self.headers.get("Authorization") != f"Bearer {expected}":
            self.send_payload(UNAUTHORIZED, AUTH_TEXT, "application/json")
            return
        if self.path == CLIENT_SECRET_PATH:
            self.send_payload(OK, json.dumps(CLIENT_SECRET_BODY), "application/json")
        elif self.path == TRANSCRIPTION_PATH:
            self.send_payload(OK, json.dumps(TRANSCRIPTION_BODY), "application/json")
        elif self.path == CALLS_PATH:
            self.send_payload(OK, SDP_ANSWER, "application/sdp")
        else:
            self.send_payload(NOT_FOUND, "", "text/plain")

    def send_payload(self, status: int, text: str, kind: str) -> None:
        encoded = text.encode()
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def posted_fields(call: RecordedCall) -> dict[str, bytes]:
    kind = ""
    for name, value in call["headers"].items():
        if name.lower() == "content-type":
            kind = value
    boundary = kind.partition("boundary=")[2].split(";")[0].strip().strip('"').encode("ascii")
    fields: dict[str, bytes] = {}
    for chunk in call["body"].split(b"--" + boundary):
        head, separator, content = chunk.partition(b"\r\n\r\n")
        if not separator:
            continue
        for line in head.decode("latin-1").split("\r\n"):
            if 'name="' not in line.lower():
                continue
            name = line.split('name="', 1)[1].split('"', 1)[0]
            fields[name] = content.removesuffix(b"\r\n")
    return fields


def error_fields(error: BaseException) -> ErrorFields:
    match error:
        case ErrorFields():
            return error
        case object():
            raise AssertionError("the realtime error carries no status")


def native_cause(error: BaseException) -> NativeRealtimeError:
    cause = error.__cause__
    match cause:
        case NativeRealtimeError():
            return cause
        case object():
            raise AssertionError("the realtime error carries no native cause")


def test_signature_matches_litellm() -> None:
    assert parameter_rows(inspect.signature(acreate_realtime_client_secret)) == (
        parameter_rows(inspect.signature(REFERENCE_SECRET.acreate_realtime_client_secret))
    )
    assert parameter_rows(
        inspect.signature(OURS.acreate_realtime_transcription_session)
    ) == parameter_rows(
        inspect.signature(REFERENCE_TRANSCRIPTION.acreate_realtime_transcription_session)
    )
    assert parameter_rows(inspect.signature(arealtime_calls)) == parameter_rows(
        inspect.signature(REFERENCE_CALLS.arealtime_calls)
    )
    assert inspect.iscoroutinefunction(acreate_realtime_client_secret)
    assert inspect.iscoroutinefunction(OURS.acreate_realtime_transcription_session)
    assert inspect.iscoroutinefunction(arealtime_calls)


def test_client_secret_calls() -> None:
    with closing(RealtimeServer()) as server:
        ours = asyncio.run(
            acreate_realtime_client_secret(
                model=MODEL,
                session={"type": "realtime", "instructions": INSTRUCTIONS},
                expires_after={"seconds": SECONDS},
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        ours_call = server.calls[-1]
        theirs = asyncio.run(
            REFERENCE_SECRET.acreate_realtime_client_secret(
                model=MODEL,
                session={"type": "realtime", "instructions": INSTRUCTIONS},
                expires_after={"seconds": SECONDS},
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        theirs_call = server.calls[-1]
    assert ours_call["path"] == theirs_call["path"] == CLIENT_SECRET_PATH
    assert JSON.validate_json(ours_call["body"]) == JSON.validate_json(theirs_call["body"])
    assert JSON.validate_json(ours_call["body"]) == CLIENT_SECRET_REQUEST
    assert DUMP.validate_json(ours.model_dump_json()) == DUMP.validate_python(theirs.json())
    assert DUMP.validate_json(ours.model_dump_json()) == CLIENT_SECRET_BODY


def test_transcription_calls() -> None:
    session: dict[str, JsonValue] = {"input_audio_transcription": {"model": NESTED_MODEL}}
    with closing(RealtimeServer()) as server:
        ours = asyncio.run(
            acreate_realtime_transcription_session(
                model=TRANSCRIPTION_MODEL,
                transcription_session=session,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        ours_call = server.calls[-1]
        theirs = asyncio.run(
            REFERENCE_TRANSCRIPTION.acreate_realtime_transcription_session(
                model=TRANSCRIPTION_MODEL,
                transcription_session=session,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        theirs_call = server.calls[-1]
    assert ours_call["path"] == theirs_call["path"] == TRANSCRIPTION_PATH
    assert JSON.validate_json(ours_call["body"]) == JSON.validate_json(theirs_call["body"])
    assert JSON.validate_json(ours_call["body"]) == TRANSCRIPTION_REQUEST
    assert DUMP.validate_json(ours.model_dump_json()) == DUMP.validate_python(theirs.json())
    assert DUMP.validate_json(ours.model_dump_json()) == TRANSCRIPTION_BODY


def test_realtime_calls() -> None:
    with closing(RealtimeServer()) as server:
        ours = asyncio.run(
            arealtime_calls(
                openai_ephemeral_key=EPHEMERAL_KEY,
                sdp_body=SDP_OFFER,
                model=MODEL,
                session={"instructions": INSTRUCTIONS},
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        ours_call = server.calls[-1]
        theirs = asyncio.run(
            REFERENCE_CALLS.arealtime_calls(
                openai_ephemeral_key=EPHEMERAL_KEY,
                sdp_body=SDP_OFFER,
                model=MODEL,
                session={"instructions": INSTRUCTIONS},
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        theirs_call = server.calls[-1]
    ours_fields = posted_fields(ours_call)
    theirs_fields = posted_fields(theirs_call)
    assert ours_call["path"] == theirs_call["path"] == CALLS_PATH
    assert ours == theirs.text == SDP_ANSWER
    assert ours_fields["sdp"] == theirs_fields["sdp"] == SDP_OFFER
    ours_session = JSON.validate_json(ours_fields["session"])
    assert ours_session == JSON.validate_json(theirs_fields["session"])
    assert ours_session == {
        "type": "realtime",
        "model": RESOLVED_MODEL,
        "instructions": INSTRUCTIONS,
    }


def test_provider_error() -> None:
    reference_error = REFERENCE_ERRORS.BaseLLMException
    with closing(RealtimeServer()) as server:
        with pytest.raises(reference_error) as theirs:
            asyncio.run(
                REFERENCE_SECRET.acreate_realtime_client_secret(
                    model=MODEL,
                    api_base=server.url,
                    api_key=BAD_KEY,
                )
            )
        with pytest.raises(BaseLLMException) as ours:
            asyncio.run(
                acreate_realtime_client_secret(
                    model=MODEL,
                    api_base=server.url,
                    api_key=BAD_KEY,
                )
            )
    assert error_fields(ours.value).status_code == UNAUTHORIZED
    assert error_fields(theirs.value).status_code == UNAUTHORIZED
    assert error_fields(ours.value).message == AUTH_TEXT
    assert error_fields(theirs.value).message == AUTH_TEXT
    assert ours.value.__class__.__name__ == theirs.value.__class__.__name__
    assert theirs.value.__class__.__name__ == "BaseLLMException"
    failure = FAILURE.validate_json(str(native_cause(ours.value)))
    assert failure["type"] == "BaseLLMException"
    assert failure["message"] == AUTH_TEXT
    assert failure["status_code"] == UNAUTHORIZED


def test_unsupported_objects() -> None:
    with pytest.raises(UnsupportedCallArgument, match="client"):
        asyncio.run(acreate_realtime_client_secret(model=MODEL, client="client"))
    with pytest.raises(UnsupportedCallArgument, match="shared_session"):
        asyncio.run(acreate_realtime_transcription_session(shared_session="session"))
    with pytest.raises(UnsupportedCallArgument, match="client"):
        asyncio.run(
            arealtime_calls(
                openai_ephemeral_key=EPHEMERAL_KEY,
                sdp_body=SDP_OFFER,
                client="client",
            )
        )
