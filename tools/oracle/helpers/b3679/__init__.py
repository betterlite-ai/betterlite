from typing import TypeAlias

from litellm.proxy.guardrails.guardrail_hooks.unified_guardrail.unified_guardrail import (
    UnifiedLLMGuardrails as UnifiedLLMGuardrails,
)
from pydantic import JsonValue as Json

from . import chunks, emit, flags, resolution, transform
from .chunks import PassthroughCall, PredicateCall
from .emit import EmissionCall
from .flags import RedundantScanCall, StreamingFlagCall
from .resolution import EndpointResolutionCall
from .transform import TransformChunkCall

Input: TypeAlias = (
    TransformChunkCall
    | PredicateCall
    | PassthroughCall
    | StreamingFlagCall
    | RedundantScanCall
    | EndpointResolutionCall
    | EmissionCall
)


def run(input_value: Input) -> Json:
    answer: Json = None
    match input_value["function"]:
        case "transform_chunk":
            answer = transform.run(input_value)
        case "chunk_predicates":
            answer = chunks.predicates(input_value)
        case "tool_call_passthrough":
            answer = chunks.passthrough(input_value)
        case "streaming_flag":
            answer = flags.streaming_flag(input_value)
        case "redundant_scan":
            answer = flags.redundant(input_value)
        case "endpoint_resolution":
            answer = resolution.resolve(input_value)
        case "http_error_emission":
            answer = emit.emission(input_value)
    return answer
