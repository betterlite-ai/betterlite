from typing import Literal, TypeAlias, TypedDict

from pydantic import JsonValue

ResourceKind: TypeAlias = Literal["retrieve", "delete"]
MetadataKind: TypeAlias = Literal["retrieve", "list", "delete", "file_list"]


class Globals(TypedDict):
    api_base: str | None
    api_key: str | None
    openai_key: str | None


class MapCall(TypedDict):
    params: dict[str, JsonValue]
    drop_params: bool


class JoinCall(TypedDict):
    api_base: str
    path_suffix: str


class UrlCall(TypedDict):
    api_base: str | None
    globals: Globals
    environment: dict[str, str]


class CreateCall(TypedDict):
    name: str
    params: dict[str, JsonValue]


class ListCall(TypedDict):
    api_base: str
    after: str | None
    limit: int | None
    order: str | None
    extra_query: dict[str, JsonValue] | None


class FileListCall(TypedDict):
    container_id: str
    api_base: str
    after: str | None
    limit: int | None
    order: str | None
    extra_query: dict[str, JsonValue] | None


class ResourceCall(TypedDict):
    kind: ResourceKind
    container_id: str
    api_base: str


class ContentCall(TypedDict):
    container_id: str
    file_id: str
    api_base: str


class HeadersCall(TypedDict):
    api_key: str | None
    headers: dict[str, str]
    globals: Globals
    environment: dict[str, str]


class ReplyCall(TypedDict):
    body: str


class MetadataCall(TypedDict):
    kind: MetadataKind
    body: str


class ErrorCall(TypedDict):
    message: str
    status: int
    headers: dict[str, str]


class Input(TypedDict):
    supported: bool
    maps: list[MapCall]
    joins: list[JoinCall]
    urls: list[UrlCall]
    creates: list[CreateCall]
    lists: list[ListCall]
    file_lists: list[FileListCall]
    resources: list[ResourceCall]
    contents: list[ContentCall]
    headers: list[HeadersCall]
    create_replies: list[ReplyCall]
    metadata_replies: list[MetadataCall]
    content_replies: list[ReplyCall]
    errors: list[ErrorCall]
