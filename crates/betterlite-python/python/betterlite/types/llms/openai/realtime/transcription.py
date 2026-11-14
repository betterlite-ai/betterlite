from collections.abc import Mapping
from typing import Literal, NotRequired

from pydantic import JsonValue
from typing_extensions import ReadOnly, TypedDict


# ast-grep-ignore: no-long-names
class OpenAIRealtimeTranscriptionAudioFormat(TypedDict):
    type: ReadOnly[Literal["audio/pcm"]]
    rate: ReadOnly[int]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeTranscriptionSettings(TypedDict):
    model: ReadOnly[str]
    language: NotRequired[ReadOnly[str]]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeServerVadTurnDetection(TypedDict):
    type: ReadOnly[Literal["server_vad"]]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeTranscriptionAudioInput(TypedDict):
    # ast-grep-ignore: no-long-names
    format: ReadOnly[OpenAIRealtimeTranscriptionAudioFormat]
    # ast-grep-ignore: no-long-names
    transcription: ReadOnly[OpenAIRealtimeTranscriptionSettings]
    # ast-grep-ignore: no-long-names
    turn_detection: ReadOnly[OpenAIRealtimeServerVadTurnDetection | None]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeTranscriptionAudio(TypedDict):
    # ast-grep-ignore: no-long-names
    input: ReadOnly[OpenAIRealtimeTranscriptionAudioInput]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeTranscriptionSession(TypedDict):
    id: ReadOnly[str]
    object: ReadOnly[Literal["realtime.transcription_session"]]
    type: ReadOnly[Literal["transcription"]]
    # ast-grep-ignore: no-long-names
    audio: ReadOnly[OpenAIRealtimeTranscriptionAudio]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeTranscriptionSessionCreated(TypedDict):
    type: ReadOnly[Literal["session.created"]]
    event_id: ReadOnly[str]
    # ast-grep-ignore: no-long-names
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
