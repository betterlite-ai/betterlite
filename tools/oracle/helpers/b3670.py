from typing import Literal, TypeAlias, TypedDict

from litellm.llms.anthropic.chat.guardrail_translation.handler import _is_client_tool_use  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/guardrail_translation/handler.py#L188
from litellm.llms.openai.responses.guardrail_translation.handler import (
    _input_item_provenance,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/guardrail_translation/handler.py#L322
    _item_rewrite_field,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/guardrail_translation/handler.py#L251
    _post_guardrail_tool_call_shapes,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/guardrail_translation/handler.py#L142
    _rewritten_input_item,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/guardrail_translation/handler.py#L260
    _tool_call_rewrite,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/guardrail_translation/handler.py#L170
    _tool_call_shapes,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/guardrail_translation/handler.py#L126
    _undeliverable_tool_call_rewrite_reason,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/guardrail_translation/handler.py#L174
)
from pydantic import JsonValue, TypeAdapter

_JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ToolCallFunction(TypedDict, total=False):
    name: str | None
    arguments: str
    provider_specific_fields: dict[str, JsonValue] | None


class ToolCallChunk(TypedDict):
    id: str | None
    type: Literal["function"]
    function: ToolCallFunction
    index: int


_CALL: TypeAdapter[ToolCallChunk] = TypeAdapter(ToolCallChunk)


class InputItemProvenanceInput(TypedDict):
    function: Literal["input_item_provenance"]
    raw_input: list[JsonValue]
    expected_messages: list[JsonValue]


class ItemRewriteFieldInput(TypedDict):
    function: Literal["item_rewrite_field"]
    item: dict[str, JsonValue]


class RewrittenItemInput(TypedDict):
    function: Literal["rewritten_item"]
    item: dict[str, JsonValue]
    rewritten: JsonValue


class ToolCallShapesInput(TypedDict):
    function: Literal["tool_call_shapes"]
    calls: list[JsonValue]


class PostGuardrailShapesInput(TypedDict):
    function: Literal["post_guardrail_shapes"]
    returned: list[JsonValue] | None
    pre: list[JsonValue]
    guardrail_name: str | None


class ToolCallRewriteInput(TypedDict):
    function: Literal["tool_call_rewrite"]
    before: JsonValue
    after: JsonValue


class UndeliverableReasonInput(TypedDict):
    function: Literal["undeliverable_reason"]
    call_ids: list[str]
    tool_call_item_count: int
    post_guardrail_tool_call_count: int
    unresolved_argument_event: bool
    rewritten_call_ids: list[str]
    event_call_ids: list[str]


class ClientToolUseInput(TypedDict):
    function: Literal["client_tool_use"]
    block: dict[str, JsonValue]


Input: TypeAlias = (
    InputItemProvenanceInput
    | ItemRewriteFieldInput
    | RewrittenItemInput
    | ToolCallShapesInput
    | PostGuardrailShapesInput
    | ToolCallRewriteInput
    | UndeliverableReasonInput
    | ClientToolUseInput
)


def call_shapes(calls: list[ToolCallChunk]) -> list[JsonValue]:
    shapes: list[JsonValue] = []
    shapes.extend(
        {"name": shape.name, "arguments": shape.arguments}
        for shape in _tool_call_shapes(calls)
    )
    return shapes


def provenance_answer(input_value: InputItemProvenanceInput) -> JsonValue:
    provenance = _input_item_provenance(
        input_value["raw_input"], input_value["expected_messages"]
    )
    if provenance is None:
        return None
    item_for_message, tainted = provenance
    message_items: dict[str, JsonValue] = {
        str(message_index): item_index
        for message_index, item_index in item_for_message.items()
    }
    tainted_indexes: list[JsonValue] = []
    tainted_indexes.extend(sorted(tainted))
    return {"item_for_message": message_items, "tainted": tainted_indexes}


def rewritten_item_answer(input_value: RewrittenItemInput) -> JsonValue:
    patched = _rewritten_input_item(input_value["item"], input_value["rewritten"])
    if patched is None:
        return None
    patched_items: dict[str, JsonValue] = {}
    for key, value in patched.items():
        patched_items[key] = _JSON.validate_python(value)
    return patched_items


def tool_call_shapes_answer(input_value: ToolCallShapesInput) -> JsonValue:
    typed_calls = [_CALL.validate_python(call) for call in input_value["calls"]]
    return call_shapes(typed_calls)


def post_guardrail_shapes_answer(input_value: PostGuardrailShapesInput) -> JsonValue:
    pre_calls = [_CALL.validate_python(call) for call in input_value["pre"]]
    pre_shapes = _tool_call_shapes(pre_calls)
    shapes = _post_guardrail_tool_call_shapes(
        input_value["returned"], pre_shapes, input_value["guardrail_name"]
    )
    answer: list[JsonValue] = []
    answer.extend(
        {"name": shape.name, "arguments": shape.arguments} for shape in shapes
    )
    return answer


def tool_call_rewrite_answer(input_value: ToolCallRewriteInput) -> JsonValue:
    before_call: ToolCallChunk = _CALL.validate_python(input_value["before"])
    after_call: ToolCallChunk = _CALL.validate_python(input_value["after"])
    before_calls = [before_call]
    after_calls = [after_call]
    before_shape = _tool_call_shapes(before_calls)[0]
    after_shape = _tool_call_shapes(after_calls)[0]
    rewrite = _tool_call_rewrite(before_shape, after_shape)
    return {"name": rewrite.name, "arguments": rewrite.arguments}


def undeliverable_reason_answer(input_value: UndeliverableReasonInput) -> JsonValue:
    return _undeliverable_tool_call_rewrite_reason(
        input_value["call_ids"],
        input_value["tool_call_item_count"],
        input_value["post_guardrail_tool_call_count"],
        input_value["unresolved_argument_event"],
        frozenset(input_value["rewritten_call_ids"]),
        frozenset(input_value["event_call_ids"]),
    )


def client_tool_use_answer(input_value: ClientToolUseInput) -> JsonValue:
    return _is_client_tool_use(input_value["block"])


def run(input_value: Input) -> JsonValue:
    answer: JsonValue
    if input_value["function"] == "input_item_provenance":
        answer = provenance_answer(input_value)
    elif input_value["function"] == "item_rewrite_field":
        answer = _item_rewrite_field(input_value["item"])
    elif input_value["function"] == "rewritten_item":
        answer = rewritten_item_answer(input_value)
    elif input_value["function"] == "tool_call_shapes":
        answer = tool_call_shapes_answer(input_value)
    elif input_value["function"] == "post_guardrail_shapes":
        answer = post_guardrail_shapes_answer(input_value)
    elif input_value["function"] == "tool_call_rewrite":
        answer = tool_call_rewrite_answer(input_value)
    elif input_value["function"] == "undeliverable_reason":
        answer = undeliverable_reason_answer(input_value)
    else:
        answer = client_tool_use_answer(input_value)
    return answer
