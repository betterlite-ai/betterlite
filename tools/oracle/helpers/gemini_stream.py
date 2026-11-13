from dataclasses import dataclass
from typing import Final, Protocol, TypedDict, cast

from litellm.llms.vertex_ai.gemini import vertex_and_google_ai_studio_gemini as gemini
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter


class StreamIterator(Protocol):
    def __iter__(self) -> "StreamIterator": ...
    def __next__(self) -> ModelResponseStream | None: ...


class Input(TypedDict):
    fragments: list[str]
    legacy_function: bool
    headers: dict[str, str]
@dataclass
class Logging:
    optional_params: dict[str, JsonValue]
@dataclass
class DeltaView:
    role: str | None = None
    content: str | None = None
    reasoning_content: str | None = None
    tool_calls: JsonValue = None
    function_call: JsonValue = None
    images: JsonValue = None
    annotations: JsonValue = None
    provider_specific_fields: JsonValue = None
@dataclass
class ChoiceView:
    index: int
    finish_reason: str | None
    delta: DeltaView
    logprobs: JsonValue = None
class PromptDetails(TypedDict, total=False):
    web_search_requests: int
@dataclass
class UsageView:
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: PromptDetails | None = None
    completion_tokens_details: JsonValue = None
@dataclass
class ChunkView:
    id: str
    model: str | None
    choices: list[ChoiceView]
    usage: UsageView | None = None
    service_tier: str | None = None
CHUNK: Final = TypeAdapter(ChunkView)
DELTA: Final = TypeAdapter(DeltaView)
JSON_VALUE: Final = TypeAdapter(JsonValue)
def _usage(usage: UsageView) -> JsonValue:
    details = usage.prompt_tokens_details
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_details": details is not None,
        "completion_details": usage.completion_tokens_details is not None,
        "web_search_requests": None if details is None else details.get("web_search_requests"),
    }
def _choice(choice: ChoiceView) -> JsonValue:
    return {
        "index": choice.index,
        "finish_reason": choice.finish_reason,
        "delta": JSON_VALUE.validate_json(DELTA.dump_json(choice.delta)),
        "logprobs": choice.logprobs,
    }
def _state(chunk: ModelResponseStream) -> JsonValue:
    view = CHUNK.validate_json(chunk.model_dump_json())
    return {
        "id": view.id,
        "model": view.model,
        "choices": [_choice(choice) for choice in view.choices],
        "usage": None if view.usage is None else _usage(view.usage),
        "service_tier": view.service_tier,
    }
def _chunks(
    input_value: Input, optional_params: dict[str, JsonValue]
) -> tuple[list[JsonValue], list[JsonValue]]:
    iterator: StreamIterator = gemini.ModelResponseIterator(
        streaming_response=iter(input_value["fragments"]),
        sync_stream=True,
        logging_obj=cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L109
            "gemini.LoggingClass", Logging(optional_params=optional_params)
        ),
        response_headers=input_value["headers"],
        response=None,
    )
    running = iter(iterator)
    chunks: list[JsonValue] = []
    for _ in range(len(input_value["fragments"])):
        chunk = next(running)
        chunks.append(None if chunk is None else _state(chunk))
    drained: list[JsonValue] = []
    while True:
        try:
            extra = next(running)
        except StopIteration:
            break
        drained.append(None if extra is None else _state(extra))
    return chunks, drained
def _optional_params(input_value: Input) -> dict[str, JsonValue]:
    if input_value["legacy_function"]:
        return {"functions": [{}]}
    return {}
def run(input_value: Input) -> JsonValue:
    chunks, drained = _chunks(input_value, _optional_params(input_value))
    return {"chunks": chunks, "drained": drained}
