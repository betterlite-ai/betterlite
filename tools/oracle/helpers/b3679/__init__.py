from typing import Literal, TypeAlias, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.unified_guardrail.unified_guardrail import (
    resolve_endpoint_translation,
)
from pydantic import JsonValue as Json

from .chunks import (
    PassthroughCall,
    PredicateCall,
    TransformCall,
    passthrough,
    predicates,
    stream_item,
    transform_chunk,
)
from .emit import (
    A2aErrorCall,
    A2aRequestIdCall,
    EmissionCall,
    emission,
    error_chunk,
    request_id,
)
from .flags import FlagCall, RedundantScanCall, redundant, streaming_flag


class EntryCall(TypedDict):
    call_type: str
    translation: str
    chat_completions: bool


class EndpointResolutionCall(TypedDict):
    function: Literal["endpoint_resolution"]
    request_route: str | None
    first_item: Json | None
    entries: list[EntryCall]


Input: TypeAlias = (
    TransformCall
    | PredicateCall
    | PassthroughCall
    | FlagCall
    | RedundantScanCall
    | EndpointResolutionCall
    | EmissionCall
    | A2aRequestIdCall
    | A2aErrorCall
)


def resolve(call: EndpointResolutionCall) -> Json:
    auth = UserAPIKeyAuth(request_route=call["request_route"])
    first = None if call["first_item"] is None else stream_item(call["first_item"])
    resolved = resolve_endpoint_translation(auth, first)
    if resolved is None:
        return None
    call_type, handler = resolved
    return {"call_type": call_type, "translation": type(handler).__name__}


def run(input_value: Input) -> Json:
    answer: Json = None
    match input_value["function"]:
        case "transform_chunk":
            answer = transform_chunk(input_value)
        case "chunk_predicates":
            answer = predicates(input_value)
        case "tool_call_passthrough":
            answer = passthrough(input_value)
        case "streaming_flag":
            answer = streaming_flag(input_value)
        case "redundant_scan":
            answer = redundant(input_value)
        case "endpoint_resolution":
            answer = resolve(input_value)
        case "http_error_emission":
            answer = emission(input_value)
        case "a2a_request_id":
            answer = request_id(input_value)
        case "a2a_error_chunk":
            answer = error_chunk(input_value)
    return answer
