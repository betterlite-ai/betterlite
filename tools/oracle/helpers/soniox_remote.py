import asyncio
import base64
from collections.abc import Callable, Coroutine
from functools import partial
from types import SimpleNamespace
from typing import Final, Generic, Literal, Protocol, Required, TypeAlias, TypedDict, TypeVar, cast

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
Result = TypeVar("Result")


class Answer(TypedDict, total=False):
    status: Required[int]
    headers: Required[dict[str, str]]
    body: Required[str]
    fail: bool


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


SONIOX: Final[Timers] = cast(Timers, soniox_handler)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/soniox/audio_transcription/handler.py#L18-L20


class Script:
    def __init__(self, answers: list[Answer], auth_headers: dict[str, str]) -> None:
        self.answers = list(answers)
        self.auth_headers = auth_headers
        self.sent: list[dict[str, JsonValue]] = []
        self.sleeps: list[int] = []
        # The source helpers look up time.sleep and asyncio.sleep on their own module when they run
        SONIOX.time = SimpleNamespace(sleep=self.sleep)
        SONIOX.asyncio = SimpleNamespace(sleep=self.async_sleep)

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
        self, url: str, headers: dict[str, str], files: FileMap, timeout: float  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L799
    ) -> httpx.Response:
        entry = record("POST", url, headers, self.auth_headers, timeout)
        entry["files"] = files_sent(files)
        return self.reply(entry)

    async def get(self, url: str, headers: dict[str, str], timeout: float) -> httpx.Response:  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L713
        return self.reply(record("GET", url, headers, self.auth_headers, timeout))

    async def delete(self, url: str, headers: dict[str, str], timeout: float) -> httpx.Response:  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L1011
        return self.reply(record("DELETE", url, headers, self.auth_headers, timeout))


UploadArguments: TypeAlias = tuple[
    Script, str, dict[str, str], FileTypes, str | None, float, Config]
PollArguments: TypeAlias = tuple[
    Script, str, dict[str, str], str, float, int, float, Config]
CleanupArguments: TypeAlias = tuple[
    Script, str, dict[str, str], list[str], str | None, str | None, float]

SyncUpload: TypeAlias = Callable[
    [Script, str, dict[str, str], FileTypes, str | None, float, Config], str
]
AsyncUpload: TypeAlias = Callable[
    [Script, str, dict[str, str], FileTypes, str | None, float, Config],
    Coroutine[None, None, str]]
SyncPoll: TypeAlias = Callable[
    [Script, str, dict[str, str], str, float, int, float, Config], JsonValue
]
AsyncPoll: TypeAlias = Callable[
    [Script, str, dict[str, str], str, float, int, float, Config],
    Coroutine[None, None, JsonValue]]
SyncCleanup: TypeAlias = Callable[
    [Script, str, dict[str, str], list[str], str | None, str | None, float], None
]
AsyncCleanup: TypeAlias = Callable[
    [Script, str, dict[str, str], list[str], str | None, str | None, float],
    Coroutine[None, None, None]]

SYNC_UPLOAD: SyncUpload = cast(SyncUpload, HANDLER._sync_upload_file)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/soniox/audio_transcription/handler.py#L482
ASYNC_UPLOAD: AsyncUpload = cast(AsyncUpload, HANDLER._async_upload_file)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/soniox/audio_transcription/handler.py#L716
SYNC_POLL: SyncPoll = cast(SyncPoll, HANDLER._sync_poll_until_completed)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/soniox/audio_transcription/handler.py#L509
ASYNC_POLL: AsyncPoll = cast(AsyncPoll, HANDLER._async_poll_until_completed)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/soniox/audio_transcription/handler.py#L742
SYNC_CLEANUP: SyncCleanup = cast(SyncCleanup, HANDLER._sync_cleanup)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/soniox/audio_transcription/handler.py#L550
ASYNC_CLEANUP: AsyncCleanup = cast(AsyncCleanup, HANDLER._async_cleanup)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/soniox/audio_transcription/handler.py#L783


class CoroutineJob(Generic[Result]):
    def __init__(self, coroutine: Coroutine[None, None, Result]) -> None:
        self.coroutine = coroutine

    def __call__(self) -> Result:
        return asyncio.run(self.coroutine)


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
        "kind": "provider", "status": captured.status_code,
        "message": captured.message, "headers": JSON.validate_python(pictured),
    }


