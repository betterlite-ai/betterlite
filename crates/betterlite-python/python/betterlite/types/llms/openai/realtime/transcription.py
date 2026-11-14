from collections.abc import Mapping
from typing import Literal

from pydantic import JsonValue
from typing_extensions import NotRequired, ReadOnly, TypedDict


# ast-grep-ignore: no-long-names
class OpenAIRealtimeTranscriptionAudioFormat(TypedDict):
    type: ReadOnly[Literal["audio/pcm"]]
    rate: ReadOnly[int]


class OpenAIRealtimeTranscriptionSettings(TypedDict):
    model: ReadOnly[str]
    language: NotRequired[ReadOnly[str]]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeServerVadTurnDetection(TypedDict):
    type: ReadOnly[Literal["server_vad"]]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeTranscriptionAudioInput(TypedDict):
    format: ReadOnly[OpenAIRealtimeTranscriptionAudioFormat]
    transcription: ReadOnly[OpenAIRealtimeTranscriptionSettings]
    # ast-grep-ignore: no-long-names
    turn_detection: ReadOnly[OpenAIRealtimeServerVadTurnDetection | None]


class OpenAIRealtimeTranscriptionAudio(TypedDict):
    input: ReadOnly[OpenAIRealtimeTranscriptionAudioInput]


class OpenAIRealtimeTranscriptionSession(TypedDict):
    id: ReadOnly[str]
    object: ReadOnly[Literal["realtime.transcription_session"]]
    type: ReadOnly[Literal["transcription"]]
    audio: ReadOnly[OpenAIRealtimeTranscriptionAudio]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeTranscriptionSessionCreated(TypedDict):
    type: ReadOnly[Literal["session.created"]]
    event_id: ReadOnly[str]
    session: ReadOnly[OpenAIRealtimeTranscriptionSession]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeInputAudioTranscriptionDelta(TypedDict):
    type: ReadOnly[Literal["conversation.item.input_audio_transcription.delta"]]
    event_id: ReadOnly[str]
    item_id: ReadOnly[str]
    content_index: ReadOnly[int]
    delta: ReadOnly[str]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeInputAudioTranscriptionCompleted(TypedDict):
    type: ReadOnly[Literal["conversation.item.input_audio_transcription.completed"]]
    event_id: ReadOnly[str]
    item_id: ReadOnly[str]
    content_index: ReadOnly[int]
    transcript: ReadOnly[str]
    usage: NotRequired[ReadOnly[Mapping[str, JsonValue]]]
