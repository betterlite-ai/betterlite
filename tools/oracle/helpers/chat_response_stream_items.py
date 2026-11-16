from types import SimpleNamespace
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.responses.litellm_completion_transformation import streaming_iterator
from litellm.responses.litellm_completion_transformation.streaming_iterator import (
    LiteLLMCompletionStreamingIterator as StreamIterator,
)
from litellm.types.llms.base import BaseLiteLLMOpenAIResponseObject as BaseResponse
from litellm.types.llms.openai import ResponsesAPIOptionalRequestParams as RequestParams
from litellm.types.utils import Delta, ModelResponse, ModelResponseStream, StreamingChoices
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

OBJECT: TypeAdapter[dict[str, Json]] = TypeAdapter(dict[str, Json])


class DumpedMessage(TypedDict):
    content: Json


class DumpedChoice(TypedDict):
    message: DumpedMessage


class DumpedResponse(TypedDict):
    choices: list[DumpedChoice]


DUMPED: TypeAdapter[DumpedResponse] = TypeAdapter(DumpedResponse)


class MessageFields(TypedDict):
    content: NotRequired[Json]
    annotations: NotRequired[Json]


class CompletionChoice(TypedDict):
    message: MessageFields


class CompletionFields(TypedDict):
    choices: list[CompletionChoice]


class ChunkChoice(TypedDict):
    index: int
    delta: dict[str, Json]
    finish_reason: Json


class ChunkFields(TypedDict):
    id: str
    object: str
    created: int
    model: str
    choices: list[ChunkChoice]
    usage: NotRequired[dict[str, Json]]


class InitialStep(TypedDict):
    kind: Literal["initial"]


class MessageItemsStep(TypedDict):
    kind: Literal["message_items"]


class ContentPartStep(TypedDict):
    kind: Literal["content_part"]


class DoneStep(TypedDict):
    kind: Literal["done"]
    target: Literal["text", "part", "item"]
    completion: CompletionFields


class ReasoningDoneStep(TypedDict):
    kind: Literal["reasoning_done"]
    item_id: str
    content: str
    sequence_number: int


class DefaultDoneStep(TypedDict):
    kind: Literal["default_done"]
    completion: CompletionFields


class PendingStep(TypedDict):
    kind: Literal["pending"]


class SnapshotStep(TypedDict):
    kind: Literal["snapshot"]
    chunk: ChunkFields
    hidden: NotRequired[dict[str, Json]]


class MergeStep(TypedDict):
    kind: Literal["merge_fields"]
    fields: dict[str, Json]


class AssembleStep(TypedDict):
    kind: Literal["assemble"]


class FinishedStep(TypedDict):
    kind: Literal["finished"]


class SetStep(TypedDict):
    kind: Literal["set"]
    cached_item_id: NotRequired[str]
    cached_response_id: NotRequired[str]
    cached_reasoning_item_id: NotRequired[str]
    sequence_number: NotRequired[int]
    message_output_index: NotRequired[int]
    next_tool_output_index: NotRequired[int]
    litellm_model_response: NotRequired[CompletionFields]


Step: TypeAlias = (
    InitialStep
    | MessageItemsStep
    | ContentPartStep
    | DoneStep
    | ReasoningDoneStep
    | DefaultDoneStep
    | PendingStep
    | SnapshotStep
    | MergeStep
    | AssembleStep
    | FinishedStep
    | SetStep
)


class Input(TypedDict):
    model: str
    request_value: str
    responses_api_request: dict[str, Json]
    custom_llm_provider: str | None
    litellm_metadata: dict[str, Json] | None
    logging_marker: str
    now: int
    uuid_ids: list[str]
    steps: list[Step]


class FixedTime:
    def __init__(self, now: int) -> None:
        self.now = now

    def time(self) -> float:
        return float(self.now)


class FixedUuid:
    def __init__(self, ids: list[str]) -> None:
        self.ids = ids
        self.count = 0

    def uuid4(self) -> str:
        if self.ids:
            return self.ids.pop(0)
        self.count += 1
        return f"missing-uuid-{self.count}"


def no_response_cost(result: Json) -> Json:
    del result
    return None


