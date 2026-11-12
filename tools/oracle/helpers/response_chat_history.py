from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

from litellm.completion_extras.litellm_responses_transformation.transformation import (
    LiteLLMResponsesTransformationHandler,
    _as_chat_reasoning_items,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L189
    _incomplete_reason_from_response_payload,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L216
    _map_incomplete_reason_to_finish_reason,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L200
    _provider_metadata,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L81
    _reasoning_input_items,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L119
    _reasoning_items_from_output_items,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L181
    _upstream_response_id,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L91
    tool_call_dict_from_output_item,
)
from openai.types.responses import ResponseOutputMessage, ResponseReasoningItem

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class HistoryCall(TypedDict):
    operation: Literal[
        "leading_system_instructions", "assistant_call_history", "correlated_tool_output"
    ]
    histories: list[list[dict[str, Json]]]


class DictItem(TypedDict):
    kind: Literal["dict"]
    value: dict[str, Json]


class TypedItem(TypedDict):
    kind: Literal["typed"]
    value: dict[str, Json]


class OtherItem(TypedDict):
    kind: Literal["typed_other"]


OutputItem: TypeAlias = DictItem | TypedItem | OtherItem


class ReasoningCall(TypedDict):
    operation: Literal["reasoning_history"]
    messages: list[dict[str, Json]]
    output_item_groups: list[list[OutputItem]]


class DictProvider(TypedDict):
    kind: Literal["dict"]
    value: dict[str, Json]


class AttributesProvider(TypedDict):
    kind: Literal["attributes"]
    region: str


class UnsupportedProvider(TypedDict):
    kind: Literal["unsupported"]
    value: Json


Provider: TypeAlias = DictProvider | AttributesProvider | UnsupportedProvider


class ToolFieldCase(TypedDict):
    item: dict[str, Json]
    index: int
    provider_specific_fields: NotRequired[Provider]


class ToolFieldsCall(TypedDict):
    operation: Literal["output_tool_fields"]
    items: list[ToolFieldCase]


class MetadataResponse(TypedDict):
    fields: NotRequired[dict[str, Json] | None]
    id: NotRequired[str | None]
    payload: Json


class MetadataCall(TypedDict):
    operation: Literal["shared_output_metadata"]
    responses: list[MetadataResponse]


Input: TypeAlias = HistoryCall | ReasoningCall | ToolFieldsCall | MetadataCall


class AttributeFields:
    def __init__(self, region: str) -> None:
        self.region = region


def run(input_value: Input) -> Json:
    if input_value["operation"] == "reasoning_history":
        return _reasoning_history(input_value)
    if input_value["operation"] == "output_tool_fields":
        return _tool_fields(input_value)
    if input_value["operation"] == "shared_output_metadata":
        return _metadata(input_value)
    return _history(input_value)


def _history(call: HistoryCall) -> Json:
    results: list[Json] = []
    for messages in call["histories"]:
        input_items, instructions = (
            LiteLLMResponsesTransformationHandler().convert_chat_completion_messages_to_responses_api(  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L360
                messages,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L360
            )
        )
        converted = {"input": input_items, "instructions": instructions}
        results.append(
            cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L360
                Json, converted
            )
        )
    return results


def _reasoning_history(call: ReasoningCall) -> Json:
    projected = [
        _reasoning_input_items(message)  # type: ignore[arg-type]  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L119
        for message in call["messages"]
    ]
    groups: list[Json] = []
    for group in call["output_item_groups"]:
        output_items: list[Json | ResponseReasoningItem | ResponseOutputMessage] = [
            _output_item(item) for item in group
        ]
        records = list(
            _reasoning_items_from_output_items(output_items)  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L181
        )
        as_chat = _as_chat_reasoning_items(records)  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L189
        group_output = {"built": records, "chat_items": as_chat}
        groups.append(
            cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L181
                Json, group_output
            )
        )
    output = {"reasoning_input": projected, "groups": groups}
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L181
        Json, output
    )


def _output_item(call: OutputItem) -> Json | ResponseReasoningItem | ResponseOutputMessage:
    if call["kind"] == "dict":
        return call["value"]
    if call["kind"] == "typed":
        return ResponseReasoningItem.model_validate(call["value"])
    return ResponseOutputMessage(
        id="msg_unused", type="message", role="assistant", content=[], status="completed"
    )


def _tool_fields(call: ToolFieldsCall) -> Json:
    results: list[Json] = []
    for case in call["items"]:
        item: dict[str, Json | AttributeFields] = dict(case["item"])
        if "provider_specific_fields" in case:
            item["provider_specific_fields"] = _provider_value(case["provider_specific_fields"])
        results.append(
            cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L230
                Json, tool_call_dict_from_output_item(item, case["index"])
            )
        )
    return results


def _provider_value(call: Provider) -> Json | AttributeFields:
    if call["kind"] == "dict":
        return call["value"]
    if call["kind"] == "attributes":
        return AttributeFields(call["region"])
    return call["value"]


def _metadata(call: MetadataCall) -> Json:
    results: list[Json] = []
    for response in call["responses"]:
        metadata = _provider_metadata(response.get("fields"))  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L81
        upstream = _upstream_response_id(response.get("id"))  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L91
        reason = _incomplete_reason_from_response_payload(response["payload"])  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L216
        projection = {
            "metadata": dict(metadata),
            "upstream_response_id": upstream,
            "incomplete_reason": reason,
            "incomplete_finish": _map_incomplete_reason_to_finish_reason(reason),  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L200
        }
        results.append(
            cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L200
                Json, projection
            )
        )
    return results