def record(
    method: str, url: str, headers: dict[str, str], expected: dict[str, str], timeout: float
) -> dict[str, JsonValue]:
    pictured = {name.lower(): headers.get(name) for name in expected}
    return {
        "method": method, "url": url,
        "headers": JSON.validate_python(pictured), "timeout_ms": round(timeout * 1000),
    }


def files_sent(files: FileMap) -> JsonValue:
    parts = [
        {
            "name": name, "filename": file_name,
            "content_type": content_type, "body": base64.b64encode(content).decode(),
        }
        for name, (file_name, content, content_type) in files.items()
    ]
    return JSON.validate_python(parts)


def observe(client: Script, answers: list[Answer], invoke: Callable[[], Result]) -> JsonValue:
    outcome: JsonValue
    try:
        outcome = {"kind": "returned", "value": JSON.validate_python(invoke())}
    except SonioxException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/soniox/common_utils.py#L53-L56
        outcome = provider_outcome(error, answers)
    except httpx.HTTPError:
        outcome = {"kind": "transport"}
    return {
        "requests": JSON.validate_python(client.sent),
        "sleeps": JSON.validate_python(client.sleeps), "outcome": outcome,
    }


def upload_arguments(call: UploadCall, client: Script) -> UploadArguments:
    audio: FileTypes = (call["filename"], call["content"].encode())
    return (client, call["api_base"], call["auth_headers"], audio,
            call["filename_override"], call["timeout_ms"] / 1000, CONFIG)


def poll_arguments(call: PollCall, client: Script) -> PollArguments:
    return (client, call["api_base"], call["auth_headers"], call["transcription_id"],
            call["poll_interval"], call["max_attempts"], call["timeout_ms"] / 1000, CONFIG)


def cleanup_arguments(call: CleanupCall, client: Script) -> CleanupArguments:
    return (client, call["api_base"], call["auth_headers"], call["cleanup"],
            call["file_id"], call["transcription_id"], call["timeout_ms"] / 1000)


def upload_sync(call: UploadCall) -> JsonValue:
    client = SyncClient(call["answers"], call["auth_headers"])
    invoke: Callable[[], str] = partial(SYNC_UPLOAD, *upload_arguments(call, client))
    return observe(client, call["answers"], invoke)


def upload_async(call: UploadCall) -> JsonValue:
    client = AsyncClient(call["answers"], call["auth_headers"])
    invoke: Callable[[], str] = CoroutineJob(ASYNC_UPLOAD(*upload_arguments(call, client)))
    return observe(client, call["answers"], invoke)


def poll_sync(call: PollCall) -> JsonValue:
    client = SyncClient(call["answers"], call["auth_headers"])
    invoke: Callable[[], JsonValue] = partial(SYNC_POLL, *poll_arguments(call, client))
    return observe(client, call["answers"], invoke)


def poll_async(call: PollCall) -> JsonValue:
    client = AsyncClient(call["answers"], call["auth_headers"])
    invoke: Callable[[], JsonValue] = CoroutineJob(ASYNC_POLL(*poll_arguments(call, client)))
    return observe(client, call["answers"], invoke)


def cleanup_sync(call: CleanupCall) -> JsonValue:
    client = SyncClient(call["answers"], call["auth_headers"])
    invoke: Callable[[], None] = partial(SYNC_CLEANUP, *cleanup_arguments(call, client))
    return observe(client, call["answers"], invoke)


def cleanup_async(call: CleanupCall) -> JsonValue:
    client = AsyncClient(call["answers"], call["auth_headers"])
    invoke: Callable[[], None] = CoroutineJob(ASYNC_CLEANUP(*cleanup_arguments(call, client)))
    return observe(client, call["answers"], invoke)


def answer(call: Call) -> JsonValue:
    match call["function"]:
        case "upload":
            return {"sync": upload_sync(call), "async": upload_async(call)}
        case "poll":
            return {"sync": poll_sync(call), "async": poll_async(call)}
        case "cleanup":
            return {"sync": cleanup_sync(call), "async": cleanup_async(call)}


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}
