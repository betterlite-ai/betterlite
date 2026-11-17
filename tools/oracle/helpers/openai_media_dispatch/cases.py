from typing import Annotated, Final, Literal, TypeAlias, TypedDict

import litellm
from pydantic import Field, JsonValue, TypeAdapter

litellm.suppress_debug_info = True


class ImageFile(TypedDict):
    content: str
    name: str | None


class ImageGenerationCase(TypedDict):
    case: Literal["image_generation"]
    api_base: str
    model: str
    prompt: str
    options: dict[str, JsonValue]
    status: int
    reply: JsonValue


class ImageEditCase(TypedDict):
    case: Literal["image_edit"]
    api_base: str
    model: str
    prompt: str | None
    image: list[ImageFile]
    mask: list[ImageFile] | None
    options: dict[str, JsonValue]


class ImageVariationCase(TypedDict):
    case: Literal["image_variation"]
    api_base: str
    model: str
    image_filename: str | None
    image_content: str
    image_content_type: str | None
    options: dict[str, JsonValue]
    status: int
    reply: JsonValue


class SpeechOptions(TypedDict, total=False):
    response_format: str
    speed: float
    instructions: str
    extra_headers: dict[str, str]


class SpeechCase(TypedDict):
    case: Literal["speech"]
    api_base: str
    model: str
    input: str
    voice: str
    options: SpeechOptions
    status: int
    content_type: str | None
    body: str


TranscriptionFamily: TypeAlias = Literal["whisper", "gpt"]


class TranscriptionCase(TypedDict):
    case: Literal["transcription"]
    api_base: str
    model: str
    family: TranscriptionFamily
    options: dict[str, JsonValue]
    status: int
    reply: str


class VideoCreateCase(TypedDict):
    case: Literal["video_create"]
    api_base: str
    model: str
    prompt: str
    options: dict[str, JsonValue]
    reference: str | None
    provider: str | None
    reply: str


VideoLifecycleOp: TypeAlias = Literal["list", "status", "delete", "content", "remix"]


class VideoLifecycleCase(TypedDict):
    case: Literal["video_lifecycle"]
    api_base: str
    provider: str | None
    op: VideoLifecycleOp
    video_id: str | None
    after: str | None
    limit: int | None
    order: str | None
    variant: str | None
    prompt: str | None
    reply: str


VideoExtendedOp: TypeAlias = Literal["edit", "extension", "create_character", "get_character"]


class VideoExtendedCase(TypedDict):
    case: Literal["video_extended_operations"]
    api_base: str
    provider: str | None
    model: str
    op: VideoExtendedOp
    prompt: str | None
    video_id: str | None
    video_file: str | None
    extra_body: dict[str, JsonValue] | None
    seconds: str | None
    name: str | None
    video: str | None
    character_id: str | None
    reply: str


MediaCase: TypeAlias = Annotated[
    (
        ImageGenerationCase
        | ImageEditCase
        | ImageVariationCase
        | SpeechCase
        | TranscriptionCase
        | VideoCreateCase
        | VideoLifecycleCase
        | VideoExtendedCase
    ),
    Field(discriminator="case"),
]
CASES: Final = TypeAdapter(MediaCase)

ImageCases: TypeAlias = ImageGenerationCase | ImageEditCase | ImageVariationCase
AudioCases: TypeAlias = SpeechCase | TranscriptionCase
VideoCases: TypeAlias = VideoCreateCase | VideoLifecycleCase
