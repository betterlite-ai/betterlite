import json
from collections.abc import Iterator
from typing import Literal, TypeAlias, TypedDict

from litellm.responses.litellm_completion_transformation.custom_tools import (
    build_tool_call_item_kwargs,
    convert_custom_tool_to_function_tool,
    extract_custom_tool_names,
    openai_shaped_tool_call_item_id,
    serialize_tool_call_arguments,
    unwrap_custom_tool_arguments,
)
from litellm.responses.litellm_completion_transformation.transformation import (
    LiteLLMCompletionResponsesConfig as ResponsesConfig,
)
from litellm.responses.litellm_completion_transformation.transformation import (
    _attribute_fields,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L179
    _input_item_role,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L185
)
from pydantic import JsonValue, TypeAdapter

Json: TypeAlias = JsonValue


class PaddedInput(TypedDict):
    fill: str
    count: int


class IdInput(TypedDict):
    item_type: str
    tool_id: str


class CallInput(TypedDict):
    call_id: str
    name: str
    arguments: str
    status: str


class NamesInput(TypedDict):
    operation: Literal["custom_names"]
    tools: list[Json]


class SchemaInput(TypedDict):
    operation: Literal["custom_schema"]
    custom_tools: list[dict[str, Json]]


class StringsInput(TypedDict):
    operation: Literal["argument_strings"]
    values: list[Json]
    arguments: list[str]
    padded: list[PaddedInput]
    ids: list[IdInput]
    calls: list[CallInput]
    custom_names: list[str]


class ChoicesInput(TypedDict):
    operation: Literal["tool_choices"]
    choices: list[Json]


class RecordsInput(TypedDict):
    operation: Literal["record_helpers"]
    roles: list[dict[str, Json]]
    plain: list[Json]
    records: list[dict[str, Json]]


Input: TypeAlias = NamesInput | SchemaInput | StringsInput | ChoicesInput | RecordsInput


class Record:
    def __init__(self, fields: dict[str, Json]) -> None:
        self.fields = fields

    def __iter__(self) -> Iterator[tuple[str, Json]]:
        return iter(self.fields.items())


def run(input_value: Input) -> Json:
    wire = TypeAdapter(JsonValue)
    request_choice = ResponsesConfig._transform_tool_choice  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L228
    response_choice = ResponsesConfig._transform_tool_choice_for_responses_api_response  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAny]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L290
    if input_value["operation"] == "custom_names":
        names = sorted(extract_custom_tool_names(input_value["tools"]))
        return wire.validate_json(json.dumps(names))
    if input_value["operation"] == "custom_schema":
        converted = [  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/custom_tools.py#L179
            convert_custom_tool_to_function_tool(tool)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/custom_tools.py#L179
            for tool in input_value["custom_tools"]
        ]
        return wire.validate_json(json.dumps(converted))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/custom_tools.py#L179
    if input_value["operation"] == "argument_strings":
        custom_names = set(input_value["custom_names"])
        strings = {
            "serialized": [
                serialize_tool_call_arguments(value, "fallback")
                for value in input_value["values"]
            ],
            "unwrapped": [
                unwrap_custom_tool_arguments(text) for text in input_value["arguments"]
            ],
            "padded": [
                {"wrapper_kept": text.startswith('{"content"'), "chars": len(text)}
                for text in (
                    unwrap_custom_tool_arguments(
                        '{"content":"' + case["fill"] * case["count"] + '"}'
                    )
                    for case in input_value["padded"]
                )
            ],
            "ids": [
                openai_shaped_tool_call_item_id(case["item_type"], case["tool_id"])
                for case in input_value["ids"]
            ],
            "items": [
                build_tool_call_item_kwargs(
                    call["call_id"],
                    call["name"],
                    call["arguments"],
                    call["status"],
                    custom_names,
                )
                for call in input_value["calls"]
            ],
        }
        return wire.validate_json(json.dumps(strings))
    if input_value["operation"] == "tool_choices":
        choices = {
            "request": [request_choice(choice) for choice in input_value["choices"]],  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L228
            "response": [response_choice(choice) for choice in input_value["choices"]],  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L290
        }
        return wire.validate_json(json.dumps(choices, default=list))
    records = {
        "roles": [_input_item_role(item) for item in input_value["roles"]],
        "plain": [_attribute_fields(value) for value in input_value["plain"]],
        "records": [
            _attribute_fields(Record(fields)) for fields in input_value["records"]
        ],
    }
    return wire.validate_json(json.dumps(records))
