from types import SimpleNamespace
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.responses.litellm_completion_transformation import streaming_iterator
from litellm.responses.litellm_completion_transformation.streaming_iterator import (
    LiteLLMCompletionStreamingIterator as StreamIterator,
)
from litellm.types.llms.base import BaseLiteLLMOpenAIResponseObject as BaseResponse
from litellm.types.llms.openai import (
    ChatCompletionRedactedThinkingBlock as RedactedThinkingBlock,
)
from litellm.types.llms.openai import ChatCompletionThinkingBlock as ReasoningBlock
from litellm.types.llms.openai import ResponsesAPIOptionalRequestParams as RequestParams
from litellm.types.utils import (
    Delta,
    ModelResponse,
    ModelResponseStream,
    StreamingChoices,
)
from openai.types.responses import ResponseOutputItem
from pydantic import TypeAdapter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

ThinkingPart: TypeAlias = ReasoningBlock | RedactedThinkingBlock  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L76


class DeltaFields(TypedDict):
    content: NotRequired[str | None]
    reasoning_content: NotRequired[str | None]
    thinking_blocks: NotRequired[Json]


class ChoiceFields(TypedDict):
    delta: NotRequired[DeltaFields]
    finish_reason: NotRequired[Json]


class ChunkFields(TypedDict):
    choices: NotRequired[list[ChoiceFields]]


class CorrelateStep(TypedDict):
    kind: Literal["correlate"]
    calls: list[Json]


class AssignStep(TypedDict):
    kind: Literal["assign"]
    call_ids: list[str]


class DeltasStep(TypedDict):
    kind: Literal["deltas"]
    tool_calls: Json


class FinalStep(TypedDict):
    kind: Literal["final"]
    tool_calls: Json


class ReserveStep(TypedDict):
    kind: Literal["reserve"]
    provider_fields: Json


class ReasoningStep(TypedDict):
    kind: Literal["reasoning_end"]
    chunks: list[ChunkFields]


class SignedStep(TypedDict):
    kind: Literal["signed_thinking"]
    deltas: list[DeltaFields]


class TypedItem(TypedDict):
    kind: Literal["typed"]
    item: Json


class DictItem(TypedDict):
    kind: Literal["dict"]
    item: Json


class ReplaceStep(TypedDict):
    kind: Literal["replace_items"]
    items: list[TypedItem | DictItem]
    item_type: str
    item_id: str | None


Step: TypeAlias = (
    CorrelateStep
    | AssignStep
    | DeltasStep
    | FinalStep
    | ReserveStep
    | ReasoningStep
    | SignedStep
    | ReplaceStep
)


class Input(TypedDict):
    model: str
    request_value: str
    responses_api_request: dict[str, Json]
    custom_llm_provider: str | None
    litellm_metadata: dict[str, Json] | None
    logging_marker: str
    steps: list[Step]


