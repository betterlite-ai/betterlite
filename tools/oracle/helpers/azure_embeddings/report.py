import json
from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.types.utils import EmbeddingResponse
from openai.types import CreateEmbeddingResponse
from pydantic import JsonValue as Json
from pydantic import TypeAdapter, ValidationError

EmbeddingInput: TypeAlias = list[str] | list[int] | list[list[int]]
ClientName: TypeAlias = Literal["azure_open_ai", "async_azure_open_ai", "open_ai", "async_open_ai"]
DECODE_PREFIX = "Failed to parse raw Azure embedding response:"
DECODE_MARKER = "<json decode failure>"


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
    aembedding: bool
    model: str
    api_base: str
    api_version: str
    timeout: float
    api_key: str
    azure_ad_token: str | None
    max_retries: int | None
    optional_params: dict[str, Json]
    headers: dict[str, str]
    litellm_params: LiteLlmParams


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
    headers: NotRequired[dict[str, Json] | None]


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
    additional_headers: dict[str, Json]
    headers: NotRequired[dict[str, Json]]


class Private(TypedDict):
    _hidden_params: Hidden
    _response_headers: dict[str, Json]


ERROR_FIELDS = TypeAdapter(ErrorFields)
RESPONSE_VIEW = TypeAdapter(ResponseView)
SNAPSHOT = TypeAdapter(Snapshot)


def view_value(view: ResponseView) -> dict[str, Json]:
    return {
        "model": view["model"],
        "object": view["object"],
        "data": view["data"],
    }


class Recorder:
    def __init__(self) -> None:
        self.pre: list[Json] = []
        self.post: list[Json] = []

    def post_value(self, snapshot: PostSnapshot) -> dict[str, Json]:
        arguments = snapshot["additional_args"]
        original = snapshot["original_response"]
        kind = "error_text"
        value: dict[str, Json] | None = None
        api_base: str | None = None
        if "api_base" in arguments:
            api_base = arguments["api_base"]
        match original:
            case CreateEmbeddingResponse():
                kind = "sdk_object"
                value = view_value(RESPONSE_VIEW.validate_json(original.model_dump_json()))
            case str():
                try:
                    value = view_value(RESPONSE_VIEW.validate_json(original))
                    kind = "dumped"
                except ValidationError:
                    value = None
        return {
            "input": snapshot["input"],
            "api_key": snapshot["api_key"],
            "kind": kind,
            "original_response": value,
            "complete_input_dict": arguments["complete_input_dict"],
            "api_base": api_base,
        }

    def log(self, details: dict[str, Json]) -> None:
        snapshot = SNAPSHOT.validate_python(details)
        if snapshot["log_event_type"] == "pre_api_call":
            arguments = snapshot["additional_args"]
            self.pre.append(
                {
                    "input": snapshot["input"],
                    "api_key": snapshot["api_key"],
                    "azure_ad_token": arguments["headers"]["azure_ad_token"],
                    "complete_input_dict": arguments["complete_input_dict"],
                }
            )
        else:
            self.post.append(self.post_value(snapshot))


def reply_value(response: EmbeddingResponse) -> dict[str, Json]:
    view = TypeAdapter(ReplyView).validate_json(response.model_dump_json())
    private = TypeAdapter(Private).validate_python(response.__pydantic_private__)
    hidden = private["_hidden_params"]
    hidden_headers: dict[str, Json] | None = None
    if "headers" in hidden:
        hidden_headers = hidden["headers"]
    usage = view["usage"]
    return {
        "model": view["model"],
        "object": view["object"],
        "data": view["data"],
        "usage": {
            "prompt_tokens": usage["prompt_tokens"],
            "completion_tokens": usage["completion_tokens"],
            "total_tokens": usage["total_tokens"],
        },
        "response_headers": private["_response_headers"],
        "additional_headers": hidden["additional_headers"],
        "hidden_headers": hidden_headers,
    }


def azure_value(error: AzureOpenAIError) -> dict[str, Json]:
    fields = ERROR_FIELDS.validate_python(error, from_attributes=True)
    value: dict[str, Json] = {"kind": "azure", "status": fields["status_code"], "headers": None}
    if "headers" in fields:
        value["headers"] = fields["headers"]
    message = fields["message"]
    match error.__context__:
        case json.JSONDecodeError():
            message = DECODE_PREFIX if message.startswith(DECODE_PREFIX) else DECODE_MARKER
        case BaseException() | None:
            pass
    value["message"] = message
    return value
