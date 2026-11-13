import asyncio
from typing import Literal, NotRequired, TypedDict, cast

from litellm.anthropic_interface.exceptions.exception_mapping_utils import (
    anthropic_error_sse_frame,
)
from litellm.exceptions import MidStreamFallbackError
from litellm.llms.anthropic.pass_through.adapters.streaming_iterator import (
    _CombinedChunkSplitter,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L94
    _delta_payload_field,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L80
    _error_status_and_message,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L65
)
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.llms.anthropic import StreamingContentBlockDeltaType
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter


class ChunkInput(TypedDict):
    id: str
    created: int
    model: str
    system_fingerprint: NotRequired[str | None]
    provider_specific_fields: NotRequired[dict[str, JsonValue] | None]
    choices: list[dict[str, JsonValue]]
    usage: NotRequired[dict[str, JsonValue] | None]
    hidden_params: NotRequired[dict[str, JsonValue]]


class FailureInput(TypedDict):
    kind: Literal["base_llm", "mid_stream_fallback", "unclassified"]
    status_code: int | None
    message: str


class SplitStep(TypedDict):
    function: Literal["split"]
    mode: Literal["sync", "async"]
    splitters: int
    rounds: int
    chunks: list[ChunkInput]
    chunks_attr: list[ChunkInput] | None
    messages_attr: list[JsonValue] | None
    failure: FailureInput | None


class FailureStep(TypedDict):
    function: Literal["error_status_and_message", "mid_stream_error_sse_event"]
    failure: FailureInput


class DeltaStep(TypedDict):
    function: Literal["delta_payload_field"]
    delta_type: StreamingContentBlockDeltaType


Step = SplitStep | FailureStep | DeltaStep


class Input(TypedDict):
    steps: list[Step]


class ChoiceDump(TypedDict):
    index: int
    finish_reason: NotRequired[str | None]
    delta: dict[str, JsonValue]


class ChunkDump(TypedDict):
    id: str
    object: str
    created: int
    model: NotRequired[str | None]
    system_fingerprint: NotRequired[str | None]
    provider_specific_fields: NotRequired[dict[str, JsonValue] | None]
    choices: list[ChoiceDump]
    usage: NotRequired[dict[str, JsonValue] | None]


class UnrelatedError(Exception):
    def __init__(self, message: str, status_code: int) -> None:
        super().__init__(message)
        self.status_code = status_code


CHUNK: TypeAdapter[ModelResponseStream] = TypeAdapter(ModelResponseStream)
DUMP: TypeAdapter[ChunkDump] = TypeAdapter(ChunkDump)

DELTA_FIELDS = (
    "role", "content", "reasoning_content", "thinking_blocks", "tool_calls", "function_call",
    "audio", "images", "annotations", "refusal", "provider_specific_fields",
)


def build_chunk(chunk: ChunkInput) -> ModelResponseStream:
    built = CHUNK.validate_python(chunk)
    for built_choice, spec in zip(built.choices, chunk["choices"], strict=True):
        # validation drops an empty finish_reason, and only a null one leaves the splitter alone
        if spec.get("finish_reason") == "":
            built_choice.finish_reason = ""  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2020
    hidden = chunk.get("hidden_params")
    if hidden is not None:
        built._hidden_params = hidden  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
    return built


