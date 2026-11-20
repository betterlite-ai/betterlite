from typing import Literal, TypedDict

from pydantic import JsonValue

Provider = Literal["gemini", "vertex_ai"]


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class Params(TypedDict, total=False):
    api_key: str
    api_base: str
    model_info: dict[str, JsonValue]


class Case(TypedDict):
    mode: Literal["sync", "async"]
    provider: Provider
    model: str
    contents: JsonValue
    config: dict[str, JsonValue]
    tools: JsonValue | None
    system_instruction: JsonValue | None
    litellm_params: Params
    extra_headers: dict[str, str] | None
    extra_body: dict[str, JsonValue] | None
    timeout: float | None
    stream: bool
    reply: Reply
