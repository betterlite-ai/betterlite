from collections.abc import AsyncIterator, Iterator, Mapping
from typing import Literal, NotRequired, Protocol, TypedDict

from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import JsonValue, TypeAdapter

Provider = Literal["anthropic", "deepseek_r1", "moonshot"]
Construction = Literal["legacy", "anthropic", "deepseek_r1", "moonshot", "base"]
Step = Literal["next", "anext"]

STREAM: TypeAdapter[ModelResponseStream] = TypeAdapter(ModelResponseStream)
RESPONSE: TypeAdapter[ModelResponse] = TypeAdapter(ModelResponse)


class FramedInput(TypedDict):
    construction: Construction
    provider: Provider | None
    sync: bool
    model: str
    json_mode: bool | None
    chunks: list[str]
    headers: dict[str, str] | None


class FakeInput(TypedDict):
    response: dict[str, JsonValue]
    json_mode: bool | None
    steps: list[Step]


class Input(TypedDict):
    framed: list[FramedInput]
    fake: list[FakeInput]


class FailureView(TypedDict):
    status: int | None


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class GenericView(TypedDict):
    kind: Literal["generic"]
    text: str
    tool_use: JsonValue
    is_finished: bool
    finish_reason: str
    usage: UsageView | None
    index: int | None


class StreamView(TypedDict):
    kind: Literal["stream"]
    content: str | None
    reasoning_content: str | None
    finish_reason: str | None
    tool_calls: list[JsonValue] | None
    usage: UsageView | None


class FramedResult(TypedDict):
    chunks: list[GenericView | StreamView]
    failure: FailureView | None


class OkStep(TypedDict):
    ok: GenericView | StreamView


class FailureStep(TypedDict):
    failure: FailureView


class DoneStep(TypedDict):
    done: bool


class FakeResult(TypedDict):
    steps: list[OkStep | FailureStep | DoneStep]


class Output(TypedDict):
    framed: list[FramedResult]
    fake: list[FakeResult]


class GenericRaw(TypedDict):
    text: str
    tool_use: JsonValue
    is_finished: bool
    finish_reason: str
    usage: UsageView | None
    index: int | None


class FunctionView(TypedDict):
    name: NotRequired[str | None]
    arguments: NotRequired[str | None]


class ToolCallView(TypedDict):
    id: NotRequired[str | None]
    index: NotRequired[int | None]
    function: NotRequired[FunctionView]


class DeltaView(TypedDict):
    content: NotRequired[str | None]
    reasoning_content: NotRequired[str | None]
    tool_calls: NotRequired[list[JsonValue] | None]


class ChoiceView(TypedDict):
    index: int
    finish_reason: NotRequired[str | None]
    delta: DeltaView


class ChunkView(TypedDict):
    choices: list[ChoiceView]
    usage: JsonValue | None


class ChunkLike(Protocol): ...


class Framed(Protocol):
    def iter_bytes(
        self, iterator: Iterator[bytes], *, response_headers: Mapping[str, str] | None = None
    ) -> Iterator[ChunkLike]: ...

    def aiter_bytes(
        self, iterator: AsyncIterator[bytes], *, response_headers: Mapping[str, str] | None = None
    ) -> AsyncIterator[ChunkLike]: ...
