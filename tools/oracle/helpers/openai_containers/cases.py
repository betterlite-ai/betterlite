from typing import Literal, TypeAlias, TypedDict

from litellm.types.containers.main import ContainerCreateOptionalRequestParams as CreateParams
from pydantic import JsonValue

ReplyKind: TypeAlias = Literal["create", "retrieve", "list", "delete", "file_list"]


class Globals(TypedDict):
    api_base: str | None
    api_key: str | None
    openai_key: str | None


class MapCall(TypedDict):
    params: CreateParams
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


class QueryCall(TypedDict):
    after: str | None
    limit: int | None
    order: str | None
    extra_query: dict[str, JsonValue] | None


class ListCall(TypedDict):
    kind: Literal["list"]
    api_base: str
    query: QueryCall


class FileListCall(TypedDict):
    kind: Literal["file_list"]
    container_id: str
    api_base: str
    query: QueryCall


class ResourceCall(TypedDict):
    kind: Literal["retrieve", "delete"]
    container_id: str
    api_base: str


class ContentCall(TypedDict):
    kind: Literal["content"]
    container_id: str
    file_id: str
    api_base: str


RequestCall: TypeAlias = ListCall | FileListCall | ResourceCall | ContentCall


class HeadersCall(TypedDict):
    api_key: str | None
    headers: dict[str, str]
    globals: Globals
    environment: dict[str, str]


class ReplyCall(TypedDict):
    kind: ReplyKind
    body: str


class BodyCall(TypedDict):
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
    requests: list[RequestCall]
    headers: list[HeadersCall]
    replies: list[ReplyCall]
    content_replies: list[BodyCall]
    errors: list[ErrorCall]
