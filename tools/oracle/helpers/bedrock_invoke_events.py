import asyncio
from collections.abc import AsyncIterator, Iterator, Mapping
from typing import Literal, NotRequired, Protocol, TypedDict

from litellm.llms.bedrock.chat import get_bedrock_event_stream_decoder
from litellm.llms.bedrock.chat.invoke_handler import (
    AmazonAnthropicClaudeStreamDecoder,
    AmazonDeepSeekR1StreamDecoder,
    AmazonOpenAICompatibleStreamDecoder,
    AWSEventStreamDecoder,
    MockResponseIterator,
)
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import JsonValue, TypeAdapter, ValidationError

Provider = Literal["anthropic", "deepseek_r1", "moonshot"]
Construction = Literal["legacy", "anthropic", "deepseek_r1", "moonshot", "base"]
Step = Literal["next", "anext"]


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


class Framed(Protocol):
    def iter_bytes(
        self, iterator: Iterator[bytes], *, response_headers: Mapping[str, str] | None = None
    ) -> Iterator[object]: ...

    def aiter_bytes(
        self, iterator: AsyncIterator[bytes], *, response_headers: Mapping[str, str] | None = None
    ) -> AsyncIterator[object]: ...


GENERIC: TypeAdapter[GenericRaw] = TypeAdapter(GenericRaw)
STREAM: TypeAdapter[ModelResponseStream] = TypeAdapter(ModelResponseStream)
CHUNK: TypeAdapter[ChunkView] = TypeAdapter(ChunkView)
USAGE: TypeAdapter[UsageView] = TypeAdapter(UsageView)
TOOLS: TypeAdapter[list[ToolCallView]] = TypeAdapter(list[ToolCallView])
RESPONSE: TypeAdapter[ModelResponse] = TypeAdapter(ModelResponse)


def usage_view(raw: JsonValue) -> UsageView | None:
    if raw is None:
        return None
    return USAGE.validate_python(raw)


def tool_view(tool: ToolCallView) -> JsonValue:
    function = tool.get("function")
    if function is None:
        return {
            "id": tool.get("id"),
            "name": None,
            "arguments": None,
            "index": tool.get("index"),
        }
    return {
        "id": tool.get("id"),
        "name": function.get("name"),
        "arguments": function.get("arguments"),
        "index": tool.get("index"),
    }


def tool_views(raw: list[JsonValue] | None) -> list[JsonValue] | None:
    if raw is None:
        return None
    return [tool_view(tool) for tool in TOOLS.validate_python(raw)]


def stream_view(stream: ModelResponseStream) -> StreamView:
    view = CHUNK.validate_json(stream.model_dump_json())
    choice = view["choices"][0]
    delta = choice["delta"]
    return {
        "kind": "stream",
        "content": delta.get("content"),
        "reasoning_content": delta.get("reasoning_content"),
        "finish_reason": choice.get("finish_reason"),
        "tool_calls": tool_views(delta.get("tool_calls")),
        "usage": usage_view(view["usage"]),
    }


def generic_view(raw: GenericRaw) -> GenericView:
    return {
        "kind": "generic",
        "text": raw["text"],
        "tool_use": raw["tool_use"],
        "is_finished": raw["is_finished"],
        "finish_reason": raw["finish_reason"],
        "usage": raw["usage"],
        "index": raw["index"],
    }


def chunk_view(value: object) -> GenericView | StreamView:
    try:
        generic = GENERIC.validate_python(value)
    except ValidationError:
        return stream_view(STREAM.validate_python(value))
    return generic_view(generic)


async def collect_async(iterator: AsyncIterator[object]) -> tuple[list[object], FailureView | None]:
    values: list[object] = []
    while True:
        try:
            value = await anext(iterator)
        except StopAsyncIteration:
            return values, None
        except BedrockError as error:
            return values, {"status": error.status_code}
        except ValueError:
            return values, {"status": None}
        values.append(value)


def collect_sync(iterator: Iterator[object]) -> tuple[list[object], FailureView | None]:
    values: list[object] = []
    while True:
        try:
            value = next(iterator)
        except StopIteration:
            return values, None
        except BedrockError as error:
            return values, {"status": error.status_code}
        except ValueError:
            return values, {"status": None}
        values.append(value)


async def frame_stream(frames: list[bytes]) -> AsyncIterator[bytes]:
    for frame in frames:
        yield frame


def framed_result(values: list[object], failure: FailureView | None) -> FramedResult:
    return {"chunks": [chunk_view(value) for value in values], "failure": failure}


def construction_decoder(scenario: FramedInput) -> Framed:
    model = scenario["model"]
    sync = scenario["sync"]
    json_mode = scenario["json_mode"]
    if scenario["construction"] == "legacy":
        return get_bedrock_event_stream_decoder(
            scenario["provider"], model, sync, json_mode is True
        )
    if scenario["construction"] == "anthropic":
        return AmazonAnthropicClaudeStreamDecoder(
            model=model, sync_stream=sync, json_mode=json_mode
        )
    if scenario["construction"] == "deepseek_r1":
        return AmazonDeepSeekR1StreamDecoder(model=model, sync_stream=sync)
    if scenario["construction"] == "moonshot":
        return AmazonOpenAICompatibleStreamDecoder(model=model, sync_stream=sync)
    return AWSEventStreamDecoder(model=model, json_mode=json_mode)


def run_framed(scenario: FramedInput) -> FramedResult:
    frames = [bytes.fromhex(chunk) for chunk in scenario["chunks"]]
    decoder = construction_decoder(scenario)
    if scenario["sync"]:
        values, failure = collect_sync(
            decoder.iter_bytes(iter(frames), response_headers=scenario["headers"])
        )
        return framed_result(values, failure)
    values, failure = asyncio.run(
        collect_async(
            decoder.aiter_bytes(frame_stream(frames), response_headers=scenario["headers"])
        )
    )
    return framed_result(values, failure)


async def anext_or_none(iterator: AsyncIterator[object]) -> object | None:
    return await anext(iterator, None)


def step_view(value: object | None) -> OkStep | FailureStep | DoneStep:
    if value is None:
        return {"done": True}
    return {"ok": chunk_view(value)}


def run_fake(scenario: FakeInput) -> FakeResult:
    response = RESPONSE.validate_python(scenario["response"])
    iterator = MockResponseIterator(model_response=response, json_mode=scenario["json_mode"])
    sync_iterator: Iterator[object] = iterator
    async_iterator: AsyncIterator[object] = iterator
    steps: list[OkStep | FailureStep | DoneStep] = []
    for step in scenario["steps"]:
        try:
            if step == "next":
                steps.append(step_view(next(sync_iterator, None)))
            else:
                steps.append(step_view(asyncio.run(anext_or_none(async_iterator))))
        except ValueError:
            steps.append({"failure": {"status": None}})
    return {"steps": steps}


def run(input_value: Input) -> Output:
    return {
        "framed": [run_framed(scenario) for scenario in input_value["framed"]],
        "fake": [run_fake(scenario) for scenario in input_value["fake"]],
    }
