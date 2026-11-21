from typing import Literal, TypedDict

from pydantic import JsonValue


class Globals(TypedDict):
    api_base: str | None
    api_key: str | None
    headers: dict[str, str] | None


class Configured(TypedDict):
    api_base: str | None
    api_key: str | None
    globals: Globals
    environment: dict[str, str]


class RewriteInput(TypedDict):
    op: Literal["rewrite"]
    base: str


class EmbeddingInput(Configured):
    op: Literal["embedding"]


class DispatchInput(Configured):
    op: Literal["dispatch"]
    model: str
    messages: list[dict[str, JsonValue]]
    headers: dict[str, str] | None
    stream: bool
    status: int | None


Input = RewriteInput | EmbeddingInput | DispatchInput


class PostCall(TypedDict):
    error: bool
    headers: bool


class Observation(TypedDict):
    branch: str | None
    base: str | None
    key: str | None
    headers: dict[str, str] | None
    messages: JsonValue
    post_calls: list[PostCall]
    errors: bool
