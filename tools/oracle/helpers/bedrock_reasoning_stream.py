from collections.abc import Iterable, Mapping
from datetime import UTC, datetime
from typing import NotRequired, Protocol, TypedDict, cast

import httpx
from litellm.litellm_core_utils.core_helpers import (
    get_provider_response_headers_from_hidden_params,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.chat.chat_completions.transformation import (
    AmazonBedrockRuntimeChatCompletionsConfig,
    BedrockRuntimeChatCompletionsStreamingHandler,
    ReasoningTagSplitter,
    chat_completions_params_refused_for,
    chat_completions_params_refused_while_reasoning,
    chat_completions_reasoning_efforts_refused_for,
    non_string_reasoning_effort,
    split_reasoning_tag,
    without_refused_reasoning_effort,
)
from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import JsonValue, TypeAdapter

JsonObject = dict[str, JsonValue]
JSON_ADAPTER: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ChunkParser(Protocol):
    def chunk_parser(self, chunk: dict[str, JsonValue]) -> ModelResponseStream: ...


class RuntimeConfig(Protocol):
    def transform_response(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/chat_completions/transformation.py#L450-L463
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: Logging,
        request_data: dict[str, JsonValue],
        messages: list[dict[str, str]],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        encoding: None,
    ) -> ModelResponse: ...


class SplitStep(TypedDict):
    fragments: list[str]
    messages: list[str]


class ChunkStep(TypedDict):
    chunks: list[JsonObject]


class EffortStep(TypedDict):
    model: str
    params: JsonObject


class RefusalStep(TypedDict):
    models: list[str]
    params: JsonObject


class BufferedStep(TypedDict):
    reply: JsonValue
    headers: dict[str, str]


class Step(TypedDict, total=False):
    split: SplitStep
    chunks: ChunkStep
    effort: EffortStep
    refusals: RefusalStep
    buffered: BufferedStep


class Input(TypedDict):
    step: Step


class DeltaView(TypedDict):
    role: NotRequired[str | None]
    content: NotRequired[str | None]
    reasoning_content: NotRequired[str | None]
    tool_calls: NotRequired[JsonValue]


class StreamChoiceView(TypedDict):
    index: int
    finish_reason: str | None
    delta: DeltaView


class StreamView(TypedDict):
    id: str
    created: int
    model: str | None
    choices: list[StreamChoiceView]
    usage: NotRequired[JsonValue]
    service_tier: NotRequired[str | None]


class ReplyMessageView(TypedDict):
    content: str | None
    reasoning_content: NotRequired[str | None]


class ReplyChoiceView(TypedDict):
    index: int
    finish_reason: str | None
    message: ReplyMessageView


class ReplyView(TypedDict):
    id: str
    created: int
    model: str | None
    choices: list[ReplyChoiceView]


SPLIT_ADAPTER: TypeAdapter[SplitStep] = TypeAdapter(SplitStep)
CHUNK_ADAPTER: TypeAdapter[ChunkStep] = TypeAdapter(ChunkStep)
EFFORT_ADAPTER: TypeAdapter[EffortStep] = TypeAdapter(EffortStep)
REFUSAL_ADAPTER: TypeAdapter[RefusalStep] = TypeAdapter(RefusalStep)
BUFFERED_ADAPTER: TypeAdapter[BufferedStep] = TypeAdapter(BufferedStep)
STREAM_ADAPTER: TypeAdapter[StreamView] = TypeAdapter(StreamView)
REPLY_ADAPTER: TypeAdapter[ReplyView] = TypeAdapter(ReplyView)
PARAMS_ADAPTER: TypeAdapter[JsonObject] = TypeAdapter(JsonObject)


def sorted_names(names: Iterable[str]) -> list[JsonValue]:
    listed: list[JsonValue] = []
    listed.extend(sorted(names))
    return listed


def split_view(state: ReasoningTagSplitter, reasoning: str, content: str) -> JsonObject:
    return {
        "phase": state.phase,
        "pending": state.pending,
        "reasoning": reasoning,
        "content": content,
    }


def run_split(step: SplitStep) -> JsonValue:
    state = ReasoningTagSplitter()
    feeds: list[JsonValue] = []
    for fragment in step["fragments"]:
        state, reasoning, content = state.feed(fragment)
        feeds.append(split_view(state, reasoning, content))
    state, flush_reasoning, flush_content = state.flush()
    again, again_reasoning, again_content = state.flush()
    messages: list[JsonValue] = []
    for message in step["messages"]:
        message_reasoning, message_content = split_reasoning_tag(message)
        messages.append({"reasoning": message_reasoning, "content": message_content})
    view: JsonObject = {
        "feeds": feeds,
        "flush": split_view(state, flush_reasoning, flush_content),
        "flush_again": split_view(again, again_reasoning, again_content),
        "messages": messages,
    }
    return JSON_ADAPTER.validate_python(view)


def chunk_view(reply: ModelResponseStream) -> JsonObject:
    view = STREAM_ADAPTER.validate_json(reply.model_dump_json())
    choices: list[JsonValue] = [
        {
            "index": choice["index"],
            "finish_reason": choice["finish_reason"],
            "role": choice["delta"].get("role"),
            "content": choice["delta"].get("content"),
            "reasoning_content": choice["delta"].get("reasoning_content"),
            "tool_calls": choice["delta"].get("tool_calls"),
        }
        for choice in view["choices"]
    ]
    return {
        "id": view["id"],
        "created": view["created"],
        "model": view["model"],
        "choices": choices,
        "usage": view.get("usage"),
        "service_tier": view.get("service_tier"),
    }


def run_chunks(step: ChunkStep) -> JsonValue:
    handler = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/chat_completions/transformation.py#L218-L227
        ChunkParser,
        BedrockRuntimeChatCompletionsStreamingHandler(
            streaming_response=iter(()), sync_stream=True
        ),
    )
    views: list[JsonValue] = [chunk_view(handler.chunk_parser(chunk)) for chunk in step["chunks"]]
    return JSON_ADAPTER.validate_python(views)


