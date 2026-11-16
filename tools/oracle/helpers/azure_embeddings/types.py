from typing import Literal, NotRequired, TypeAlias, TypedDict

from openai.types import CreateEmbeddingResponse
from pydantic import JsonValue as Json

EmbeddingInput: TypeAlias = list[str] | list[int] | list[list[int]]
ClientName: TypeAlias = Literal[
    "azure_open_ai", "async_azure_open_ai", "open_ai", "async_open_ai"
]


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class ReplyAnswer(TypedDict):
    reply: Reply


class ConnectionAnswer(TypedDict):
    connection: dict[str, str]


class LiteLlmParams(TypedDict, total=False):
    max_retries: int
    timeout: float


class Step(TypedDict):
    input: EmbeddingInput
    answer: ReplyAnswer | ConnectionAnswer
    client: ClientName
    model: str
    api_base: str
    api_version: str
    timeout: float
    api_key: str
    aembedding: NotRequired[bool]
    azure_ad_token: NotRequired[str]
    max_retries: NotRequired[int]
    optional_params: NotRequired[dict[str, Json]]
    headers: NotRequired[dict[str, str]]
    litellm_params: NotRequired[LiteLlmParams]


class Input(TypedDict):
    steps: list[Step]


class PreArgs(TypedDict):
    complete_input_dict: dict[str, Json]
    headers: dict[str, str | None]


class PostArgs(TypedDict):
    complete_input_dict: dict[str, Json]
    api_base: NotRequired[str]


class PreSnapshot(TypedDict):
    log_event_type: Literal["pre_api_call"]
    input: Json
    api_key: str | None
    additional_args: PreArgs


class PostSnapshot(TypedDict):
    log_event_type: Literal["post_api_call"]
    input: Json
    api_key: str | None
    additional_args: PostArgs
    original_response: CreateEmbeddingResponse | str


Snapshot: TypeAlias = PreSnapshot | PostSnapshot


class ErrorFields(TypedDict):
    status_code: int
    message: str
    headers: NotRequired[dict[str, str] | None]


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ReplyView(TypedDict):
    model: str | None
    object: str
    data: list[Json]
    usage: UsageView


class ResponseView(TypedDict):
    model: str
    object: str
    data: list[Json]


class Hidden(TypedDict):
    additional_headers: dict[str, str]
    headers: NotRequired[dict[str, str]]


class Private(TypedDict):
    _hidden_params: Hidden
    _response_headers: dict[str, str]
