import json

from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.types.utils import EmbeddingResponse
from openai.types import CreateEmbeddingResponse
from pydantic import JsonValue as Json
from pydantic import TypeAdapter, ValidationError

from azure_embeddings.types import (
    ErrorFields,
    PostSnapshot,
    Private,
    ReplyView,
    ResponseView,
    Snapshot,
)

DECODE_PREFIX = "Failed to parse raw Azure embedding response:"
DECODE_MARKER = "<json decode failure>"
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
            "api_base": arguments["api_base"] if "api_base" in arguments else None,
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
    return {
        "model": view["model"],
        "object": view["object"],
        "data": view["data"],
        "usage": {
            "prompt_tokens": view["usage"]["prompt_tokens"],
            "completion_tokens": view["usage"]["completion_tokens"],
            "total_tokens": view["usage"]["total_tokens"],
        },
        "response_headers": private["_response_headers"],
        "additional_headers": hidden["additional_headers"],
        "hidden_headers": hidden["headers"] if "headers" in hidden else None,
    }


def azure_value(error: AzureOpenAIError) -> dict[str, Json]:
    fields = ERROR_FIELDS.validate_python(error, from_attributes=True)
    message = fields["message"]
    match error.__context__:
        case json.JSONDecodeError():
            message = DECODE_PREFIX if message.startswith(DECODE_PREFIX) else DECODE_MARKER
        case BaseException() | None:
            pass
    return {
        "kind": "azure",
        "status": fields["status_code"],
        "message": message,
        "headers": fields["headers"] if "headers" in fields else None,
    }