def run_effort(step: EffortStep) -> JsonValue:
    refused = sorted_names(
        chat_completions_params_refused_while_reasoning(step["model"], step["params"])
    )
    non_string = sorted_names(non_string_reasoning_effort(step["params"]))
    view: JsonObject = {"non_string": non_string, "refused_while_reasoning": refused}
    return JSON_ADAPTER.validate_python(view)


def run_refusals(step: RefusalStep) -> JsonValue:
    views: list[JsonValue] = []
    for model in step["models"]:
        kept = PARAMS_ADAPTER.validate_python(
            without_refused_reasoning_effort(model, step["params"])
        )
        views.append({
            "model": model,
            "refused_params": sorted_names(chat_completions_params_refused_for(model)),
            "refused_efforts": sorted_names(chat_completions_reasoning_efforts_refused_for(model)),
            "without_refused_effort": kept,
        })
    return JSON_ADAPTER.validate_python(views)


def run_buffered(step: BufferedStep) -> JsonValue:
    config = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/chat_completions/transformation.py#L450-L463
        RuntimeConfig, AmazonBedrockRuntimeChatCompletionsConfig()
    )
    headers = httpx.Headers(step["headers"])
    messages: list[dict[str, str]] = []
    logging_obj = Logging(
        model="bedrock-runtime",
        messages=messages,
        stream=False,
        call_type="completion",
        start_time=datetime(2020, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="function-1",
        litellm_trace_id="trace-1",
    )
    response = config.transform_response(
        model="bedrock-runtime",
        raw_response=httpx.Response(200, json=step["reply"], headers=headers),
        model_response=ModelResponse(),
        logging_obj=logging_obj,
        request_data={},
        messages=messages,
        optional_params={},
        litellm_params={},
        encoding=None,
    )
    view = REPLY_ADAPTER.validate_json(response.model_dump_json())
    hidden_params = get_provider_response_headers_from_hidden_params(response)
    hidden: Mapping[str, str] = {} if hidden_params is None else hidden_params
    choices: list[JsonValue] = [
        {
            "index": choice["index"],
            "finish_reason": choice["finish_reason"],
            "content": choice["message"].get("content"),
            "reasoning_content": choice["message"].get("reasoning_content"),
        }
        for choice in view["choices"]
    ]
    sent: JsonObject = {name: hidden[name] for name in step["headers"] if name in hidden}
    answer: JsonObject = {
        "id": view["id"],
        "created": view["created"],
        "choices": choices,
        "headers": sent,
    }
    return JSON_ADAPTER.validate_python(answer)


def run(input_value: Input) -> JsonValue:
    step = input_value["step"]
    split_step = step.get("split")
    if split_step is not None:
        return run_split(SPLIT_ADAPTER.validate_python(split_step))
    chunk_step = step.get("chunks")
    if chunk_step is not None:
        return run_chunks(CHUNK_ADAPTER.validate_python(chunk_step))
    effort_step = step.get("effort")
    if effort_step is not None:
        return run_effort(EFFORT_ADAPTER.validate_python(effort_step))
    refusal_step = step.get("refusals")
    if refusal_step is not None:
        return run_refusals(REFUSAL_ADAPTER.validate_python(refusal_step))
    buffered_step = step.get("buffered")
    if buffered_step is not None:
        return run_buffered(BUFFERED_ADAPTER.validate_python(buffered_step))
    raise ValueError("step names no known function")
