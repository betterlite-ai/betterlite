from typing import Literal, TypeAlias, TypedDict

from pydantic import JsonValue

Object: TypeAlias = dict[str, JsonValue]


class ChoicesChunk(TypedDict):
    choices: list[Object]


class EventChunk(TypedDict):
    type: str


class UrlStep(TypedDict):
    kind: Literal["url"]
    model: str
    embedding: bool
    api_base: str | None
    account_env: str | None
    params: Object


class HeadersStep(TypedDict):
    kind: Literal["headers"]
    model: str
    api_key: str | None
    headers: Object


class ParamsStep(TypedDict):
    kind: Literal["params"]
    model: str
    embedding: bool
    non_default: Object


class InfoStep(TypedDict):
    kind: Literal["info"]
    api_base: str | None
    api_key: str | None
    jwt_env: str | None


class RequestStep(TypedDict):
    kind: Literal["request"]
    model: str
    messages: list[Object]
    options: Object


class ResponseStep(TypedDict):
    kind: Literal["response"]
    model: str
    body: str
    request_data: Object
    messages: list[Object]


class EmbeddingRequestStep(TypedDict):
    kind: Literal["embedding_request"]
    model: str
    text: str | list[str]
    options: Object


class EmbeddingResponseStep(TypedDict):
    kind: Literal["embedding_response"]
    model: str
    body: str


class ChunkStep(TypedDict):
    kind: Literal["chunk"]
    chunk: ChoicesChunk


class EventStep(TypedDict):
    kind: Literal["event"]
    chunk: EventChunk


Step: TypeAlias = (
    UrlStep
    | HeadersStep
    | ParamsStep
    | InfoStep
    | RequestStep
    | ResponseStep
    | EmbeddingRequestStep
    | EmbeddingResponseStep
    | ChunkStep
    | EventStep
)


class Input(TypedDict):
    steps: list[Step]
