from typing import Literal

from typing_extensions import ReadOnly, TypedDict


class OpenAIRealtimeInputAudioBufferSpeechEvent(TypedDict):  # ast-grep-ignore: no-long-names
    type: ReadOnly[
        Literal["input_audio_buffer.speech_started", "input_audio_buffer.speech_stopped"]
    ]
    event_id: ReadOnly[str]
    item_id: ReadOnly[str]


class OpenAIRealtimeErrorDetail(TypedDict):
    type: ReadOnly[str]
    message: ReadOnly[str]


class OpenAIRealtimeErrorEvent(TypedDict):
    type: ReadOnly[Literal["error"]]
    error: ReadOnly[OpenAIRealtimeErrorDetail]
