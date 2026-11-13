import asyncio
from collections.abc import AsyncIterator, Iterator

from litellm.llms.bedrock.chat import get_bedrock_event_stream_decoder
from litellm.llms.bedrock.chat.invoke_handler import (
    AmazonAnthropicClaudeStreamDecoder,
    AmazonDeepSeekR1StreamDecoder,
    AmazonOpenAICompatibleStreamDecoder,
    AWSEventStreamDecoder,
    MockResponseIterator,
)
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter, ValidationError

from .types import (
    RESPONSE,
    STREAM,
    ChunkLike,
    ChunkView,
    DoneStep,
    FailureStep,
    FailureView,
    FakeInput,
    FakeResult,
    Framed,
    FramedInput,
    FramedResult,
    GenericRaw,
    GenericView,
    Input,
    OkStep,
    Output,
    StreamView,
    ToolCallView,
    UsageView,
)

GENERIC: TypeAdapter[GenericRaw] = TypeAdapter(GenericRaw)
CHUNK: TypeAdapter[ChunkView] = TypeAdapter(ChunkView)
USAGE: TypeAdapter[UsageView] = TypeAdapter(UsageView)
TOOLS: TypeAdapter[list[ToolCallView]] = TypeAdapter(list[ToolCallView])


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


def chunk_view(value: ChunkLike) -> GenericView | StreamView:
    try:
        generic = GENERIC.validate_python(value)
    except ValidationError:
        return stream_view(STREAM.validate_python(value))
    return generic_view(generic)


async def collect_async(
    iterator: AsyncIterator[ChunkLike],
) -> tuple[list[ChunkLike], FailureView | None]:
    values: list[ChunkLike] = []
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


def collect_sync(iterator: Iterator[ChunkLike]) -> tuple[list[ChunkLike], FailureView | None]:
    values: list[ChunkLike] = []
    try:
        values.extend(iterator)
    except BedrockError as error:
        return values, {"status": error.status_code}
    except ValueError:
        return values, {"status": None}
    return values, None


async def frame_stream(frames: list[bytes]) -> AsyncIterator[bytes]:
    for frame in frames:
        yield frame


def construction_decoder(scenario: FramedInput) -> Framed:
    model = scenario["model"]
    sync = scenario["sync"]
    json_mode = scenario["json_mode"]
    if scenario["construction"] == "legacy":
        legacy: Framed = get_bedrock_event_stream_decoder(
            scenario["provider"], model, sync, json_mode is True
        )
        return legacy
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
    else:
        values, failure = asyncio.run(
            collect_async(
                decoder.aiter_bytes(frame_stream(frames), response_headers=scenario["headers"])
            )
        )
    return {"chunks": [chunk_view(value) for value in values], "failure": failure}


async def next_async(iterator: AsyncIterator[ChunkLike]) -> ChunkLike | None:
    try:
        return await anext(iterator)
    except StopAsyncIteration:
        return None


def step_view(value: ChunkLike | None) -> OkStep | FailureStep | DoneStep:
    if value is None:
        return {"done": True}
    return {"ok": chunk_view(value)}


def run_fake(scenario: FakeInput) -> FakeResult:
    response = RESPONSE.validate_python(scenario["response"])
    iterator = MockResponseIterator(model_response=response, json_mode=scenario["json_mode"])
    sync_iterator: Iterator[ChunkLike] = iterator
    async_iterator: AsyncIterator[ChunkLike] = iterator
    steps: list[OkStep | FailureStep | DoneStep] = []
    for step in scenario["steps"]:
        try:
            if step == "next":
                steps.append(step_view(next(sync_iterator, None)))
            else:
                steps.append(step_view(asyncio.run(next_async(async_iterator))))
        except ValueError:
            steps.append({"failure": {"status": None}})
    return {"steps": steps}


def run(input_value: Input) -> Output:
    return {
        "framed": [run_framed(scenario) for scenario in input_value["framed"]],
        "fake": [run_fake(scenario) for scenario in input_value["fake"]],
    }
