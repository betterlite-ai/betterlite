from collections.abc import Mapping
from typing import Literal, TypeAlias

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.values import JsonValue, StoredValue, ValidationError


class InputTextContent(ResponseObject):
    text: str
    type: Literal["input_text"]
    prompt_cache_breakpoint: JsonValue


class OutputTextContent(ResponseObject):
    text: str
    type: Literal["output_text"]


class InputImageContent(ResponseObject):
    image_url: str
    type: Literal["input_image"]
    detail: str | None


class InputAudioContent(ResponseObject):
    input_audio: JsonValue
    type: Literal["input_audio"]


ContentItem: TypeAlias = (
    str | InputTextContent | OutputTextContent | InputImageContent | InputAudioContent
)
GraderContent: TypeAlias = ContentItem | list[ContentItem]


def content_item(value: StoredValue) -> StoredValue:
    match value:
        case list():
            return [content_item(item) for item in value]
        case Mapping() if "type" not in value:
            raise ValidationError("content", ("type",), "value is required")
        case Mapping():
            selected: ResponseObject
            match value["type"]:
                case "input_text":
                    selected = InputTextContent(**value)
                case "output_text":
                    selected = OutputTextContent(**value)
                case "input_image":
                    selected = InputImageContent(**value)
                case "input_audio":
                    selected = InputAudioContent(**value)
                case _other:
                    raise ValidationError("content", ("type",), "unknown content type")
            return selected
        case _other:
            return value
