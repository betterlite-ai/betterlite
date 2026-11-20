from typing import Final, Literal, NotRequired, TypedDict

import litellm
from pydantic import JsonValue, TypeAdapter

from foundry_anthropic_count import run as count_run
from foundry_anthropic_messages import run as messages_run
from foundry_image_edits import run as edits_run
from foundry_image_generation import run as generation_run
from foundry_passthrough import run as passthrough_run
from foundry_passthrough.support import NonStreamCall
from foundry_rerank import run as rerank_run
from foundry_responses import DecisionsStep, UrlStep, WebsocketStep, run as responses_run
from foundry_vector_stores import run as vector_run

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True
JSON: Final = TypeAdapter(JsonValue)


class ImageInput(TypedDict):
    text: str


class Input(TypedDict):
    case: Literal[
        "image_generation", "image_edit", "rerank", "responses_decisions",
        "vector_search", "passthrough", "messages_count",
    ]
    operation: NotRequired[str]
    model: NotRequired[str]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    body: NotRequired[str]
    status: NotRequired[int]
    prompt: NotRequired[str]
    images: NotRequired[list[ImageInput | None]]
    options: NotRequired[dict[str, JsonValue]]
    model_group: NotRequired[str]
    endpoint: NotRequired[str]
    request_url: NotRequired[str]


class ModelsAnswer(TypedDict):
    models: list[str]


class EditRow(TypedDict):
    value: JsonValue


class EditAnswer(TypedDict):
    transforms: list[EditRow]


class RerankAnswer(TypedDict):
    replies: list[JsonValue]


class EndpointsAnswer(TypedDict):
    endpoints: JsonValue


class NonStreamRow(TypedDict):
    kind: str
    call_type: str | None
    standard: NotRequired[JsonValue]


class NonStreamAnswer(TypedDict):
    nonstreams: list[NonStreamRow]


class UrlsAnswer(TypedDict):
    urls: list[str]


def image_generation(values: Input) -> JsonValue:
    answer = TypeAdapter(ModelsAnswer).validate_python(
        generation_run({"models": [values.get("model", "")]})
    )
    return answer["models"][0]


def image_edit(values: Input) -> JsonValue:
    images: list[ImageInput | None] = values.get("images", [])
    options: dict[str, JsonValue] = values.get("options", {})
    answer = edits_run(
        {
            "transforms": [
                {
                    "model": values.get("model", ""),
                    "prompt": values.get("prompt", ""),
                    "images": [
                        None if field is None else {"kind": "bytes", "text": field["text"]}
                        for field in images
                    ],
                    "options": options,
                }
            ]
        }
    )
    row = TypeAdapter(EditAnswer).validate_python(answer)["transforms"][0]
    return row["value"]


def rerank(values: Input) -> JsonValue:
    answer = rerank_run(
        {
            "replies": [
                {
                    "status": values.get("status", 200),
                    "body": values.get("body", ""),
                    "model_group": values.get("model_group"),
                }
            ]
        }
    )
    return TypeAdapter(RerankAnswer).validate_python(answer)["replies"][0]


def responses_decisions(values: Input) -> JsonValue:
    step: UrlStep | DecisionsStep | WebsocketStep
    if values.get("operation") == "decisions_route":
        decisions: DecisionsStep = {
            "kind": "decisions_route",
            "model": values.get("model", ""),
            "api_base": values.get("api_base", ""),
            "api_key": values.get("api_key", ""),
        }
        step = decisions
    elif values.get("operation") == "no_native_websocket":
        step = {"kind": "no_native_websocket"}
    else:
        step = {"kind": "url", "api_base": values.get("api_base", "")}
    return TypeAdapter(list[JsonValue]).validate_python(responses_run({"steps": [step]}))[0]


def vector_search(values: Input) -> JsonValue:
    del values
    answer = vector_run({"store_type_endpoints": {}})
    return TypeAdapter(EndpointsAnswer).validate_python(answer)["endpoints"]


def passthrough(values: Input) -> JsonValue:
    call: NonStreamCall = {
        "status": values.get("status", 200),
        "body": values.get("body", ""),
        "model": values.get("model", ""),
        "endpoint": values.get("endpoint", ""),
        "request_url": values.get("request_url"),
    }
    answer = passthrough_run({"nonstreams": [call]})
    row = TypeAdapter(NonStreamAnswer).validate_python(answer)["nonstreams"][0]
    return JSON.validate_python(
        {
            "kind": row["kind"],
            "call_type": row["call_type"],
            "standard": row.get("standard"),
        }
    )


def messages_count(values: Input) -> JsonValue:
    base = values.get("api_base", "")
    messages = TypeAdapter(UrlsAnswer).validate_python(
        messages_run({"function": "url", "api_bases": [base], "environment": {}})
    )["urls"][0]
    count = TypeAdapter(UrlsAnswer).validate_python(
        count_run({"function": "endpoint", "api_bases": [base]})
    )["urls"][0]
    return JSON.validate_python({"messages": messages, "count": count})


def run(input_value: Input) -> JsonValue:
    match input_value["case"]:
        case "image_generation":
            return image_generation(input_value)
        case "image_edit":
            return image_edit(input_value)
        case "rerank":
            return rerank(input_value)
        case "responses_decisions":
            return responses_decisions(input_value)
        case "vector_search":
            return vector_search(input_value)
        case "passthrough":
            return passthrough(input_value)
        case "messages_count":
            return messages_count(input_value)
