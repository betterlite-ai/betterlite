import base64
import io
from collections.abc import Callable, Mapping, Sequence
from typing import IO, Annotated, Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.openai.videos.transformation import OpenAIVideoConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.main import CharacterObject, VideoObject
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)
TEXT: Final = TypeAdapter(str)

Content: TypeAlias = IO[bytes] | bytes | str
StoredFiles: TypeAlias = (
    Content
    | tuple[str | None, Content]
    | tuple[str | None, Content, str | None]
    | tuple[str | None, Content, str | None, Mapping[str, str]]
)
Files: TypeAlias = Sequence[tuple[str, StoredFiles]]
Parts: TypeAlias = Mapping[str, StoredFiles] | Sequence[tuple[str, StoredFiles]]
Kind: TypeAlias = Literal["bytes", "text", "stream"]
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


class CreateCharacterRequest(TypedDict):
    op: Literal["create_character_request"]
    name: str
    fixture: Fixture
    api_base: str


class CreateCharacterResponse(TypedDict):
    op: Literal["create_character_response"]
    body: str


class GetCharacterRequest(TypedDict):
    op: Literal["get_character_request"]
    character_id: str
    api_base: str


class GetCharacterResponse(TypedDict):
    op: Literal["get_character_response"]
    body: str


class EditRequest(TypedDict):
    op: Literal["edit_request"]
    prompt: str
    video_id: str
    api_base: str
    fixture: Fixture | None
    extra_body: EditBody | None
    prefetched_source_data: EditBody | None


class EditResponse(TypedDict):
    op: Literal["edit_response"]
    body: str
    provider: str | None
    request_data: EditBody | None


class ExtensionRequest(TypedDict):
    op: Literal["extension_request"]
    prompt: str
    video_id: str
    seconds: str
    api_base: str
    extra_body: EditBody | None


class ExtensionResponse(TypedDict):
    op: Literal["extension_response"]
    body: str
    provider: str | None


Request: TypeAlias = (
    CreateCharacterRequest | GetCharacterRequest | EditRequest | ExtensionRequest
)
Reply: TypeAlias = (
    CreateCharacterResponse | GetCharacterResponse | EditResponse | ExtensionResponse
)
Call: TypeAlias = Annotated[
    CreateCharacterRequest
    | CreateCharacterResponse
    | GetCharacterRequest
    | GetCharacterResponse
    | EditRequest
    | EditResponse
    | ExtensionRequest
    | ExtensionResponse,
    Field(discriminator="op"),
]
CALLS: TypeAdapter[list[Call]] = TypeAdapter(list[Call])


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


def create_character_value(call: CreateCharacterRequest) -> JsonValue:
    state = state_of(call["fixture"])
    url, files = CONFIG.transform_video_create_character_request(
        call["name"], state.content, call["api_base"], GenericLiteLLMParams(), {}
    )
    match files:
        case [
            (name_field, (None, name)),
            (video_field, (filename, _, content_type)),
            *_,
        ]:
            parts: list[JsonValue] = [
                part_value(name_field, None, None, "text", TEXT.validate_python(name)),
                part_value(video_field, filename, content_type, state.kind, state.body),
            ]
            rendered: dict[str, JsonValue] = {"url": url, "files": parts}
            return rendered
        case _:
            raise ValueError("the transform returned no name field and video file")


def get_character_value(call: GetCharacterRequest) -> JsonValue:
    url, _ = CONFIG.transform_video_get_character_request(
        call["character_id"], call["api_base"], GenericLiteLLMParams(), {}
    )
    rendered: dict[str, JsonValue] = {"url": url}
    return rendered


def edit_files(files: Parts, state: FixtureState | None) -> JsonValue:
    match files:
        case [(field, (filename, _, content_type)), *_]:
            if state is None:
                raise ValueError("a multipart edit needs a fixture")
            return [part_value(field, filename, content_type, state.kind, state.body)]
        case _:
            raise ValueError("the transform returned no video file")


def edit_value(call: EditRequest) -> JsonValue:
    fixture = call["fixture"]
    state = None if fixture is None else state_of(fixture)
    content = None if state is None else state.content
    url, data, files = CONFIG.transform_video_edit_request(
        call["prompt"],
        call["video_id"],
        call["api_base"],
        GenericLiteLLMParams(),
        {},
        content,
        call["extra_body"],
        call["prefetched_source_data"],
    )
    rendered_files = None if files is None else edit_files(files, state)
    rendered: dict[str, JsonValue] = {
        "url": url,
        "body": JSON.validate_python(dict(data)),
        "files": rendered_files,
    }
    return rendered


def extension_value(call: ExtensionRequest) -> JsonValue:
    url, data = CONFIG.transform_video_extension_request(
        call["prompt"],
        call["video_id"],
        call["seconds"],
        call["api_base"],
        GenericLiteLLMParams(),
        {},
        call["extra_body"],
    )
    rendered: dict[str, JsonValue] = {"url": url, "body": dict(data)}
    return rendered


def reply(body: str) -> httpx.Response:
    request = httpx.Request("POST", "https://mock.example.test/v1/videos")
    return httpx.Response(200, content=body.encode(), request=request)


def create_character_reply(call: CreateCharacterResponse) -> JsonValue:
    transformed = CONFIG.transform_video_create_character_response(
        reply(call["body"]), Logger()
    )
    return JSON.validate_json(transformed.model_dump_json())


def get_character_reply(call: GetCharacterResponse) -> JsonValue:
    transformed = CONFIG.transform_video_get_character_response(
        reply(call["body"]), Logger()
    )
    return JSON.validate_json(transformed.model_dump_json())


def edit_reply(call: EditResponse) -> JsonValue:
    transformed = CONFIG.transform_video_edit_response(
        reply(call["body"]), Logger(), call["provider"], call["request_data"]
    )
    return JSON.validate_json(transformed.model_dump_json())


def extension_reply(call: ExtensionResponse) -> JsonValue:
    transformed = CONFIG.transform_video_extension_response(
        reply(call["body"]), Logger(), call["provider"]
    )
    return JSON.validate_json(transformed.model_dump_json())


def request_value(call: Request) -> JsonValue:
    match call["op"]:
        case "create_character_request":
            return create_character_value(call)
        case "get_character_request":
            return get_character_value(call)
        case "edit_request":
            return edit_value(call)
        case "extension_request":
            return extension_value(call)


def reply_value(call: Reply) -> JsonValue:
    match call["op"]:
        case "create_character_response":
            return create_character_reply(call)
        case "get_character_response":
            return get_character_reply(call)
        case "edit_response":
            return edit_reply(call)
        case "extension_response":
            return extension_reply(call)


def call_value(call: Call) -> JsonValue:
    match call["op"]:
        case "create_character_request" | "get_character_request" | "edit_request" | "extension_request":
            return request_value(call)
        case "create_character_response" | "get_character_response" | "edit_response" | "extension_response":
            return reply_value(call)


def run(raw: dict[str, JsonValue]) -> JsonValue:
    calls = CALLS.validate_python(raw["calls"])
    values: list[JsonValue] = [call_value(call) for call in calls]
    return values
