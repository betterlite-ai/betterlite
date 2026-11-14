from typing import Literal

from typing_extensions import NotRequired, TypedDict


# ast-grep-ignore: no-long-names
class OpenAIRealtimeStreamResponseOutputItemContent(TypedDict, total=False):
    audio: str
    id: str
    text: str
    transcript: str | None
    type: Literal[
        "input_audio",
        "input_text",
        "text",
        "audio",
        "item_reference",
        "output_text",
        "output_audio",
    ]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeStreamResponseOutputItem(TypedDict, total=False):
    arguments: str
    call_id: str
    id: str
    content: list[OpenAIRealtimeStreamResponseOutputItemContent]  # ast-grep-ignore: no-long-names
    name: str
    object: Literal["realtime.item"]
    role: Literal["assistant", "user", "system"]
    status: Literal["completed", "incomplete", "in_progress"]
    output: str
    type: Literal["function_call", "message", "function_call_output"]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeStreamResponseOutputItemAdded(TypedDict):
    type: Literal["response.output_item.added"]
    response_id: str
    output_index: int
    item: OpenAIRealtimeStreamResponseOutputItem  # ast-grep-ignore: no-long-names
    event_id: NotRequired[str]
