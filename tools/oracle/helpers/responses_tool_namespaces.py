import json
import logging
from collections.abc import Iterator
from typing import Literal, TypeAlias, TypedDict, cast

from litellm.responses.litellm_completion_transformation.transformation import (
    TOOL_CALLS_CACHE,
)
from litellm.responses.litellm_completion_transformation.transformation import (
    LiteLLMCompletionResponsesConfig as ResponsesConfig,
)
from litellm.types.llms.openai import (
    ResponsesAPIOptionalRequestParams as RequestParams,
)
from litellm.types.utils import (
    ChatCompletionMessageToolCall,
    Choices,
    Function,
    Message,
    ModelResponse,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

Json: TypeAlias = JsonValue
WIRE = TypeAdapter(JsonValue)


class MappingFields(TypedDict):
    kind: Literal["mapping"]
    fields: dict[str, Json]


class RecordFields(TypedDict):
    kind: Literal["record"]
    fields: dict[str, Json]


class PlainFields(TypedDict):
    kind: Literal["plain"]
    value: Json


FieldsInput: TypeAlias = MappingFields | RecordFields | PlainFields


class CallInput(TypedDict):
    id: str
    type: str
    name: str
    arguments: str | dict[str, Json] | None
    status: Literal["in_progress", "completed", "incomplete"] | None
    tool_fields: FieldsInput | None
    function_fields: FieldsInput | None


class ChoiceInput(TypedDict):
    tool_calls: list[CallInput]
    provider_specific_fields: dict[str, Json] | None


class ChatInput(TypedDict):
    operation: Literal["tools_to_chat"]
    tools: list[dict[str, Json]]


class ResponsesInput(TypedDict):
    operation: Literal["tools_to_responses"]
    tools: list[dict[str, Json]]


class OutputsInput(TypedDict):
    operation: Literal["tool_outputs"]
    tools: list[dict[str, Json]]
    choices: list[ChoiceInput]


Input: TypeAlias = ChatInput | ResponsesInput | OutputsInput


class Record:
    def __init__(self, fields: dict[str, Json]) -> None:
        self.fields = fields

    def __iter__(self) -> Iterator[tuple[str, Json]]:
        return iter(self.fields.items())


class WarningCapture(logging.Filter):
    def __init__(self) -> None:
        super().__init__()
        self.warnings: list[str] = []

    @override
    def filter(self, record: logging.LogRecord) -> bool:
        self.warnings.append(record.getMessage())
        return True


def provider_value(fields: FieldsInput) -> dict[str, Json] | Record | Json:
    if fields["kind"] == "mapping":
        return fields["fields"]
    if fields["kind"] == "record":
        return Record(fields["fields"])
    return fields["value"]


def tool_call(call: CallInput) -> ChatCompletionMessageToolCall:
    function = Function(arguments=call["arguments"], name=call["name"])
    if call["status"] is not None:
        function["status"] = call["status"]
    if call["function_fields"] is not None:
        function["provider_specific_fields"] = provider_value(call["function_fields"])
    chat_call = ChatCompletionMessageToolCall(function=function, id=call["id"], type=call["type"])
    if call["tool_fields"] is not None:
        chat_call["provider_specific_fields"] = provider_value(call["tool_fields"])
    return chat_call


def tool_outputs(input_value: OutputsInput) -> Json:
    choices = [
        Choices(
            message=Message(
                tool_calls=[tool_call(call) for call in choice["tool_calls"]],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1390
                provider_specific_fields=choice["provider_specific_fields"],
            )
        )
        for choice in input_value["choices"]
    ]
    request = cast(RequestParams, {"tools": input_value["tools"]})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1277
    items = ResponsesConfig.transform_chat_completion_tools_to_responses_tools(
        ModelResponse(choices=choices),
        request,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1277
    )
    cached: dict[str, Json] = {}
    for choice in input_value["choices"]:
        for call in choice["tool_calls"]:
            stored = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/in_memory_cache.py#L211
                ChatCompletionMessageToolCall | None,
                TOOL_CALLS_CACHE.get_cache(key=call["id"]),  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/in_memory_cache.py#L211
            )
            cached[call["id"]] = None if stored is None else stored.function.name
    dumped_items = [WIRE.validate_json(item.model_dump_json(exclude_none=True)) for item in items]
    return {"items": dumped_items, "cached": cached}


def run(input_value: Input) -> Json:
    if input_value["operation"] == "tools_to_responses":
        converted = ResponsesConfig.transform_chat_completion_tool_params_to_responses_api_tools(
            input_value["tools"]
        )
        return WIRE.validate_json(json.dumps(converted))
    if input_value["operation"] == "tool_outputs":
        return WIRE.validate_json(json.dumps(tool_outputs(input_value)))
    capture = WarningCapture()
    logger = logging.getLogger("LiteLLM")
    logger.addFilter(capture)
    try:
        chat_tools, search_options = (
            ResponsesConfig.transform_responses_api_tools_to_chat_completion_tools(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L2042
                input_value["tools"]
            )
        )
    finally:
        logger.removeFilter(capture)
    chat = {  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L2042
        "chat_tools": chat_tools,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L2042
        "web_search_options": search_options,
        "warnings": capture.warnings,
    }
    return WIRE.validate_json(json.dumps(chat))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L2042
