from typing import Final, Literal, TypedDict

from litellm.llms.anthropic.pass_through.adapters.transformation import (
    LiteLLMAnthropicMessagesAdapter as MessagesAdapter,
)
from litellm.types.llms.anthropic import AnthropicMessagesToolChoice
from pydantic import JsonValue as Json
from pydantic import TypeAdapter


class Adapter(MessagesAdapter):
    # the base __init__ has no annotations, so a typed subclass avoids an untyped-call suppression
    def __init__(self) -> None:
        ...


ADAPTER: Final = Adapter()
ANSWER: Final = TypeAdapter(Json)


class ToolChoiceCall(TypedDict):
    operation: Literal["tool_choice"]
    tool_choice: AnthropicMessagesToolChoice


class ToolsCall(TypedDict):
    operation: Literal["tools"]
    tools: list[dict[str, Json]]
    model: str | None


class OutputFormatCall(TypedDict):
    operation: Literal["output_format"]
    output_format: Json


class RequestCall(TypedDict):
    operation: Literal["request"]
    request: dict[str, Json]


Input = ToolChoiceCall | ToolsCall | OutputFormatCall | RequestCall


def tool_choice_answer(call: ToolChoiceCall) -> Json:
    choice = ADAPTER.translate_anthropic_tool_choice_to_openai(call["tool_choice"])
    return ANSWER.validate_python(choice)


def tools_answer(call: ToolsCall) -> Json:
    tools: Json
    mapping: Json
    tools, mapping = ADAPTER.translate_anthropic_tools_to_openai(call["tools"], call["model"])  # type: ignore[arg-type, assignment, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L781
    answer: dict[str, Json] = {"tools": tools, "tool_name_mapping": mapping}
    return ANSWER.validate_python(answer)


def output_format_answer(call: OutputFormatCall) -> Json:
    response_format = ADAPTER.translate_anthropic_output_format_to_openai(call["output_format"])
    converted: Json = ANSWER.validate_python(response_format)
    answer: dict[str, Json] = {
        "response_format": converted,
        "output_format": call["output_format"],
    }
    return ANSWER.validate_python(answer)


def request_answer(call: RequestCall) -> Json:
    kwargs: Json
    mapping: Json
    kwargs, mapping = ADAPTER.translate_anthropic_to_openai(call["request"])  # type: ignore[arg-type, assignment, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/transformation.py#L1214
    answer: dict[str, Json] = {"kwargs": kwargs, "tool_name_mapping": mapping}
    return ANSWER.validate_python(answer)


def run(input_value: Input) -> Json:
    match input_value["operation"]:
        case "tool_choice":
            return tool_choice_answer(input_value)
        case "tools":
            return tools_answer(input_value)
        case "output_format":
            return output_format_answer(input_value)
        case "request":
            return request_answer(input_value)
