from collections.abc import Callable
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from pydantic import JsonValue, TypeAdapter

from foundry_anthropic_count import run as count_run
from foundry_anthropic_messages import run as messages_run
from foundry_image_edits import run as edits_run
from foundry_image_generation import run as generation_run
from foundry_passthrough import run as passthrough_run
from foundry_passthrough.support import NonStreamCall
from foundry_rerank import ReplyCall
from foundry_rerank import run as rerank_run
from foundry_responses import DecisionsStep, UrlStep, WebsocketStep
from foundry_responses import run as responses_run
from foundry_vector_stores import run as vector_run

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True


class Input(TypedDict):
    case: str


class EditCase(TypedDict):
    case: Literal["image_edit"]
    model: str
    prompt: str
    images: list[dict[str, str] | None]
    options: dict[str, JsonValue]


class UrlCase(TypedDict):
    case: Literal["responses_decisions"]
    operation: Literal["url"]
    api_base: str


class DecisionsCase(TypedDict):
    case: Literal["responses_decisions"]
    operation: Literal["decisions_route"]
    model: str
    api_base: str
    api_key: str


class PolicyCase(TypedDict):
    case: Literal["responses_decisions"]
    operation: Literal["no_native_websocket"]


ResponsesCase: TypeAlias = UrlCase | DecisionsCase | PolicyCase


class EditRow(TypedDict):
    value: JsonValue


class EditAnswer(TypedDict):
    transforms: list[EditRow]


class RerankAnswer(TypedDict):
    replies: list[JsonValue]


class NonStreamRow(TypedDict):
    kind: str
    call_type: str | None
    standard: NotRequired[JsonValue]


class NonStreamAnswer(TypedDict):
    nonstreams: list[NonStreamRow]


FIELDS: Final = TypeAdapter(dict[str, str])
STRINGS: Final = TypeAdapter(dict[str, list[str]])
EDIT: Final = TypeAdapter(EditCase)
REPLY: Final = TypeAdapter(ReplyCall)
RESPONSES: Final = TypeAdapter(ResponsesCase)
NONSTREAM: Final = TypeAdapter(NonStreamCall)
EDITS: Final = TypeAdapter(EditAnswer)
REPLIES: Final = TypeAdapter(RerankAnswer)
NONSTREAMS: Final = TypeAdapter(NonStreamAnswer)


def image_generation(values: Input) -> JsonValue:
    model = FIELDS.validate_python(values)["model"]
    answer = STRINGS.validate_python(generation_run({"models": [model]}))
    return answer["models"][0]


def image_edit(values: Input) -> JsonValue:
    case = EDIT.validate_python(values)
    answer = EDITS.validate_python(
        edits_run(
            {
                "transforms": [
                    {
                        "model": case["model"],
                        "prompt": case["prompt"],
                        "images": [
                            None if field is None else {"kind": "bytes", "text": field["text"]}
                            for field in case["images"]
                        ],
                        "options": case["options"],
                    }
                ]
            }
        )
    )
    return answer["transforms"][0]["value"]


def rerank(values: Input) -> JsonValue:
    reply = REPLY.validate_python(values)
    return REPLIES.validate_python(rerank_run({"replies": [reply]}))["replies"][0]


def step_of(case: ResponsesCase) -> UrlStep | DecisionsStep | WebsocketStep:
    if case["operation"] == "decisions_route":
        return {
            "kind": "decisions_route",
            "model": case["model"],
            "api_base": case["api_base"],
            "api_key": case["api_key"],
        }
    if case["operation"] == "no_native_websocket":
        return {"kind": "no_native_websocket"}
    return {"kind": "url", "api_base": case["api_base"]}


def responses_decisions(values: Input) -> JsonValue:
    step = step_of(RESPONSES.validate_python(values))
    answers = TypeAdapter(list[JsonValue]).validate_python(responses_run({"steps": [step]}))
    return answers[0]


def vector_search(values: Input) -> JsonValue:
    del values
    answer = vector_run({"store_type_endpoints": {}})
    return TypeAdapter(dict[str, JsonValue]).validate_python(answer)["endpoints"]


def passthrough(values: Input) -> JsonValue:
    call = NONSTREAM.validate_python(values)
    answer = passthrough_run({"nonstreams": [call]})
    row = NONSTREAMS.validate_python(answer)["nonstreams"][0]
    return {"kind": row["kind"], "call_type": row["call_type"], "standard": row.get("standard")}


def messages_count(values: Input) -> JsonValue:
    base = FIELDS.validate_python(values)["api_base"]
    message = STRINGS.validate_python(
        messages_run({"function": "url", "api_bases": [base], "environment": {}})
    )
    count = STRINGS.validate_python(count_run({"function": "endpoint", "api_bases": [base]}))
    return {"messages": message["urls"][0], "count": count["urls"][0]}


Handler: TypeAlias = Callable[[Input], JsonValue]

HANDLERS: Final[dict[str, Handler]] = {
    "image_generation": image_generation,
    "image_edit": image_edit,
    "rerank": rerank,
    "responses_decisions": responses_decisions,
    "vector_search": vector_search,
    "passthrough": passthrough,
    "messages_count": messages_count,
}


def run(input_value: Input) -> JsonValue:
    handler = HANDLERS[input_value["case"]]
    return handler(input_value)
