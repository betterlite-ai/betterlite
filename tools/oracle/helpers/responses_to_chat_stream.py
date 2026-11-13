from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.completion_extras.litellm_responses_transformation.transformation import (
    OpenAiResponsesToChatCompletionStreamIterator,
)
from litellm.types.utils import ModelResponseStream
from pydantic import TypeAdapter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class EventCall(TypedDict):
    mode: Literal["event"]
    events: list[dict[str, Json]]


class LineCall(TypedDict):
    mode: Literal["line"]
    lines: list[str]


class StatelessCall(TypedDict):
    mode: Literal["stateless"]
    events: list[dict[str, Json]]
    tool_call_index_map: NotRequired[dict[str, int] | None]


Input: TypeAlias = EventCall | LineCall | StatelessCall

DUMP = TypeAdapter(dict[str, Json])
MODEL = TypeAdapter(ModelResponseStream)


class LineReader(OpenAiResponsesToChatCompletionStreamIterator):
    def read(self, line: str) -> ModelResponseStream:
        return MODEL.validate_python(self._handle_string_chunk(line))


def dump(chunk: ModelResponseStream) -> Json:
    return DUMP.validate_json(chunk.model_dump_json())


def run(input_value: Input) -> Json:
    if input_value["mode"] == "line":
        return _lines(input_value)
    if input_value["mode"] == "stateless":
        return _stateless(input_value)
    return _events(input_value)


def _iterator() -> OpenAiResponsesToChatCompletionStreamIterator:
    return OpenAiResponsesToChatCompletionStreamIterator(iter([]), sync_stream=True)


def _events(call: EventCall) -> Json:
    iterator = _iterator()
    return {"steps": [dump(iterator.chunk_parser(event)) for event in call["events"]]}


def _lines(call: LineCall) -> Json:
    reader = LineReader(iter([]), sync_stream=True)
    return {"steps": [{"kind": "chunk", "chunk": dump(reader.read(line))} for line in call["lines"]]}


def _stateless(call: StatelessCall) -> Json:
    shared = call.get("tool_call_index_map")
    indices = None if shared is None else {int(key): value for key, value in shared.items()}
    return {
        "steps": [
            dump(
                OpenAiResponsesToChatCompletionStreamIterator.translate_responses_chunk_to_openai_stream(
                    event, indices
                )
            )
            for event in call["events"]
        ]
    }
