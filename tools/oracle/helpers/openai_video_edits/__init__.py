from typing import Annotated, Literal, TypeAlias, TypedDict

import httpx
from litellm.types.router import GenericLiteLLMParams
from pydantic import Field, JsonValue, TypeAdapter

from .fixtures import (
    CONFIG,
    EditBody,
    Fixture,
    FixtureState,
    Logger,
    Parts,
    part_value,
    state_of,
)

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
TEXT: TypeAdapter[str] = TypeAdapter(str)


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


def call_value(call: Call) -> JsonValue:
    match call["op"]:
        case "create_character_request":
            value = create_character_value(call)
        case "create_character_response":
            value = create_character_reply(call)
        case "get_character_request":
            value = get_character_value(call)
        case "get_character_response":
            value = get_character_reply(call)
        case "edit_request":
            value = edit_value(call)
        case "edit_response":
            value = edit_reply(call)
        case "extension_request":
            value = extension_value(call)
        case "extension_response":
            value = extension_reply(call)
    return value


def run(raw: dict[str, JsonValue]) -> JsonValue:
    calls = CALLS.validate_python(raw["calls"])
    values: list[JsonValue] = [call_value(call) for call in calls]
    return values
