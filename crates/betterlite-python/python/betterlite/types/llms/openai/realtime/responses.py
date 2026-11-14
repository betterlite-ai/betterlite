from typing import Literal

from pydantic import JsonValue
from typing_extensions import TypedDict

from betterlite.types.llms.openai.realtime.items import (
    OpenAIRealtimeStreamResponseOutputItem,
)


class OpenAIRealtimeResponseContentPart(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    audio: str
    text: str
    transcript: str | None
    type: Literal["audio", "text", "output_audio", "output_text"]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeResponseContentPartAdded(TypedDict):
    type: Literal["response.content_part.added"]
    content_index: int
    event_id: str
    item_id: str
    output_index: int
    part: OpenAIRealtimeResponseContentPart  # ast-grep-ignore: no-long-names
    response_id: str


class OpenAIRealtimeResponseDelta(TypedDict):
    content_index: int
    delta: str
    event_id: str
    item_id: str
    output_index: int
    response_id: str
    type: Literal[
        "response.text.delta",
        "response.audio.delta",
        "response.output_text.delta",
        "response.output_audio.delta",
        "response.output_audio_transcript.delta",
    ]


class OpenAIRealtimeResponseTextDone(TypedDict):
    content_index: int
    event_id: str
    item_id: str
    output_index: int
    response_id: str
    text: str
    type: Literal["response.text.done", "response.output_text.done"]


class OpenAIRealtimeResponseAudioDone(TypedDict):
    content_index: int
    event_id: str
    item_id: str
    output_index: int
    response_id: str
    type: Literal[
        "response.audio.done", "response.output_audio.done", "response.output_audio_transcript.done"
    ]


class OpenAIRealtimeContentPartDone(TypedDict):  # ast-grep-ignore: no-long-names
    content_index: int
    event_id: str
    item_id: str
    output_index: int
    response_id: str
    part: OpenAIRealtimeResponseContentPart  # ast-grep-ignore: no-long-names
    type: Literal["response.content_part.done"]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeFunctionCallArgumentsDone(TypedDict):
    type: Literal["response.function_call_arguments.done"]
    event_id: str
    response_id: str
    item_id: str
    output_index: int
    call_id: str
    name: str
    arguments: str


class OpenAIRealtimeOutputItemDone(TypedDict):
    event_id: str
    item: OpenAIRealtimeStreamResponseOutputItem  # ast-grep-ignore: no-long-names
    output_index: int
    response_id: str
    type: Literal["response.output_item.done"]


class OpenAIRealtimeResponseDoneObject(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    conversation_id: str
    id: str
    max_output_tokens: int
    metadata: dict[str, JsonValue]
    modalities: list[JsonValue]
    object: Literal["realtime.response"]
    output: list[OpenAIRealtimeStreamResponseOutputItem]  # ast-grep-ignore: no-long-names
    output_audio_format: str
    status: Literal["completed", "cancelled", "failed", "incomplete"]
    status_details: dict[str, JsonValue]
    temperature: float
    usage: dict[str, JsonValue]
    voice: str


class OpenAIRealtimeDoneEvent(TypedDict):
    event_id: str
    response: OpenAIRealtimeResponseDoneObject  # ast-grep-ignore: no-long-names
    type: Literal["response.done"]
