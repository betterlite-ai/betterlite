import logging
from typing import Literal, Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.bedrock import common_utils
from litellm.llms.bedrock.common_utils import (
    apply_bedrock_invoke_structured_output,
    strip_unsupported_bedrock_invoke_output_config_keys,
    tools_without_eager_input_streaming,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


# LiteLLM types these four bodies as a bare dict; the JSON the cases send is that same mapping.
class Common(Protocol):
    # ast-grep-ignore: no-long-names
    def ensure_bedrock_anthropic_messages_tool_names(
        self, request_body: dict[str, JsonValue]
    ) -> None: ...

    # ast-grep-ignore: no-long-names
    def normalize_custom_field_on_tools(self, request_body: dict[str, JsonValue]) -> None: ...

    # ast-grep-ignore: no-long-names
    def normalize_json_schema_custom_types_to_object(
        self, schema: dict[str, JsonValue]
    ) -> None: ...

    # ast-grep-ignore: no-long-names
    def normalize_tool_input_schema_types_for_bedrock_invoke(
        self, request_body: dict[str, JsonValue]
    ) -> None: ...


COMMON: Common = common_utils


class ApplyStep(TypedDict):
    function: Literal["apply_structured_output"]
    model: str
    body: dict[str, JsonValue]


class StripStep(TypedDict):
    function: Literal["strip_unsupported_config_keys"]
    model: str
    body: dict[str, JsonValue]


class BodyStep(TypedDict):
    function: Literal["normalize_custom_field", "normalize_tool_schemas", "ensure_tool_names"]
    body: dict[str, JsonValue]


class ToolsStep(TypedDict):
    function: Literal["tools_without_eager_streaming"]
    body: dict[str, JsonValue]


class SchemaStep(TypedDict):
    function: Literal["normalize_schema"]
    schema: dict[str, JsonValue]


Step: TypeAlias = ApplyStep | StripStep | BodyStep | ToolsStep | SchemaStep


class Input(TypedDict):
    model_cost: dict[str, dict[str, JsonValue]]
    steps: list[Step]


class Collector(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.seen = False

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno >= logging.WARNING:
            self.seen = True


def warned_answer(step: ApplyStep | StripStep) -> JsonValue:
    body = TypeAdapter(dict[str, object]).validate_python(step["body"])
    collector = Collector()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(collector)
    try:
        if step["function"] == "apply_structured_output":
            apply_bedrock_invoke_structured_output(step["model"], body)
        else:
            strip_unsupported_bedrock_invoke_output_config_keys(step["model"], body)
    finally:
        logger.removeHandler(collector)
    return {"body": TypeAdapter(JsonValue).validate_python(body), "warned": collector.seen}


def body_answer(step: BodyStep) -> JsonValue:
    body = step["body"]
    if step["function"] == "normalize_custom_field":
        COMMON.normalize_custom_field_on_tools(body)
    elif step["function"] == "normalize_tool_schemas":
        COMMON.normalize_tool_input_schema_types_for_bedrock_invoke(body)
    else:
        COMMON.ensure_bedrock_anthropic_messages_tool_names(body)
    return {"body": body}


def tools_answer(step: ToolsStep) -> JsonValue:
    tools = tools_without_eager_input_streaming(step["body"])
    return {"tools": TypeAdapter(JsonValue).validate_python(tools)}


def schema_answer(step: SchemaStep) -> JsonValue:
    schema = step["schema"]
    COMMON.normalize_json_schema_custom_types_to_object(schema)
    return {"schema": schema}


def answer(step: Step) -> JsonValue:
    if step["function"] == "apply_structured_output":
        return warned_answer(step)
    if step["function"] == "strip_unsupported_config_keys":
        return warned_answer(step)
    if step["function"] == "normalize_schema":
        return schema_answer(step)
    if step["function"] == "tools_without_eager_streaming":
        return tools_answer(step)
    return body_answer(step)


def run(input_value: Input) -> list[JsonValue]:
    litellm.model_cost = input_value["model_cost"]
    return [answer(step) for step in input_value["steps"]]
