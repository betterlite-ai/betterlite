import base64
import io
from collections.abc import Callable, Mapping, Sequence
from typing import IO, Annotated, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.openai.videos.transformation import OpenAIVideoConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.main import CharacterObject, VideoObject
from pydantic import Field, JsonValue
from typing_extensions import override

Kind: TypeAlias = Literal["bytes", "text", "stream"]
Content: TypeAlias = IO[bytes] | bytes | str
StoredFiles: TypeAlias = (
    Content
    | tuple[str | None, Content]
    | tuple[str | None, Content, str | None]
    | tuple[str | None, Content, str | None, Mapping[str, str]]
)
Files: TypeAlias = Sequence[tuple[str, StoredFiles]]
Parts: TypeAlias = Mapping[str, StoredFiles] | Sequence[tuple[str, StoredFiles]]
JsonObject: TypeAlias = Mapping[str, JsonValue]
EditBody: TypeAlias = dict[str, object]
EditForm: TypeAlias = Mapping[str, object]
EditResult: TypeAlias = tuple[str, EditForm, Parts | None]


class VideoConfig(Protocol):
    @property
    def transform_video_create_character_request(  # ast-grep-ignore: no-long-names
        self,
    ) -> Callable[
        [str, Content, str, GenericLiteLLMParams, dict[str, str]],
        tuple[str, Files],
    ]: ...

    @property
    def transform_video_create_character_response(  # ast-grep-ignore: no-long-names
        self,
    ) -> Callable[[httpx.Response, Logging], CharacterObject]: ...

    @property
    def transform_video_get_character_request(  # ast-grep-ignore: no-long-names
        self,
    ) -> Callable[
        [str, str, GenericLiteLLMParams, dict[str, str]],
        tuple[str, JsonObject],
    ]: ...

    @property
    def transform_video_get_character_response(  # ast-grep-ignore: no-long-names
        self,
    ) -> Callable[[httpx.Response, Logging], CharacterObject]: ...

    @property
    def transform_video_edit_request(
        self,
    ) -> Callable[
        [
            str,
            str,
            str,
            GenericLiteLLMParams,
            dict[str, str],
            Content | None,
            EditBody | None,
            EditBody | None,
        ],
        EditResult,
    ]: ...

    @property
    def transform_video_edit_response(
        self,
    ) -> Callable[
        [httpx.Response, Logging, str | None, EditBody | None],
        VideoObject,
    ]: ...

    @property
    def transform_video_extension_request(  # ast-grep-ignore: no-long-names
        self,
    ) -> Callable[
        [str, str, str, str, GenericLiteLLMParams, dict[str, str], EditBody | None],
        tuple[str, JsonObject],
    ]: ...

    @property
    def transform_video_extension_response(  # ast-grep-ignore: no-long-names
        self,
    ) -> Callable[[httpx.Response, Logging, str | None], VideoObject]: ...


class VideoConfigImpl(OpenAIVideoConfig):
    # OpenAIVideoConfig.__init__ carries no annotations; this subclass states the one mypy needs
    def __init__(self) -> None:
        pass


CONFIG: VideoConfig = VideoConfigImpl()


class Logger(Logging):
    @override
    def __init__(self) -> None:
        pass


class BytesFixture(TypedDict):
    kind: Literal["bytes"]
    data_base64: str


class TextFixture(TypedDict):
    kind: Literal["text"]
    value: str


class StreamFixture(TypedDict):
    kind: Literal["stream"]
    data_base64: str
    name: str | None


Fixture: TypeAlias = Annotated[
    BytesFixture | TextFixture | StreamFixture,
    Field(discriminator="kind"),
]


class NamedBytesIO(io.BytesIO):
    def __init__(self, data: bytes, name: str) -> None:
        super().__init__(data)
        self.name = name


class FixtureState:
    def __init__(self, content: Content, kind: Kind, body: JsonValue) -> None:
        self.content = content
        self.kind = kind
        self.body = body


def state_of(spec: Fixture) -> FixtureState:
    match spec["kind"]:
        case "bytes":
            data = base64.b64decode(spec["data_base64"])
            return FixtureState(data, "bytes", base64.b64encode(data).decode("ascii"))
        case "text":
            return FixtureState(spec["value"], "text", spec["value"])
        case "stream":
            data = base64.b64decode(spec["data_base64"])
            name = spec["name"]
            if name is None:
                return FixtureState(io.BytesIO(data), "stream", None)
            return FixtureState(NamedBytesIO(data, name), "stream", None)


def part_value(
    field: str,
    filename: str | None,
    content_type: str | None,
    kind: Kind,
    body: JsonValue,
) -> JsonValue:
    rendered: dict[str, JsonValue] = {
        "field": field,
        "filename": filename,
        "content_type": content_type,
        "body_kind": kind,
        "body": body,
    }
    return rendered
