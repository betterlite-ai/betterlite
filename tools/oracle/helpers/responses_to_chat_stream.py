from typing import Literal, TypeAlias, TypedDict

from litellm.completion_extras.litellm_responses_transformation.transformation import (
    OpenAiResponsesToChatCompletionStreamIterator as ResponsesStream,
)
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter


class StreamEvent(TypedDict, total=False):
    type: str


class EventCall(TypedDict):
    mode: Literal["event"]
    events: list[StreamEvent]


class LineCall(TypedDict):
    mode: Literal["line"]
    lines: list[str]


Input: TypeAlias = EventCall | LineCall

DUMP = TypeAdapter(dict[str, JsonValue])
MODEL = TypeAdapter(ModelResponseStream)
EVENT = TypeAdapter(dict[str, object])


class LineReader(ResponsesStream):
    def read(self, line: str) -> ModelResponseStream:
        return MODEL.validate_python(self._handle_string_chunk(line))


def run(input_value: Input) -> JsonValue:
    if input_value["mode"] == "line":
        return _lines(input_value)
    return _events(input_value)


def _events(call: EventCall) -> JsonValue:
    iterator = ResponsesStream(iter([]), sync_stream=True)
    steps: list[JsonValue] = [
        DUMP.validate_json(iterator.chunk_parser(EVENT.validate_python(event)).model_dump_json())
        for event in call["events"]
    ]
    result: dict[str, JsonValue] = {"steps": steps}
    return result


def _lines(call: LineCall) -> JsonValue:
    reader = LineReader(iter([]), sync_stream=True)
    steps: list[JsonValue] = [
        {"chunk": DUMP.validate_json(reader.read(line).model_dump_json())} for line in call["lines"]
    ]
    result: dict[str, JsonValue] = {"steps": steps}
    return result
