from os import PathLike
from typing import IO, Literal, TypeAlias

from pydantic import Field, JsonValue, PrivateAttr
from typing_extensions import ReadOnly, TypedDict

from betterlite.types.llms.base import LiteLLMPydanticObjectBase
from betterlite.types.utils.base import InitValue

FileTypes: TypeAlias = IO[bytes] | bytes | PathLike[str]


class _VideoDictAccess(LiteLLMPydanticObjectBase):
    def __getitem__(self, key: str) -> InitValue:
        # ast-grep-ignore: no-reflection
        found: InitValue = getattr(self, key)
        return found

    def get(self, key: str, default: InitValue = None) -> InitValue:
        # ast-grep-ignore: no-reflection
        found: InitValue = getattr(self, key, default)
        return found

    def __contains__(self, key: str) -> bool:
        # ast-grep-ignore: no-reflection
        return hasattr(self, key)


class VideoObject(_VideoDictAccess):
    id: str
    object: Literal["video"]
    status: str
    created_at: int | None = None
    completed_at: int | None = None
    expires_at: int | None = None
    error: dict[str, JsonValue] | None = None
    progress: int | None = None
    remixed_from_video_id: str | None = None
    seconds: str | None = None
    size: str | None = None
    model: str | None = None
    usage: dict[str, JsonValue] | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)


class VideoResponse(_VideoDictAccess):
    data: list[VideoObject]
    hidden_params: dict[str, JsonValue] = Field(default_factory=dict)


# ast-grep-ignore: no-long-names
class VideoCreateOptionalRequestParams(TypedDict, total=False):
    input_reference: FileTypes | None
    image: JsonValue
    parameters: dict[str, JsonValue] | None
    model: str | None
    resolution: ReadOnly[str | None]
    seconds: str | None
    size: str | None
    characters: list[dict[str, str]] | None
    user: str | None
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None


# ast-grep-ignore: no-long-names
class VideoCreateRequestParams(VideoCreateOptionalRequestParams, total=False):
    prompt: str


class DecodedVideoId(TypedDict, total=False):
    custom_llm_provider: str | None
    model_id: str | None
    video_id: str


class CharacterObject(_VideoDictAccess):
    id: str
    object: Literal["character"] = "character"
    created_at: int
    name: str
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)


class VideoEditRequestParams(TypedDict, total=False):
    prompt: str
    video: dict[str, str]


class VideoExtensionRequestParams(TypedDict, total=False):
    prompt: str
    seconds: str
    video: dict[str, str]
