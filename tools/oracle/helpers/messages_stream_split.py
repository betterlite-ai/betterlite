import asyncio
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

from litellm.exceptions import MidStreamFallbackError
from litellm.llms.anthropic.pass_through.adapters.streaming_iterator import (
    _CombinedChunkSplitter,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L94
    _delta_payload_field,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L80
    _error_status_and_message,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L65
    _mid_stream_error_sse_event,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L71
)
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.llms.anthropic import StreamingContentBlockDeltaType
from litellm.types.utils import ModelResponseStream, StreamingChoices
from pydantic import JsonValue as Json

JsonObject: TypeAlias = dict[str, Json]


class ChunkInput(TypedDict):
    id: str
    created: int
    model: str
    choices: list[JsonObject]
    usage: NotRequired[dict[str, int] | None]
    hidden_params: NotRequired[JsonObject]
    provider_specific_fields: NotRequired[JsonObject | None]
    system_fingerprint: NotRequired[str | None]


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
    messages_attr: list[Json] | None
    failure: FailureInput | None


class FailureStep(TypedDict):
    function: Literal["error_status_and_message", "mid_stream_error_sse_event"]
    failure: FailureInput


class DeltaStep(TypedDict):
    function: Literal["delta_payload_field"]
    delta_type: StreamingContentBlockDeltaType


Step: TypeAlias = SplitStep | FailureStep | DeltaStep


class Input(TypedDict):
    steps: list[Step]


class UnrelatedError(Exception):
    def __init__(self, message: str, status_code: int) -> None:
        super().__init__(message)
        self.status_code = status_code


class SplitterRun:
    def __init__(self, splitter: _CombinedChunkSplitter, stream: "FakeChunkStream") -> None:
        self.splitter = splitter
        self.stream = stream
        self.pieces: list[Json] = []
        self.ended = False
        self.error: JsonObject | None = None


class FakeChunkStream:
    def __init__(
        self,
        chunks: list[ModelResponseStream],
        chunks_attr: list[ModelResponseStream] | None,
        messages_attr: list[Json] | None,
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


def build_chunk(chunk: ChunkInput) -> ModelResponseStream:
    built = ModelResponseStream(
        id=chunk["id"],
        created=chunk["created"],
        model=chunk["model"],
        system_fingerprint=chunk.get("system_fingerprint"),
        provider_specific_fields=chunk.get("provider_specific_fields"),
        usage=chunk.get("usage"),
        choices=cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103
            list[StreamingChoices], chunk["choices"]
        ),
    )
    hidden = chunk.get("hidden_params")
    if hidden is not None:
        built._hidden_params = hidden  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
    for built_choice, spec in zip(built.choices, chunk["choices"], strict=True):
        if spec.get("finish_reason") == "":
            built_choice.finish_reason = ""  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2020
    return built


DELTA_FIELDS = (
    "role",
    "content",
    "reasoning_content",
    "thinking_blocks",
    "tool_calls",
    "function_call",
    "audio",
    "images",
    "annotations",
    "refusal",
    "provider_specific_fields",
)


def project_delta(delta: JsonObject) -> JsonObject:
    return {
        field: delta.get(field) for field in DELTA_FIELDS
    }


def project_choice(choice: JsonObject) -> JsonObject:
    delta = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2006
        JsonObject, choice.get("delta")
    )
    return {
        "index": choice.get("index"),
        "finish_reason": choice.get("finish_reason"),
        "delta": project_delta(delta),
    }


def project_usage(dumped: JsonObject) -> Json:
    usage = dumped.get("usage")
    if usage is None:
        return None
    fields = cast("JsonObject", usage)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2139
    counters = ("prompt_tokens", "completion_tokens", "total_tokens")
    return {counter: fields.get(counter) for counter in counters}


def project_chunk(chunk: ModelResponseStream) -> JsonObject:
    dumped = cast(JsonObject, chunk.model_dump())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2096
    choices = cast(list[JsonObject], dumped.get("choices"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2104
    choice_views: list[Json] = [project_choice(choice) for choice in choices]
    hidden = cast(JsonObject, chunk._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
    return {
        "id": chunk.id,
        "object": chunk.object,
        "created": chunk.created,
        "model": chunk.model,
        "system_fingerprint": chunk.system_fingerprint,
        "provider_specific_fields": dumped.get("provider_specific_fields"),
        "choices": choice_views,
        "usage": project_usage(dumped),
        "hidden_params": hidden,
    }


def build_failure(failure: FailureInput) -> Exception:
    kind = failure["kind"]
    status = failure["status_code"]
    if kind == "base_llm":
        if status is None:
            raise ValueError("a reported failure names its status_code")
        return BaseLLMException(status_code=status, message=failure["message"])
    if kind == "mid_stream_fallback":
        if status is None:
            raise ValueError("a reported failure names its status_code")
        error = MidStreamFallbackError(
            message=failure["message"], model="m", llm_provider="openai"
        )
        error.status_code = status
        error.message = failure["message"]
        return error
    return UnrelatedError(failure["message"], 500 if status is None else status)


def status_view(error: Exception) -> JsonObject:
    status, message = _error_status_and_message(error)  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L65
    return {"status": status, "message": message}


def build_run(step: SplitStep) -> SplitterRun:
    chunks = [build_chunk(chunk) for chunk in step["chunks"]]
    spec_chunks = step["chunks_attr"]
    chunks_attr = None if spec_chunks is None else [build_chunk(chunk) for chunk in spec_chunks]
    failure = None if step["failure"] is None else build_failure(step["failure"])
    stream = FakeChunkStream(chunks, chunks_attr, step["messages_attr"], failure)
    return SplitterRun(_CombinedChunkSplitter(stream), stream)


def splitter_view(run: SplitterRun) -> JsonObject:
    chunks_attr = run.stream.chunks
    projected_chunks: list[Json] | None = None
    if chunks_attr is not None:
        projected_chunks = [project_chunk(chunk) for chunk in chunks_attr]
    sources: list[Json] = [project_chunk(chunk) for chunk in run.stream.source]
    return {
        "pulls": run.stream.pulls,
        "pieces": run.pieces,
        "ended": run.ended,
        "error": run.error,
        "chunks": projected_chunks,
        "messages": run.stream.messages,
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


async def drive(step: SplitStep) -> JsonObject:
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


def answer(step: Step) -> Json:
    if step["function"] == "split":
        return asyncio.run(drive(step))
    if step["function"] == "error_status_and_message":
        return status_view(build_failure(step["failure"]))
    if step["function"] == "mid_stream_error_sse_event":
        return _mid_stream_error_sse_event(build_failure(step["failure"])).decode()  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L71
    if step["function"] == "delta_payload_field":
        return _delta_payload_field(step["delta_type"])  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/streaming_iterator.py#L80
    raise ValueError("unknown messages_stream_split step")


def run(input_value: Input) -> list[Json]:
    return [
        answer(step) for step in input_value["steps"]
    ]
