from typing import Literal, Required, TypedDict

import litellm
from pydantic import JsonValue

litellm.suppress_debug_info = True

Answer = Literal["yes", "no", "fail"]
Object = dict[str, JsonValue]

MODEL = "groq/llama-3.3-70b-versatile"
STT_MODEL = "whisper-large-v3"


class Capabilities(TypedDict, total=False):
    web_search: Answer
    reasoning: Answer
    response_schema: Answer


class Call(TypedDict, total=False):
    model: Required[str]
    messages: Required[list[Object]]
    reply: Required[Object]
    chunk: Required[Object]
    non_default: Object
    optional_params: Object
    drop_params: bool
    global_drop_params: bool
    json_mode: bool
    api_base: str
    api_key: str
    environment_base: str
    environment_key: str
    web_search_requests: int | None
    browser_open_requests: int | None
    medium_rate: str


class Input(TypedDict, total=False):
    capabilities: Capabilities
    credentials: list[Call]
    supported: list[Call]
    messages: list[Call]
    options: list[Call]
    replies: list[Call]
    chunks: list[Call]
    stt: list[Call]
    costs: list[Call]
