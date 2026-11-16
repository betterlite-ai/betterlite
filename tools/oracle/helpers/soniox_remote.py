import asyncio
import base64
from collections.abc import Callable, Coroutine
from functools import partial
from types import SimpleNamespace
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

import httpx
import litellm
import litellm.llms.soniox.audio_transcription.handler as soniox_handler
from litellm.llms.soniox.audio_transcription.handler import (
    SonioxAudioTranscriptionHandler as Handler,
)
from litellm.llms.soniox.audio_transcription.transformation import (
    SonioxAudioTranscriptionConfig as Config,
)
from litellm.llms.soniox.common_utils import SonioxException
from litellm.types.utils import FileTypes
from pydantic import BaseModel, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
CONFIG: Final = Config.__new__(Config)
HANDLER: Final = Handler()
FileMap: TypeAlias = dict[str, tuple[str, bytes, str]]


class Answer(TypedDict):
    status: int
    headers: dict[str, str]
    body: str
    fail: NotRequired[bool]


class BaseCall(TypedDict):
    auth_headers: dict[str, str]
    api_base: str
    timeout_ms: int
    answers: list[Answer]


class UploadCall(BaseCall):
    function: Literal["upload"]
    filename: str
    content: str
    filename_override: str | None


class PollCall(BaseCall):
    function: Literal["poll"]
    transcription_id: str
    poll_interval: float
    max_attempts: int


class CleanupCall(BaseCall):
    function: Literal["cleanup"]
    cleanup: list[str]
    transcription_id: str | None
    file_id: str | None


Call: TypeAlias = UploadCall | PollCall | CleanupCall


class Input(TypedDict):
    calls: list[Call]


class Timers(Protocol):
    time: SimpleNamespace
    asyncio: SimpleNamespace


SONIOX: Final[Timers] = cast(Timers, soniox_handler)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/soniox/audio_transcription/handler.py#L18-L20


class Script:
    def __init__(self, answers: list[Answer], auth_headers: dict[str, str]) -> None:
        self.answers = list(answers)
        self.auth_headers = auth_headers
        self.sent: list[dict[str, JsonValue]] = []
        self.sleeps: list[int] = []

    def sleep(self, seconds: float) -> None:
        milliseconds = round(seconds * 1000)
        self.sleeps.append(milliseconds)

    async def async_sleep(self, seconds: float) -> None:
        milliseconds = round(seconds * 1000)
        self.sleeps.append(milliseconds)

    def reply(self, entry: dict[str, JsonValue]) -> httpx.Response:
        self.sent.append(entry)
        if not self.answers:
            raise httpx.ConnectError("the scripted transport has no answer left")
        answer = self.answers.pop(0)
        if answer.get("fail"):
            raise httpx.ConnectError("the scripted transport failed")
        return httpx.Response(
            answer["status"], headers=answer["headers"], content=answer["body"].encode()
        )


class SyncClient(Script):
    def post(
        self, url: str, headers: dict[str, str], files: FileMap, timeout: float
    ) -> httpx.Response:
        entry = record("POST", url, headers, self.auth_headers, timeout)
        entry["files"] = files_sent(files)
        return self.reply(entry)

    def get(self, url: str, headers: dict[str, str], timeout: float) -> httpx.Response:
        return self.reply(record("GET", url, headers, self.auth_headers, timeout))

    def delete(self, url: str, headers: dict[str, str], timeout: float) -> httpx.Response:
        return self.reply(record("DELETE", url, headers, self.auth_headers, timeout))


class AsyncClient(Script):
    async def post(
        self, url: str, headers: dict[str, str], files: FileMap, timeout: float
    ) -> httpx.Response:
        entry = record("POST", url, headers, self.auth_headers, timeout)
        entry["files"] = files_sent(files)
        return self.reply(entry)

    async def get(self, url: str, headers: dict[str, str], timeout: float) -> httpx.Response:
        return self.reply(record("GET", url, headers, self.auth_headers, timeout))

    async def delete(self, url: str, headers: dict[str, str], timeout: float) -> httpx.Response:
        return self.reply(record("DELETE", url, headers, self.auth_headers, timeout))


class CommonArgs(TypedDict):
    http_client: SyncClient | AsyncClient
    base_url: str
    auth_headers: dict[str, str]
    timeout: float


class UploadArgs(CommonArgs):
    audio_file: FileTypes
    filename_override: str | None
    provider_config: Config


class PollArgs(CommonArgs):
    transcription_id: str
    poll_interval: float
    max_attempts: int
    provider_config: Config


class CleanupArgs(CommonArgs):
    cleanup: list[str]
    file_id_to_cleanup: str | None
    transcription_id: str | None


SYNC_UPLOAD, ASYNC_UPLOAD, SYNC_POLL, ASYNC_POLL, SYNC_CLEANUP, ASYNC_CLEANUP = cast(Callable[..., JsonValue], HANDLER._sync_upload_file), cast(Callable[..., Coroutine[None, None, JsonValue]], HANDLER._async_upload_file), cast(Callable[..., JsonValue], HANDLER._sync_poll_until_completed), cast(Callable[..., Coroutine[None, None, JsonValue]], HANDLER._async_poll_until_completed), cast(Callable[..., JsonValue], HANDLER._sync_cleanup), cast(Callable[..., Coroutine[None, None, JsonValue]], HANDLER._async_cleanup)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/soniox/audio_transcription/handler.py#L482-L588


