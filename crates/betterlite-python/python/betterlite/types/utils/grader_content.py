from typing import Literal, TypeAlias

from pydantic import JsonValue

from betterlite.types.utils.base import ResponseObject


class InputTextContent(ResponseObject):
    text: str
    type: Literal["input_text"]
    prompt_cache_breakpoint: JsonValue = None


class OutputTextContent(ResponseObject):
    text: str
    type: Literal["output_text"]


class InputImageContent(ResponseObject):
    image_url: str
    type: Literal["input_image"]
    detail: str | None = None


class InputAudioContent(ResponseObject):
    input_audio: JsonValue
    type: Literal["input_audio"]


ContentItem: TypeAlias = (
    str | InputTextContent | OutputTextContent | InputImageContent | InputAudioContent
)
GraderContent: TypeAlias = ContentItem | list[ContentItem]
GraderRole: TypeAlias = Literal["user", "assistant", "system", "developer"]


class GraderInput(ResponseObject):
    content: GraderContent
    role: GraderRole
    type: Literal["message"] | None = None
