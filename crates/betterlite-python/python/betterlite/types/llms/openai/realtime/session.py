from typing import Literal, TypeAlias

from pydantic import JsonValue
from typing_extensions import Required, TypedDict

REASONING_EFFORT: TypeAlias = Literal["none", "minimal", "low", "medium", "high", "xhigh", "max"]


class OpenAIRealtimeStreamSession(TypedDict, total=False):
    id: Required[str]
    input_audio_format: str
    input_audio_noise_reduction: JsonValue
    input_audio_transcription: JsonValue
    instructions: str
    max_response_output_tokens: int | Literal["inf"]
    modalities: list[str]
    model: str
    output_audio_format: str
    temperature: float
    tool_choice: str
    tools: list[JsonValue]
    turn_detection: JsonValue
    voice: str


# ast-grep-ignore: no-long-names
class OpenAIRealtimeStreamSessionEvents(TypedDict):
    event_id: str
    session: OpenAIRealtimeStreamSession
    type: Literal["session.created", "session.updated"]


class OpenAIRealtimeTurnDetection(TypedDict, total=False):
    create_response: bool
    eagerness: str
    interrupt_response: bool
    prefix_padding_ms: int
    silence_duration_ms: int
    threshold: int
    type: str
