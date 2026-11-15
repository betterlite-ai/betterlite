from typing import Final, TypedDict

from litellm.llms.vertex_ai.agent_engine.sse_iterator import VertexAgentEngineResponseIterator
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter


class DeltaView(TypedDict):
    content: str | None
    role: str | None


class ChoiceView(TypedDict):
    finish_reason: str | None
    delta: DeltaView


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class StreamView(TypedDict):
    choices: list[ChoiceView]
    usage: UsageView | None


class Input(TypedDict):
    chunks: list[dict[str, JsonValue]]


ITERATOR: Final = VertexAgentEngineResponseIterator(
    streaming_response=iter(()), sync_stream=True
)
# chunk_parser is typed as a union, and every Agent Engine line answers the stream kind
MODEL: Final = TypeAdapter(ModelResponseStream)
STREAM: Final = TypeAdapter(StreamView)


def chunk_outcome(chunk: dict[str, JsonValue]) -> JsonValue:
    parsed = MODEL.validate_python(ITERATOR.chunk_parser(chunk))
    view = STREAM.validate_json(parsed.model_dump_json())
    choice = view["choices"][0]
    usage = view["usage"]
    return {
        "content": choice["delta"]["content"],
        "role": choice["delta"]["role"],
        "finish_reason": choice["finish_reason"],
        "usage": None
        if usage is None
        else [usage["prompt_tokens"], usage["completion_tokens"], usage["total_tokens"]],
    }


def run(input_value: Input) -> JsonValue:
    return {"chunks": [chunk_outcome(chunk) for chunk in input_value["chunks"]]}
