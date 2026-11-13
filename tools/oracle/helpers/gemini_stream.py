from dataclasses import dataclass, field
from typing import Final, TypedDict, cast

from litellm.llms.vertex_ai.gemini import vertex_and_google_ai_studio_gemini as gemini
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter


class Input(TypedDict):
    fragments: list[str]
    legacy_function: bool
    headers: dict[str, str]


@dataclass
class Logging:
    optional_params: dict[str, JsonValue]


@dataclass
class FunctionView:
    name: str
    arguments: str


@dataclass
class ToolCallView:
    id: str
    type: str
    index: int
    function: FunctionView
    provider_specific_fields: JsonValue = None


@dataclass
class DeltaView:
    role: str | None = None
    content: str | None = None
    reasoning_content: str | None = None
    tool_calls: list[ToolCallView] | None = None
    function_call: FunctionView | None = None
    images: JsonValue = None
    annotations: JsonValue = None
    provider_specific_fields: JsonValue = None


@dataclass
class ChoiceView:
    index: int
    finish_reason: str | None
    delta: DeltaView
    logprobs: JsonValue = None


@dataclass
class PromptDetailsView:
    web_search_requests: int | None = None


@dataclass
class UsageView:
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: PromptDetailsView | None = None
    completion_tokens_details: JsonValue = None


@dataclass
class ChunkView:
    id: str
    model: str | None
    choices: list[ChoiceView]
    usage: UsageView | None = None
    service_tier: str | None = None
    vertex_ai_grounding_metadata: list[JsonValue] = field(default_factory=list)
    vertex_ai_url_context_metadata: list[JsonValue] = field(default_factory=list)
    vertex_ai_safety_ratings: list[JsonValue] = field(default_factory=list)
    vertex_ai_safety_results: list[JsonValue] = field(default_factory=list)
    vertex_ai_citation_metadata: list[JsonValue] = field(default_factory=list)


CHUNK: Final = TypeAdapter(ChunkView)


def _usage(usage: UsageView) -> JsonValue:
    details = usage.prompt_tokens_details
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_details": details is not None,
        "completion_details": usage.completion_tokens_details is not None,
        "web_search_requests": None if details is None else details.web_search_requests,
    }


def _tool_call(call: ToolCallView) -> JsonValue:
    return {
        "id": call.id,
        "type": call.type,
        "index": call.index,
        "function": {"name": call.function.name, "arguments": call.function.arguments},
        "provider_specific_fields": call.provider_specific_fields,
    }


def _tool_calls(calls: list[ToolCallView] | None) -> JsonValue:
    if calls is None:
        return None
    return [_tool_call(call) for call in calls]


def _function_call(function: FunctionView | None) -> JsonValue:
    if function is None:
        return None
    return {"name": function.name, "arguments": function.arguments}


def _delta(delta: DeltaView) -> JsonValue:
    return {
        "role": delta.role,
        "content": delta.content,
        "reasoning_content": delta.reasoning_content,
        "tool_calls": _tool_calls(delta.tool_calls),
        "function_call": _function_call(delta.function_call),
        "images": delta.images,
        "annotations": delta.annotations,
        "provider_specific_fields": delta.provider_specific_fields,
    }


def _choice(choice: ChoiceView) -> JsonValue:
    return {
        "index": choice.index,
        "finish_reason": choice.finish_reason,
        "delta": _delta(choice.delta),
        "logprobs": choice.logprobs,
    }


def _metadata(chunk: ChunkView) -> JsonValue:
    return {
        "grounding_metadata": chunk.vertex_ai_grounding_metadata,
        "url_context_metadata": chunk.vertex_ai_url_context_metadata,
        "safety_ratings": chunk.vertex_ai_safety_ratings,
        "safety_results": chunk.vertex_ai_safety_results,
        "citation_metadata": chunk.vertex_ai_citation_metadata,
    }


def _state(chunk: ModelResponseStream) -> JsonValue:
    view = CHUNK.validate_json(chunk.model_dump_json())
    return {
        "id": view.id,
        "model": view.model,
        "choices": [_choice(choice) for choice in view.choices],
        "usage": None if view.usage is None else _usage(view.usage),
        "service_tier": view.service_tier,
        "metadata": _metadata(view),
    }


def _chunks(
    input_value: Input, optional_params: dict[str, JsonValue]
) -> tuple[list[JsonValue], list[JsonValue]]:
    iterator = iter(
        gemini.ModelResponseIterator(
            streaming_response=iter(input_value["fragments"]),
            sync_stream=True,
            logging_obj=cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L109
                "gemini.LoggingClass", Logging(optional_params=optional_params)
            ),
            response_headers=input_value["headers"],
            response=None,
        )
    )
    chunks: list[JsonValue] = []
    for _ in range(len(input_value["fragments"])):
        chunk = next(iterator)
        chunks.append(None if chunk is None else _state(chunk))
    drained: list[JsonValue] = []
    while True:
        try:
            extra = next(iterator)
        except StopIteration:
            break
        drained.append(None if extra is None else _state(extra))
    return chunks, drained


def run(input_value: Input) -> JsonValue:
    optional_params: dict[str, JsonValue] = (
        {"functions": [{}]} if input_value["legacy_function"] else {}
    )
    chunks, drained = _chunks(input_value, optional_params)
    return {"chunks": chunks, "drained": drained}
