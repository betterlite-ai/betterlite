from collections.abc import Awaitable, Callable, Coroutine, Mapping
from typing import Final, TypeAlias

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite._audio.binary import HttpxBinaryResponseContent
from betterlite._audio.errors import raise_audio_error
from betterlite._endpoints import (
    CallPayload,
    CallValue,
    model_from_arguments,
    read_settings,
    to_call_payload,
)
from betterlite._native import NativeAudioError, NativeAudioSession
from betterlite.types.utils.transcription import (
    TranscriptionResponse,
    TranscriptionUsageTokensObject,
)
from betterlite.types.utils.transcription import (
    TranscriptionUsageDurationObject as DurationUsage,
)

SETTINGS: Final[tuple[str, ...]] = (
    "api_base", "api_key", "num_retries", "openai_key", "organization", "project",
)
TRANSCRIPTION_NAMES: Final[tuple[str, ...]] = (
    "model", "file", "language", "prompt", "response_format", "timestamp_granularities",
    "temperature", "user", "timeout", "api_key", "api_base", "api_version", "max_retries",
    "custom_llm_provider", "base_url",
)
SPEECH_NAMES: Final[tuple[str, ...]] = (
    "model", "input", "voice", "api_key", "api_base", "api_version", "organization", "project",
    "max_retries", "metadata", "timeout", "response_format", "speed", "instructions", "client",
    "headers", "custom_llm_provider", "aspeech", "base_url",
)
DROPPED: Final[tuple[str, ...]] = ("atranscription", "client", "logger_fn", "shared_session")
OPTIONAL_FIELDS: Final[tuple[str, ...]] = (
    "language", "task", "duration", "words", "segments",
)

SyncSpeech: TypeAlias = Callable[[str, list[bytes]], tuple[bytes, str]]
AsyncSpeech: TypeAlias = Callable[[str, list[bytes]], Awaitable[tuple[bytes, str]]]
SpeechResult: TypeAlias = (
    HttpxBinaryResponseContent | Coroutine[None, None, HttpxBinaryResponseContent]
)

TEXT: Final[TypeAdapter[str | None]] = TypeAdapter(str | None)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class SpeechHeader(TypedDict):
    status_code: int
    headers: dict[str, str]


ANSWER: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
SPEECH_HEADER: Final[TypeAdapter[SpeechHeader]] = TypeAdapter(SpeechHeader)
DURATION_USAGE: Final[TypeAdapter[DurationUsage]] = TypeAdapter(DurationUsage)
TOKENS_USAGE: Final[TypeAdapter[TranscriptionUsageTokensObject]] = TypeAdapter(
    TranscriptionUsageTokensObject,
)


def transcription(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L7933
    model: CallValue,
    file: CallValue,
    language: CallValue = None,
    prompt: CallValue = None,
    response_format: CallValue = None,
    timestamp_granularities: CallValue = None,
    temperature: CallValue = None,
    user: CallValue = None,
    timeout: CallValue = 600,
    api_key: CallValue = None,
    api_base: CallValue = None,
    api_version: CallValue = None,
    max_retries: CallValue = None,
    custom_llm_provider: CallValue = None,
    base_url: CallValue = None,
    **kwargs: CallValue,
) -> TranscriptionResponse:
    given = (model, file, language, prompt, response_format, timestamp_granularities,
             temperature, user, timeout, api_key, api_base, api_version, max_retries,
             custom_llm_provider, base_url)
    arguments = merge_arguments(TRANSCRIPTION_NAMES, given, kwargs)
    flags = {"atranscription": False, "file_from_kwargs": False}
    return transcription_call(audio_payload(arguments, flags))


async def atranscription(*args: CallValue, **kwargs: CallValue) -> TranscriptionResponse:
    _ = model_from_arguments(args, kwargs)
    arguments = merge_arguments(TRANSCRIPTION_NAMES, args, kwargs)
    flags = {"atranscription": True, "file_from_kwargs": "file" in kwargs}
    return await atranscription_call(audio_payload(arguments, flags))


def merge_arguments(
    names: tuple[str, ...],
    positional: tuple[CallValue, ...],
    kwargs: Mapping[str, CallValue],
) -> dict[str, CallValue]:
    given = dict(zip(names, positional, strict=False))
    return {**kwargs, **given}


