from typing import Literal, TypeAlias, TypedDict

from pydantic import JsonValue

JsonObject: TypeAlias = dict[str, JsonValue]
Document: TypeAlias = str | JsonObject
Kind: TypeAlias = Literal["retrieval", "ranking"]


class ChatSupported(TypedDict):
    kind: Literal["chat_supported"]
    models: list[str]


class ChatMapCase(TypedDict):
    model: str
    non_default: JsonObject
    optional: JsonObject


class ChatMap(TypedDict):
    kind: Literal["chat_map"]
    cases: list[ChatMapCase]


class EmbeddingCase(TypedDict):
    non_default: JsonObject
    optional: JsonObject
    kwargs: JsonObject | None


class EmbeddingMap(TypedDict):
    kind: Literal["embedding_map"]
    cases: list[EmbeddingCase]


class SelectUrl(TypedDict):
    model: str
    api_base: str | None


class RerankSelect(TypedDict):
    kind: Literal["rerank_select"]
    models: list[str]
    urls: list[SelectUrl]


class RerankCase(TypedDict):
    config: Kind
    model: str
    non_default: JsonObject | None
    top_n: int | None
    query: str
    documents: list[Document]


class RerankParams(TypedDict):
    kind: Literal["rerank_params"]
    cases: list[RerankCase]


class HeaderCase(TypedDict):
    api_key: str | None
    env_key: str | None
    global_key: str | None
    headers: dict[str, str]


class RerankHeaders(TypedDict):
    kind: Literal["rerank_headers"]
    cases: list[HeaderCase]


class RerankRequest(TypedDict):
    kind: Literal["rerank_request"]
    cases: list[RerankCase]


class ResponseCase(TypedDict):
    config: Kind
    model: str
    status: int
    body: str
    headers: dict[str, str]
    request_data: JsonObject
    top_n: int | None
    query: str
    documents: list[Document]


class RerankResponseCase(TypedDict):
    kind: Literal["rerank_response"]
    cases: list[ResponseCase]


class UrlCase(TypedDict):
    api_base: str | None
    env_base: str | None
    model: str
    model_group: str | None


class PassthroughUrl(TypedDict):
    kind: Literal["passthrough_url"]
    cases: list[UrlCase]


class CredentialCase(TypedDict):
    api_key: str | None
    env_key: str | None
    headers: dict[str, str]
    stream: JsonValue


class PassthroughCredentials(TypedDict):
    kind: Literal["passthrough_credentials"]
    cases: list[CredentialCase]


Input: TypeAlias = (
    ChatSupported
    | ChatMap
    | EmbeddingMap
    | RerankSelect
    | RerankParams
    | RerankHeaders
    | RerankRequest
    | RerankResponseCase
    | PassthroughUrl
    | PassthroughCredentials
)
