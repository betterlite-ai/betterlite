from typing import Literal, TypeAlias, TypedDict

from litellm.completion_extras.litellm_responses_transformation.transformation import (
    OpenAiResponsesToChatCompletionStreamIterator as ResponsesStream,
)
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


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
EVENT = TypeAdapter(dict[str, object])


class LineReader(ResponsesStream):
    def __init__(self) -> None:
        super().__init__(iter([]), sync_stream=True)
        self.chunks: list[ModelResponseStream] = []

    @override
    def chunk_parser(
        self,
        # ast-grep-ignore: no-loose-types
        chunk: dict[str, object],
    ) -> ModelResponseStream:
        parsed = super().chunk_parser(chunk)
        self.chunks.append(parsed)
        return parsed

    def read(self, line: str) -> ModelResponseStream:
        self._handle_string_chunk(line)
        return self.chunks.pop()


def run(input_value: Input) -> JsonValue:
    if input_value["mode"] == "line":
        reader = LineReader()
        steps: list[JsonValue] = [
            DUMP.validate_json(reader.read(line).model_dump_json(exclude_unset=True))
            for line in input_value["lines"]
        ]
    else:
        iterator = ResponsesStream(iter([]), sync_stream=True)
        steps = [
            DUMP.validate_json(
                iterator.chunk_parser(EVENT.validate_python(event)).model_dump_json(
                    exclude_unset=True
                )
            )
            for event in input_value["events"]
        ]
    result: dict[str, JsonValue] = {"steps": steps}
    return result