class ErrorRecord(BaseModel):
    status_code: int
    message: str
    headers: dict[str, str] | None


def provider_outcome(error: Exception, answers: list[Answer]) -> JsonValue:
    captured = ErrorRecord.model_validate(error, from_attributes=True)
    furnished: dict[str, str] = {} if captured.headers is None else captured.headers
    names = {name.lower() for answer in answers for name in answer["headers"]}
    pictured = {name: value for name, value in furnished.items() if name.lower() in names}
    return {
        "kind": "provider",
        "status": captured.status_code,
        "message": captured.message,
        "headers": JSON.validate_python(pictured),
    }


def record(
    method: str, url: str, headers: dict[str, str], expected: dict[str, str], timeout: float
) -> dict[str, JsonValue]:
    pictured = {name.lower(): headers.get(name) for name in expected}
    return {
        "method": method,
        "url": url,
        "headers": JSON.validate_python(pictured),
        "timeout_ms": round(timeout * 1000),
    }


def files_sent(files: FileMap) -> JsonValue:
    parts = [
        {
            "name": name,
            "filename": file_name,
            "content_type": content_type,
            "body": base64.b64encode(content).decode(),
        }
        for name, (file_name, content, content_type) in files.items()
    ]
    return JSON.validate_python(parts)


def client_for(call: Call, *, asynchronous: bool) -> SyncClient | AsyncClient:
    if asynchronous:
        return AsyncClient(call["answers"], call["auth_headers"])
    return SyncClient(call["answers"], call["auth_headers"])


def observe(
    kwargs: CommonArgs,
    answers: list[Answer],
    sync_fn: Callable[..., JsonValue],
    asynchronous_fn: Callable[..., Coroutine[None, None, JsonValue]],
    *,
    asynchronous: bool,
) -> JsonValue:
    client = kwargs["http_client"]
    SONIOX.time = SimpleNamespace(sleep=client.sleep)
    SONIOX.asyncio = SimpleNamespace(sleep=client.async_sleep)
    invoke: Callable[[], JsonValue] = (
        partial(asyncio.run, asynchronous_fn(**kwargs))
        if asynchronous
        else partial(sync_fn, **kwargs)
    )
    outcome: JsonValue
    try:
        outcome = {"kind": "returned", "value": invoke()}
    except SonioxException as error:
        outcome = provider_outcome(error, answers)
    except httpx.HTTPError:
        outcome = {"kind": "transport"}
    return {
        "requests": JSON.validate_python(client.sent),
        "sleeps": JSON.validate_python(client.sleeps),
        "outcome": outcome,
    }


def upload_once(call: UploadCall, *, asynchronous: bool) -> JsonValue:
    kwargs: UploadArgs = {
        "http_client": client_for(call, asynchronous=asynchronous),
        "base_url": call["api_base"],
        "auth_headers": call["auth_headers"],
        "audio_file": (call["filename"], call["content"].encode()),
        "filename_override": call["filename_override"],
        "timeout": call["timeout_ms"] / 1000,
        "provider_config": CONFIG,
    }
    return observe(kwargs, call["answers"], SYNC_UPLOAD, ASYNC_UPLOAD, asynchronous=asynchronous)


def poll_once(call: PollCall, *, asynchronous: bool) -> JsonValue:
    kwargs: PollArgs = {
        "http_client": client_for(call, asynchronous=asynchronous),
        "base_url": call["api_base"],
        "auth_headers": call["auth_headers"],
        "transcription_id": call["transcription_id"],
        "poll_interval": call["poll_interval"],
        "max_attempts": call["max_attempts"],
        "timeout": call["timeout_ms"] / 1000,
        "provider_config": CONFIG,
    }
    return observe(kwargs, call["answers"], SYNC_POLL, ASYNC_POLL, asynchronous=asynchronous)


def cleanup_once(call: CleanupCall, *, asynchronous: bool) -> JsonValue:
    kwargs: CleanupArgs = {
        "http_client": client_for(call, asynchronous=asynchronous),
        "base_url": call["api_base"],
        "auth_headers": call["auth_headers"],
        "cleanup": call["cleanup"],
        "file_id_to_cleanup": call["file_id"],
        "transcription_id": call["transcription_id"],
        "timeout": call["timeout_ms"] / 1000,
    }
    return observe(kwargs, call["answers"], SYNC_CLEANUP, ASYNC_CLEANUP, asynchronous=asynchronous)


def answer(call: Call) -> JsonValue:
    match call["function"]:
        case "upload":
            sync = upload_once(call, asynchronous=False)
            return {"sync": sync, "async": upload_once(call, asynchronous=True)}
        case "poll":
            sync = poll_once(call, asynchronous=False)
            return {"sync": sync, "async": poll_once(call, asynchronous=True)}
        case "cleanup":
            sync = cleanup_once(call, asynchronous=False)
            return {"sync": sync, "async": cleanup_once(call, asynchronous=True)}


def run(input_value: Input) -> JsonValue:
    return {"calls": [answer(call) for call in input_value["calls"]]}
