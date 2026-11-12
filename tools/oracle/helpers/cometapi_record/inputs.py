import os
from typing import Literal, NotRequired, Protocol, TypedDict

from litellm.types.llms.openai import AllEmbeddingInputValues
from pydantic import JsonValue

EMPTY: dict[str, JsonValue] = {}


class HeaderLookup(Protocol):
    def get(self, name: str) -> str | None: ...


class Message(TypedDict):
    role: str
    content: str | list[JsonValue]
    name: NotRequired[str]
    tool_calls: NotRequired[list[JsonValue]]
    tool_call_id: NotRequired[str]
    cache_control: NotRequired[JsonValue]


NO_MESSAGES: list[Message] = []


class Catalog(TypedDict):
    chat: list[str]
    text: list[str]


class OptionsStep(TypedDict):
    kind: Literal["options"]
    model: str
    params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    catalog: Catalog


class RequestStep(TypedDict):
    kind: Literal["request"]
    model: str
    messages: list[Message]
    optional_params: dict[str, JsonValue]
    env: dict[str, str]


class UrlStep(TypedDict):
    kind: Literal["url"]
    api_base: str | None


class ChunkStep(TypedDict):
    kind: Literal["chunk"]
    chunk: dict[str, JsonValue]


class EmbeddingHeadersStep(TypedDict):
    kind: Literal["embedding_headers"]
    api_key: str | None
    headers: dict[str, str]
    env: dict[str, str]


class EmbeddingUrlStep(TypedDict):
    kind: Literal["embedding_url"]
    api_base: str | None


class EmbeddingOptionsStep(TypedDict):
    kind: Literal["embedding_options"]
    params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]


class EmbeddingRequestStep(TypedDict):
    kind: Literal["embedding_request"]
    model: str
    input: AllEmbeddingInputValues
    optional_params: dict[str, JsonValue]


class EmbeddingReplyStep(TypedDict):
    kind: Literal["embedding_reply"]
    body: str
    status: int
    headers: dict[str, str]


Step = (
    OptionsStep
    | RequestStep
    | UrlStep
    | ChunkStep
    | EmbeddingHeadersStep
    | EmbeddingUrlStep
    | EmbeddingOptionsStep
    | EmbeddingRequestStep
    | EmbeddingReplyStep
)


class Input(TypedDict):
    steps: list[Step]


def apply_env(env: dict[str, str]) -> None:
    os.environ.clear()
    for name, value in env.items():
        os.environ[name] = value