def build_iterator(input_value: Input) -> StreamIterator:
    details: dict[str, Json] = {}
    logging = SimpleNamespace(
        marker=input_value["logging_marker"],
        model_call_details=details,
        _response_cost_calculator=no_response_cost,
    )
    wrapper = SimpleNamespace(logging_obj=logging)
    request = cast(RequestParams, input_value["responses_api_request"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1277
    iterator = StreamIterator(
        model=input_value["model"],
        litellm_custom_stream_wrapper=cast(litellm.CustomStreamWrapper, wrapper),  # type: ignore[misc, no-any-unimported]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L89
        request_input=input_value["request_value"],
        responses_api_request=request,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1277
        custom_llm_provider=input_value["custom_llm_provider"],
        litellm_metadata=input_value["litellm_metadata"],
    )
    streaming_iterator.time = FixedTime(input_value["now"])  # type: ignore[attr-defined,assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L1
    streaming_iterator.uuid = FixedUuid(input_value["uuid_ids"])  # type: ignore[attr-defined,assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L2
    return iterator


def event_view(event: BaseResponse) -> Json:
    view = OBJECT.validate_json(event.model_dump_json())
    sequence = event.get("sequence_number")  # type: ignore[no-untyped-call,misc]  # pyright: ignore[reportUnknownMemberType,reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/base.py#L30
    if sequence is not None:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/base.py#L30
        view["sequence_number"] = cast(Json, sequence)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/base.py#L30
    return view


def completion_view(fields: CompletionFields) -> ModelResponse:
    message = fields["choices"][0]["message"]
    shell = SimpleNamespace(
        content=message.get("content"),
        annotations=message.get("annotations"),
    )
    fake = SimpleNamespace(choices=[SimpleNamespace(message=shell)])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L744
    return cast(ModelResponse, fake)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L744


def chunk_view(fields: ChunkFields) -> ModelResponseStream:
    choices: list[StreamingChoices] = []
    for choice in fields["choices"]:
        content = cast(str | None, choice["delta"].get("content"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1513
        finish_reason = cast(str | None, choice["finish_reason"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2007
        delta = Delta(content=content)
        choices.append(StreamingChoices(index=choice["index"], delta=delta, finish_reason=finish_reason))
    chunk = ModelResponseStream(
        id=fields["id"],
        object=fields["object"],
        created=fields["created"],
        model=fields["model"],
        choices=choices,
    )
    if "usage" in fields:
        chunk.usage = fields["usage"]  # type: ignore[attr-defined]  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2136
    return chunk


def pending_events(iterator: StreamIterator) -> Json:
    views: list[Json] = [event_view(event) for event in iterator._pending_response_events]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L135
    iterator._pending_response_events.clear()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L135
    return views


def flags_view(iterator: StreamIterator) -> dict[str, Json]:
    return {
        "sent_response_created": iterator.sent_response_created_event,
        "sent_in_progress": iterator.sent_response_in_progress_event,
        "sent_output_item_added": iterator.sent_output_item_added_event,
        "sent_message_item_added": iterator.sent_message_item_added_event,
        "sent_content_part_added": iterator.sent_content_part_added_event,
        "sent_output_text_done": iterator.sent_output_text_done_event,
        "sent_content_part_done": iterator.sent_output_content_part_done_event,
        "sent_output_item_done": iterator.sent_output_item_done_event,
        "sent_annotation_events": iterator.sent_annotation_events,
    }


def counters_view(iterator: StreamIterator) -> dict[str, Json]:
    return {
        "sequence_number": iterator._sequence_number,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L132
        "message_output_index": iterator._message_output_index,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L119
        "next_tool_output_index": iterator._next_tool_output_index,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L130
    }


def state_view(iterator: StreamIterator, marker: str) -> Json:
    return {
        "flags": flags_view(iterator),
        "counters": counters_view(iterator),
        "cached_item_id": iterator._cached_item_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L120
        "cached_response_id": iterator._cached_response_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L121
        "cached_reasoning_item_id": iterator._cached_reasoning_item_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L133
        "pending_response_events": [event_view(event) for event in iterator._pending_response_events],  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L135
        "provider_specific_fields": OBJECT.validate_python(iterator._accumulated_provider_specific_fields),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L150
        "logging_context": marker,
    }


def message_items_step(iterator: StreamIterator) -> Json:
    iterator._queue_message_item_added_events()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L600
    return pending_events(iterator)


def done_step(iterator: StreamIterator, step: DoneStep) -> Json:
    completion = completion_view(step["completion"])
    if step["target"] == "text":
        return event_view(iterator.create_output_text_done_event(completion))
    if step["target"] == "part":
        return event_view(iterator.create_output_content_part_done_event(completion))
    return event_view(iterator.create_output_item_done_event(completion))


def reasoning_step(iterator: StreamIterator, step: ReasoningDoneStep) -> Json:
    item_id = step["item_id"]
    content = step["content"]
    sequence = step["sequence_number"]
    text = event_view(iterator.create_reasoning_summary_text_done_event(item_id, content, sequence))
    part = event_view(iterator.create_reasoning_summary_part_done_event(item_id, content, sequence))
    item = event_view(iterator.create_reasoning_output_item_done_event(item_id, content, sequence))
    return [text, part, item]


def default_done_step(iterator: StreamIterator, step: DefaultDoneStep) -> Json:
    event = iterator.return_default_done_events(completion_view(step["completion"]))
    if event is None:
        return None
    return event_view(event)


def snapshot_view(snapshot: Json) -> Json:
    view = OBJECT.validate_python(snapshot)
    return {"usage": view.get("usage"), "_hidden_params": view.get("_hidden_params")}


def snapshot_step(iterator: StreamIterator, step: SnapshotStep) -> Json:
    chunk = chunk_view(step["chunk"])
    hidden = step.get("hidden")
    if hidden is not None:
        chunk._hidden_params = dict(hidden)
    snapshot = iterator._snapshot_chunk_for_stream_chunk_builder(chunk)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage,reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L658
    iterator.collected_chat_completion_chunks.append(snapshot)
    return snapshot_view(cast(Json, snapshot))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L658


def merge_step(iterator: StreamIterator, step: MergeStep) -> Json:
    iterator._merge_provider_specific_fields(step["fields"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L618
    return OBJECT.validate_python(iterator._accumulated_provider_specific_fields)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L150


def assembled_view(response: ModelResponse | None) -> Json:
    if response is None:
        return None
    dumped = DUMPED.validate_json(response.model_dump_json())
    hidden = OBJECT.validate_python(response._hidden_params)  # type: ignore[misc]  # pyright: ignore[reportPrivateUsage,reportUnknownMemberType,reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
    return {
        "content": dumped["choices"][0]["message"]["content"],
        "provider_specific_fields": hidden.get("provider_specific_fields"),
    }


def assemble_step(iterator: StreamIterator) -> Json:
    return assembled_view(iterator.create_litellm_model_response())


def set_step(iterator: StreamIterator, step: SetStep) -> Json:
    if "cached_item_id" in step:
        iterator._cached_item_id = step["cached_item_id"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L120
    if "cached_response_id" in step:
        iterator._cached_response_id = step["cached_response_id"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L121
    if "cached_reasoning_item_id" in step:
        iterator._cached_reasoning_item_id = step["cached_reasoning_item_id"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L133
    if "sequence_number" in step:
        iterator._sequence_number = step["sequence_number"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L132
    if "message_output_index" in step:
        iterator._message_output_index = step["message_output_index"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L119
    if "next_tool_output_index" in step:
        iterator._next_tool_output_index = step["next_tool_output_index"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L130
    if "litellm_model_response" in step:
        iterator.litellm_model_response = completion_view(step["litellm_model_response"])
    return None


def apply_step(iterator: StreamIterator, step: Step) -> Json:
    if step["kind"] == "initial":
        event = iterator.return_default_initial_events()
        if event is None:
            return None
        return event_view(event)
    return event_step(iterator, step)


def event_step(iterator: StreamIterator, step: Step) -> Json:
    if step["kind"] == "message_items":
        return message_items_step(iterator)
    if step["kind"] == "content_part":
        return event_view(iterator.create_content_part_added_event())
    if step["kind"] == "done":
        return done_step(iterator, step)
    if step["kind"] == "reasoning_done":
        return reasoning_step(iterator, step)
    if step["kind"] == "default_done":
        return default_done_step(iterator, step)
    return state_step(iterator, step)


def state_step(iterator: StreamIterator, step: Step) -> Json:
    if step["kind"] == "pending":
        return pending_events(iterator)
    if step["kind"] == "snapshot":
        return snapshot_step(iterator, step)
    if step["kind"] == "merge_fields":
        return merge_step(iterator, step)
    if step["kind"] == "assemble":
        return assemble_step(iterator)
    if step["kind"] == "finished":
        return iterator.is_stream_finished()
    return set_step(iterator, cast(SetStep, step))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L119


def run(input_value: Input) -> Json:
    iterator = build_iterator(input_value)
    results: list[Json] = []
    for step in input_value["steps"]:
        outcome: Json = {
            "step": apply_step(iterator, step),
            "state": state_view(iterator, input_value["logging_marker"]),
        }
        results.append(outcome)
    return {"results": results}