def build_iterator(input_value: Input) -> StreamIterator:
    logging = SimpleNamespace(marker=input_value["logging_marker"])
    wrapper = SimpleNamespace(logging_obj=logging)
    request = cast(RequestParams, input_value["responses_api_request"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1277
    return StreamIterator(
        model=input_value["model"],
        litellm_custom_stream_wrapper=cast(litellm.CustomStreamWrapper, wrapper),  # type: ignore[misc, no-any-unimported]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L89
        request_input=input_value["request_value"],
        responses_api_request=request,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L91
        custom_llm_provider=input_value["custom_llm_provider"],
        litellm_metadata=input_value["litellm_metadata"],
    )


def delta_view(fields: DeltaFields) -> Delta:
    blocks = cast(list[ThinkingPart] | None, fields.get("thinking_blocks"))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1535
    return Delta(
        content=fields.get("content"),
        reasoning_content=fields.get("reasoning_content"),
        thinking_blocks=blocks,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L192
    )


def chunk_view(fields: ChunkFields) -> ModelResponseStream:
    choice_fields = fields.get("choices")
    if choice_fields is None:
        return ModelResponseStream(choices=[])
    choices: list[StreamingChoices] = []
    for choice in choice_fields:
        delta_fields = choice.get("delta")
        delta = delta_view(delta_fields) if delta_fields is not None else Delta()
        streaming = StreamingChoices(delta=delta)
        streaming.finish_reason = cast(str | None, choice.get("finish_reason"))  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2021
        choices.append(streaming)
    return ModelResponseStream(choices=choices)


def completion_view(step: FinalStep) -> ModelResponse:
    message = SimpleNamespace(tool_calls=step["tool_calls"])
    fake = SimpleNamespace(choices=[SimpleNamespace(message=message)])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L298
    return cast(ModelResponse, fake)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L298


def replace_items(step: ReplaceStep) -> list[Json]:
    adapter: TypeAdapter[ResponseOutputItem] = TypeAdapter(ResponseOutputItem)
    inputs: list[ResponseOutputItem | Json] = []
    for entry in step["items"]:
        if entry["kind"] == "typed":
            inputs.append(adapter.validate_python(entry["item"]))
        else:
            inputs.append(entry["item"])
    # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L63
    align = streaming_iterator._output_items_with_id  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L63
    replaced = align(tuple(inputs), step["item_type"], step["item_id"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L63
    items = cast(tuple[Json, ...], replaced)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L63
    ids: list[Json] = []
    for entry, item in zip(step["items"], items, strict=True):
        if entry["kind"] == "dict":
            ids.append(cast(dict[str, Json], item).get("id"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L63
        else:
            # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L63
            view = cast(dict[str, Json], cast(ResponseOutputItem, item).model_dump(mode="json"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L63
            ids.append(view.get("id"))
    return ids


def final_step(
    iterator: StreamIterator,
    step: FinalStep,
) -> None:
    completion = completion_view(step)
    # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L298
    iterator._queue_final_tool_call_done_events(completion)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L298


def event_view(event: BaseResponse) -> dict[str, Json]:
    view = cast(dict[str, Json], event.model_dump(mode="json"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/base.py#L29
    view["sequence_number"] = cast(Json, event.get("sequence_number"))  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L276
    return view


def flags_snapshot(iterator: StreamIterator) -> dict[str, bool]:
    return {
        "finished": iterator.finished,
        "sent_response_created": iterator.sent_response_created_event,
        "sent_output_item_added": iterator.sent_output_item_added_event,
        "sent_content_part_added": iterator.sent_content_part_added_event,
        "upstream_exhausted": iterator._upstream_exhausted,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L122
        "final_tool_events_queued": iterator._final_tool_events_queued,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L131
    }


def counters_snapshot(iterator: StreamIterator) -> dict[str, int]:
    return {
        "sequence_number": iterator._sequence_number,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L132
        "message_output_index": iterator._message_output_index,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L119
        "next_tool_output_index": iterator._next_tool_output_index,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L130
    }


def queues_snapshot(iterator: StreamIterator) -> Json:
    return cast(Json, {  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L124
        "pending_tool_events": [event_view(event) for event in iterator._pending_tool_events],  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L124
        "tool_output_index_by_call_id": iterator._tool_output_index_by_call_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L125
        "tool_args_by_call_id": iterator._tool_args_by_call_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L126
        "tool_item_id_by_call_id": iterator._tool_item_id_by_call_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L127
        "tool_call_id_by_index": iterator._tool_call_id_by_index,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L128
        "ambiguous_tool_call_indexes": sorted(iterator._ambiguous_tool_call_indexes),  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L129
        "web_search_calls": iterator._web_search_calls,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L148
        "queued_web_search_call_ids": sorted(iterator._queued_web_search_call_ids),  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L149
    })


def snapshot(iterator: StreamIterator) -> Json:
    context = cast(str, cast(SimpleNamespace, iterator.litellm_logging_obj).marker)  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L105
    entries = iterator._namespace_tool_names.items()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L145
    names = {name: [mapped[0], mapped[1]] for name, mapped in entries}
    return cast(Json, {  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L95
        "model": iterator.model,
        "request_input": iterator.request_input,
        "responses_api_request": iterator.responses_api_request,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L98
        "custom_llm_provider": iterator.custom_llm_provider,
        "litellm_metadata": iterator.litellm_metadata,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L100
        "logging_context": context,
        "custom_tool_names": sorted(iterator._custom_tool_names),  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L144
        "namespace_tool_names": names,
        "flags": flags_snapshot(iterator),
        "counters": counters_snapshot(iterator),
        "queues": queues_snapshot(iterator),
    })


def run(input_value: Input) -> Json:
    iterator = build_iterator(input_value)
    results: list[Json] = []
    for step in input_value["steps"]:
        if step["kind"] == "correlate":
            # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L219
            ids = [
                iterator._tool_call_id(call)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L219
                for call in step["calls"]
            ]
            results.append(cast(Json, ids))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L219
        elif step["kind"] == "assign":
            # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L151
            indices = [
                iterator._get_or_assign_tool_output_index(call_id)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L151
                for call_id in step["call_ids"]
            ]
            results.append(cast(Json, indices))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L151
        elif step["kind"] == "reasoning_end":
            # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L184
            ends = [
                bool(iterator._is_reasoning_end(chunk_view(chunk)))  # type: ignore[misc, no-untyped-call]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L184
                for chunk in step["chunks"]
            ]
            results.append(cast(Json, ends))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L184
        elif step["kind"] == "signed_thinking":
            # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L76
            signed = [
                streaming_iterator._delta_has_signed_thinking_block(delta_view(delta))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L76
                for delta in step["deltas"]
            ]
            results.append(cast(Json, signed))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L76
        elif step["kind"] == "replace_items":
            results.append(cast(Json, replace_items(step)))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L63
        elif step["kind"] == "deltas":
            # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L234
            iterator._queue_tool_call_delta_events(step["tool_calls"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L234
        elif step["kind"] == "reserve":
            # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L197
            iterator._reserve_web_search_indexes(step["provider_fields"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L197
        else:
            final_step(iterator, step)
    return cast(Json, {"results": results, "state": snapshot(iterator)})  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/streaming_iterator.py#L298
