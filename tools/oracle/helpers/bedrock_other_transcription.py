import asyncio
import sys
from types import ModuleType
from typing import Final, Literal, NotRequired, TypedDict

import httpx
from litellm.llms.bedrock.audio_transcription import BedrockAudioTranscriptionRustDispatch
from litellm.rust_bridge import reset_native_bridge_cache
from litellm.types.utils import FileTypes, TranscriptionResponse
from pydantic import JsonValue

NATIVE_MODULE: Final = "litellm.rust_bridge._native"
UNSUPPORTED_PREFIX: Final = "Unsupported Bedrock audio format"


class BytesFile(TypedDict):
    kind: Literal["bytes"]
    content: str


class TupleFile(TypedDict):
    kind: Literal["tuple"]
    filename: str
    content: str


class Phases(TypedDict):
    connect: float | None
    read: float | None
    write: float | None
    pool: float | None


class Call(TypedDict):
    mode: Literal["sync", "async"]
    model: str
    file: BytesFile | TupleFile
    custom_llm_provider: str
    optional_params: dict[str, JsonValue]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    extra_headers: NotRequired[dict[str, JsonValue] | None]
    timeout: NotRequired[float | None]
    phases: NotRequired[Phases | None]
    reply_text: NotRequired[str | None]


class Calls(TypedDict):
    calls: list[Call]


class FakeNative(ModuleType):
    def __init__(self, reply: dict[str, JsonValue]) -> None:
        super().__init__(NATIVE_MODULE)
        self.reply = reply
        self.seen: list[JsonValue] = []

    def transcription(self, **arguments: JsonValue) -> dict[str, JsonValue]:
        self.seen.append(arguments)
        return self.reply

    async def atranscription(self, **arguments: JsonValue) -> dict[str, JsonValue]:
        return self.transcription(**arguments)


def audio_file(file: BytesFile | TupleFile) -> FileTypes:
    content = file["content"].encode()
    if file["kind"] == "bytes":
        return content
    return (file["filename"], content)


def timeout_argument(call: Call) -> float | httpx.Timeout | None:
    phases = call.get("phases")
    if phases is None:
        return call.get("timeout")
    return httpx.Timeout(
        connect=phases["connect"],
        read=phases["read"],
        write=phases["write"],
        pool=phases["pool"],
    )


def sync_call(call: Call) -> TranscriptionResponse:
    headers = call.get("extra_headers")
    return BedrockAudioTranscriptionRustDispatch().audio_transcriptions(
        model=call["model"],
        audio_file=audio_file(call["file"]),
        api_key=call.get("api_key"),
        api_base=call.get("api_base"),
        custom_llm_provider=call["custom_llm_provider"],
        extra_headers=None if headers is None else dict(headers),
        optional_params=dict(call["optional_params"]),
        timeout=timeout_argument(call),
    )


async def async_call(call: Call) -> TranscriptionResponse:
    headers = call.get("extra_headers")
    return await BedrockAudioTranscriptionRustDispatch().async_audio_transcriptions(
        model=call["model"],
        audio_file=audio_file(call["file"]),
        api_key=call.get("api_key"),
        api_base=call.get("api_base"),
        custom_llm_provider=call["custom_llm_provider"],
        extra_headers=None if headers is None else dict(headers),
        optional_params=dict(call["optional_params"]),
        timeout=timeout_argument(call),
    )


def one_call(call: Call) -> dict[str, JsonValue]:
    text = call.get("reply_text")
    native = FakeNative({} if text is None else {"text": text})
    sys.modules[NATIVE_MODULE] = native
    reset_native_bridge_cache()
    try:
        response = sync_call(call) if call["mode"] == "sync" else asyncio.run(async_call(call))
    except ValueError as error:
        if not str(error).startswith(UNSUPPORTED_PREFIX):
            raise
        return {"ok": False, "kind": "unsupported_format", "seen": native.seen}
    finally:
        del sys.modules[NATIVE_MODULE]
        reset_native_bridge_cache()
    return {"ok": True, "text": response.text, "seen": native.seen}


def run(input_value: Calls) -> dict[str, JsonValue]:
    calls: list[JsonValue] = [one_call(call) for call in input_value["calls"]]
    return {"calls": calls}
