import re
from typing import Literal, TypedDict

import litellm
from litellm.llms.base_llm.realtime import transcription_protocol
from litellm.types.realtime import (
    RealtimeInputAudioTranscriptionTokenUsage as TranscriptionTokenUsage,
)
from litellm.types.realtime import RealtimeInputAudioTranscriptionUsageInputTokenDetails
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

litellm.suppress_debug_info = True

_JSON: TypeAdapter[Json] = TypeAdapter(Json)
_EVENT_ID = re.compile(r"event_[0-9a-f]{32}")


class TokenUsageDocument(TypedDict):
    input_tokens: int
    output_tokens: int
    total_tokens: int
    text_tokens: int
    audio_tokens: int


class ParseInput(TypedDict):
    function: Literal["parse"]
    payloads: list[str]


class DecodeInput(TypedDict):
    function: Literal["decode"]
    audio: list[Json]
    max_encoded_bytes: int | None


class EventInput(TypedDict):
    function: Literal["event"]
    event_ids: list[str]
    session_id: str
    model: str
    sample_rate: int
    language: str | None
    server_vad: bool
    item_id: str
    delta: str
    transcript: str
    duration_seconds: float
    token_usage: TokenUsageDocument
    error_message: str


def _audio_format(audio: transcription_protocol.TranscriptionAudioFormat) -> Json:
    document = {
        "layout": audio.layout,
        "encoding": audio.encoding,
        "rate": audio.rate,
        "channels": audio.channels,
    }
    return _JSON.validate_python(document)


def _session_document(payload: str) -> Json:
    try:
        update = transcription_protocol.parse_transcription_session_update(payload)
    except transcription_protocol.RealtimeTranscriptionProtocolError:
        return None
    audio = update.audio_format
    document = {
        "session_type": update.session_type,
        "audio_format": None if audio is None else _audio_format(audio),
        "is_pcm16": None if audio is None else audio.is_pcm16,
        "model": update.model,
        "language": update.language,
        "unsupported_transcription_keys": list(update.unsupported_transcription_keys),
        "turn_detection": None if update.turn_detection is None else dict(update.turn_detection),
        "turn_detection_disabled": update.turn_detection_disabled,
        "turn_detection_type": update.turn_detection_type,
    }
    return _JSON.validate_python(document)


def _parse(input_value: ParseInput) -> Json:
    sessions: list[Json] = [_session_document(payload) for payload in input_value["payloads"]]
    document = {"sessions": sessions}
    return _JSON.validate_python(document)


def _decoded_bytes(audio: Json, limit: int | None) -> Json:
    try:
        decoded = transcription_protocol.decode_pcm16_append(audio, limit)
    except transcription_protocol.RealtimeTranscriptionProtocolError:
        return None
    octets = list(decoded)
    return _JSON.validate_python(octets)


def _decode(input_value: DecodeInput) -> Json:
    limit = input_value["max_encoded_bytes"]
    decoded: list[Json] = [_decoded_bytes(audio, limit) for audio in input_value["audio"]]
    document = {"decoded": decoded}
    return _JSON.validate_python(document)


def _token_usage(document: TokenUsageDocument) -> TranscriptionTokenUsage:
    return TranscriptionTokenUsage(
        type="tokens",
        input_tokens=document["input_tokens"],
        output_tokens=document["output_tokens"],
        total_tokens=document["total_tokens"],
        input_token_details=RealtimeInputAudioTranscriptionUsageInputTokenDetails(
            text_tokens=document["text_tokens"],
            audio_tokens=document["audio_tokens"],
        ),
    )


def _event(input_value: EventInput) -> Json:
    session = transcription_protocol.transcription_session(
        session_id=input_value["session_id"],
        model=input_value["model"],
        sample_rate=input_value["sample_rate"],
        language=input_value["language"],
        server_vad=input_value["server_vad"],
    )
    without_language = transcription_protocol.transcription_session(
        session_id=input_value["session_id"],
        model=input_value["model"],
        sample_rate=input_value["sample_rate"],
        language=None,
        server_vad=False,
    )
    item_id = input_value["item_id"]
    transcript = input_value["transcript"]
    created = transcription_protocol.transcription_session_created_event(session)
    started = transcription_protocol.speech_event("input_audio_buffer.speech_started", item_id)
    stopped = transcription_protocol.speech_event("input_audio_buffer.speech_stopped", item_id)
    delta = transcription_protocol.delta_event(item_id, input_value["delta"])
    silent = transcription_protocol.completed_event(item_id, transcript, None)
    timeline = transcription_protocol.duration_usage(input_value["duration_seconds"])
    timed = transcription_protocol.completed_event(item_id, transcript, timeline)
    billed = transcription_protocol.completed_event(
        item_id, transcript, _token_usage(input_value["token_usage"])
    )
    denied = transcription_protocol.error_event(input_value["error_message"])
    identifiers = [
        created["event_id"],
        started["event_id"],
        stopped["event_id"],
        delta["event_id"],
        silent["event_id"],
        timed["event_id"],
        billed["event_id"],
    ]
    matches = all(_EVENT_ID.fullmatch(identifier) is not None for identifier in identifiers)
    ids = input_value["event_ids"]
    document = {
        "generated_id_matches": matches,
        "created": {**created, "event_id": ids[0]},
        "speech_started": {**started, "event_id": ids[1]},
        "speech_stopped": {**stopped, "event_id": ids[2]},
        "delta": {**delta, "event_id": ids[3]},
        "completed_none": {**silent, "event_id": ids[4]},
        "completed_duration": {**timed, "event_id": ids[5]},
        "completed_tokens": {**billed, "event_id": ids[6]},
        "error": denied,
        "session": session,
        "session_without_language": without_language,
    }
    return _JSON.validate_python(document)


def run(input_value: ParseInput | DecodeInput | EventInput) -> Json:
    if input_value["function"] == "parse":
        return _parse(input_value)
    if input_value["function"] == "decode":
        return _decode(input_value)
    return _event(input_value)