def hidden_of(chunk: ModelResponseStream) -> JsonValue:
    return cast(dict[str, JsonValue], chunk._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088


def project_choice(choice: ChoiceDump) -> dict[str, JsonValue]:
    delta = choice["delta"]
    return {
        "index": choice["index"],
        "finish_reason": choice.get("finish_reason"),
        "delta": {field: delta.get(field) for field in DELTA_FIELDS},
    }


def project_chunk(chunk: ModelResponseStream) -> dict[str, JsonValue]:
    dumped = DUMP.validate_json(chunk.model_dump_json())
    usage = dumped.get("usage")
    counters = ("prompt_tokens", "completion_tokens", "total_tokens")
    return {
        "id": dumped["id"],
        "object": dumped["object"],
        "created": dumped["created"],
        "model": dumped.get("model"),
        "system_fingerprint": dumped.get("system_fingerprint"),
        "provider_specific_fields": dumped.get("provider_specific_fields"),
        "choices": [project_choice(choice) for choice in dumped["choices"]],
        "usage": None if usage is None else {counter: usage.get(counter) for counter in counters},
        "hidden_params": hidden_of(chunk),
    }


def build_failure(failure: FailureInput) -> Exception:
    kind = failure["kind"]
    status = failure["status_code"]
    if kind == "unclassified":
        return UnrelatedError(failure["message"], 500 if status is None else status)
    if status is None:
        raise ValueError("a reported failure names its status_code")
    if kind == "base_llm":
        return BaseLLMException(status_code=status, message=failure["message"])
    error = MidStreamFallbackError(message=failure["message"], model="m", llm_provider="openai")
    error.status_code = status
    error.message = failure["message"]
    return error


def status_view(error: Exception) -> dict[str, JsonValue]:
    status, message = _error_status_and_message(error)
    return {"status": status, "message": message}


def frame_of(failure: FailureInput) -> JsonValue:
    status, message = _error_status_and_message(build_failure(failure))
    return anthropic_error_sse_frame(status_code=status, raw_message=message)


class SplitterRun:
    def __init__(self, splitter: _CombinedChunkSplitter, stream: "FakeChunkStream") -> None:
        self.splitter = splitter
        self.stream = stream
        self.pieces: list[JsonValue] = []
        self.ended = False
        self.error: dict[str, JsonValue] | None = None


class FakeChunkStream:
    def __init__(
        self,
        chunks: list[ModelResponseStream],
        chunks_attr: list[ModelResponseStream] | None,
        messages_attr: list[JsonValue] | None,
        failure: Exception | None,
    ) -> None:
        self.chunks = chunks_attr
        self.messages = messages_attr
        self.source = chunks
        self._next = 0
        self._failure = failure
        self.pulls = 0

    def __iter__(self) -> "FakeChunkStream":
        return self

    def __aiter__(self) -> "FakeChunkStream":
        return self

    def take(self) -> ModelResponseStream | None:
        self.pulls += 1
        if self._next < len(self.source):
            chunk = self.source[self._next]
            self._next += 1
            return chunk
        if self._failure is not None:
            raise self._failure
        return None

    def __next__(self) -> ModelResponseStream:
        chunk = self.take()
        if chunk is None:
            raise StopIteration
        return chunk

    async def __anext__(self) -> ModelResponseStream:
        chunk = self.take()
        if chunk is None:
            raise StopAsyncIteration
        return chunk


def build_run(step: SplitStep) -> SplitterRun:
    chunks = [build_chunk(chunk) for chunk in step["chunks"]]
    spec_chunks = step["chunks_attr"]
    chunks_attr = None if spec_chunks is None else [build_chunk(chunk) for chunk in spec_chunks]
    failure = None if step["failure"] is None else build_failure(step["failure"])
    stream = FakeChunkStream(chunks, chunks_attr, step["messages_attr"], failure)
    return SplitterRun(_CombinedChunkSplitter(stream), stream)


def splitter_view(run: SplitterRun) -> dict[str, JsonValue]:
    chunks_attr = run.splitter.chunks
    projected_chunks: list[JsonValue] | None = None
    if chunks_attr is not None:
        projected_chunks = [project_chunk(chunk) for chunk in chunks_attr]
    messages_attr = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L126
        "list[JsonValue] | None", run.splitter.messages
    )
    messages: JsonValue = messages_attr
    sources: list[JsonValue] = [project_chunk(chunk) for chunk in run.stream.source]
    return {
        "pulls": run.stream.pulls,
        "pieces": run.pieces,
        "ended": run.ended,
        "error": run.error,
        "chunks": projected_chunks,
        "messages": messages,
        "source": sources,
    }


async def pull(run: SplitterRun, mode: Literal["sync", "async"]) -> ModelResponseStream | None:
    if mode == "sync":
        try:
            return next(run.splitter)
        except StopIteration:
            return None
    try:
        return await anext(run.splitter)
    except StopAsyncIteration:
        return None


async def drive(step: SplitStep) -> dict[str, JsonValue]:
    runs = [build_run(step) for _ in range(step["splitters"])]
    for _ in range(step["rounds"]):
        for run in runs:
            if run.ended or run.error is not None:
                continue
            try:
                piece = await pull(run, step["mode"])
            except BaseLLMException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41
                run.error = status_view(error)
            except (MidStreamFallbackError, UnrelatedError) as error:
                run.error = status_view(error)
            else:
                if piece is None:
                    run.ended = True
                else:
                    run.pieces.append(project_chunk(piece))
    return {"splitters": [splitter_view(run) for run in runs]}


def answer(step: Step) -> JsonValue:
    if step["function"] == "split":
        return asyncio.run(drive(step))
    if step["function"] == "error_status_and_message":
        return status_view(build_failure(step["failure"]))
    if step["function"] == "mid_stream_error_sse_event":
        return frame_of(step["failure"])
    if step["function"] == "delta_payload_field":
        return _delta_payload_field(step["delta_type"])
    raise ValueError("unknown messages_stream_split step")


def run(input_value: Input) -> list[JsonValue]:
    return [
        answer(step)
        for step in input_value["steps"]
    ]
