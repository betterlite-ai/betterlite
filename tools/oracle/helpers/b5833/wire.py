from typing import Literal, Required, TypedDict

from litellm.types.llms.openai import AllMessageValues
from pydantic import JsonValue

Answer = Literal["yes", "no", "fail"]
Object = dict[str, JsonValue]

MODEL = "groq/llama-3.3-70b-versatile"
STT_MODEL = "whisper-large-v3"


class Messages(list[AllMessageValues]):
    pass


class Capabilities(TypedDict, total=False):
    web_search: Answer
    reasoning: Answer
    response_schema: Answer


class CredentialCall(TypedDict, total=False):
    api_base: str
    api_key: str
    environment_base: str
    environment_key: str


class SupportedCall(TypedDict):
    model: str


class MessageCall(TypedDict):
    model: str
    messages: Messages


class OptionCall(TypedDict, total=False):
    model: Required[str]
    non_default: Object
    optional_params: Object


class ReplyCall(TypedDict, total=False):
    reply: Required[Object]
    json_mode: bool


class ChunkCall(TypedDict, total=False):
    chunk: Required[Object]


class SttCall(TypedDict, total=False):
    non_default: Object
    optional_params: Object
    drop_params: bool
    global_drop_params: bool


class CostCall(TypedDict, total=False):
    model: Required[str]
    web_search_requests: int | None
    browser_open_requests: int | None
    medium_rate: str


class Input(TypedDict, total=False):
    capabilities: Capabilities
    credentials: list[CredentialCall]
    supported: list[SupportedCall]
    messages: list[MessageCall]
    options: list[OptionCall]
    replies: list[ReplyCall]
    chunks: list[ChunkCall]
    stt: list[SttCall]
    costs: list[CostCall]