def audio_payload(
    values: Mapping[str, CallValue], flags: Mapping[str, CallValue],
) -> CallPayload:
    payload: dict[str, CallValue] = {"settings": read_settings(SETTINGS), **flags}
    payload["arguments"] = {
        name: value for name, value in values.items()
        if value is not None and name not in DROPPED
    }
    payload.update({name: values.get(name) for name in ("client", "logger_fn", "shared_session")})
    return to_call_payload(payload)


def transcription_call(payload: CallPayload) -> TranscriptionResponse:
    try:
        answer = NativeAudioSession("{}").transcription(payload.text, payload.blobs)
    except NativeAudioError as failure:
        raise_audio_error(failure)
    return transcription_response(ANSWER.validate_json(answer))


async def atranscription_call(payload: CallPayload) -> TranscriptionResponse:
    try:
        answer = await NativeAudioSession("{}").atranscription(payload.text, payload.blobs)
    except NativeAudioError as failure:
        raise_audio_error(failure)
    return transcription_response(ANSWER.validate_json(answer))


# ast-grep-ignore: no-long-names
def transcription_response_from_cached_dict(
    cached_response_dict: Mapping[str, JsonValue],
) -> TranscriptionResponse:
    response = TranscriptionResponse(TEXT.validate_python(cached_response_dict.get("text")))
    for name in OPTIONAL_FIELDS:
        if name in cached_response_dict:
            response[name] = JSON.validate_python(cached_response_dict[name])
    match cached_response_dict.get("usage"):
        case {"type": "duration"} as usage:
            response["usage"] = DURATION_USAGE.validate_python(usage)
        case {"type": "tokens"} as usage:
            response["usage"] = TOKENS_USAGE.validate_python(usage)
        case object():
            pass
    return response


def transcription_response(answer: dict[str, JsonValue]) -> TranscriptionResponse:
    response = transcription_response_from_cached_dict(answer)
    response["_response_ms"] = answer["response_ms"]
    response["_hidden_params"] = answer["hidden_params"]
    return response


def speech(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L8238
    model: CallValue,
    input: CallValue,  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L8240
    voice: CallValue = None,
    api_key: CallValue = None,
    api_base: CallValue = None,
    api_version: CallValue = None,
    organization: CallValue = None,
    project: CallValue = None,
    max_retries: CallValue = None,
    metadata: CallValue = None,
    timeout: CallValue = None,
    response_format: CallValue = None,
    speed: CallValue = None,
    instructions: CallValue = None,
    client: CallValue = None,
    headers: CallValue = None,
    custom_llm_provider: CallValue = None,
    aspeech: CallValue = None,
    base_url: CallValue = None,
    **kwargs: CallValue,
) -> SpeechResult:
    given = (model, input, voice, api_key, api_base, api_version, organization, project,
             max_retries, metadata, timeout, response_format, speed, instructions, client,
             headers, custom_llm_provider, aspeech, base_url)
    arguments = merge_arguments(SPEECH_NAMES, given, kwargs)
    pending = bool(aspeech)
    payload = audio_payload(arguments, {"aspeech": pending})
    session = NativeAudioSession("{}")
    if pending:
        return aspeech_response(session.aspeech, payload)
    return speech_response(*call_speech(session.speech, payload))


async def aspeech(*args: CallValue, **kwargs: CallValue) -> HttpxBinaryResponseContent:
    _ = model_from_arguments(args, kwargs)
    arguments = merge_arguments(SPEECH_NAMES, args, kwargs)
    payload = audio_payload(arguments, {"aspeech": True})
    session = NativeAudioSession("{}")
    return await aspeech_response(session.aspeech, payload)


def call_speech(method: SyncSpeech, payload: CallPayload) -> tuple[bytes, str]:
    try:
        return method(payload.text, payload.blobs)
    except NativeAudioError as failure:
        raise_audio_error(failure)


async def acall_speech(method: AsyncSpeech, payload: CallPayload) -> tuple[bytes, str]:
    try:
        return await method(payload.text, payload.blobs)
    except NativeAudioError as failure:
        raise_audio_error(failure)


def speech_response(content: bytes, header_text: str) -> HttpxBinaryResponseContent:
    header = SPEECH_HEADER.validate_json(header_text)
    return HttpxBinaryResponseContent(content, header["status_code"], header["headers"])


async def aspeech_response(
    method: AsyncSpeech, payload: CallPayload,
) -> HttpxBinaryResponseContent:
    content, header_text = await acall_speech(method, payload)
    return speech_response(content, header_text)
