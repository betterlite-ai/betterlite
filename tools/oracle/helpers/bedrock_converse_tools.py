from typing import Literal, TypedDict, cast

import litellm
from litellm.llms.bedrock.chat.converse_transformation import AmazonConverseConfig
from litellm.types.llms.bedrock import BedrockToolSpec
from litellm.types.llms.openai import OpenAIChatCompletionToolParam
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Choice(TypedDict):
    function: Literal["tool_choice"]
    model: str
    model_cost: dict[str, JsonValue]
    tool_choice: JsonValue
    global_drop: bool
    call_drop: bool


class Detect(TypedDict):
    function: Literal["detector"]
    tools: list[dict[str, JsonValue]] | None


class Split(TypedDict):
    function: Literal["separate"]
    tools: list[dict[str, JsonValue]]


class Shape(TypedDict):
    function: Literal["transform"]
    tools: list[dict[str, JsonValue]]


class JsonTool(TypedDict):
    function: Literal["json_tool"]
    json_schema: dict[str, JsonValue] | None
    description: str | None


class Apply(TypedDict):
    function: Literal["apply_tools"]
    tools: list[dict[str, JsonValue]]
    model: str
    stream: JsonValue
    optional_params: dict[str, JsonValue]


class Trans(TypedDict):
    function: Literal["translate"]
    model: str
    model_cost: dict[str, JsonValue]
    value: dict[str, JsonValue]
    stream: JsonValue
    is_thinking_enabled: bool
    optional_params: dict[str, JsonValue]


class Schema(TypedDict):
    function: Literal["additional"]
    schema: dict[str, JsonValue]


class Config(TypedDict):
    function: Literal["output_config"]
    json_schema: dict[str, JsonValue] | None
    name: str | None
    description: str | None


class Spec(TypedDict):
    function: Literal["tool_spec"]
    name: str
    description: str
    parameters: dict[str, JsonValue]
    strict: bool | None
    supports_strict_tools: bool


class Media(TypedDict):
    function: Literal["media_types"]


Input = Choice | Detect | Split | Shape | JsonTool | Apply | Trans | Schema | Config | Spec | Media
Rest = Apply | Trans | Schema | Config | Spec | Media


# The typed helpers are reachable through a subclass, which is the only seam ruff and pyright allow.
class ConverseToolsConfig(AmazonConverseConfig):
    def map_choice(self, call: Choice) -> dict[str, JsonValue] | None:
        return cast("dict[str, JsonValue] | None", self.map_tool_choice_values(call["model"], cast("str | dict[str, JsonValue]", call["tool_choice"]), call["call_drop"]))  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L739-L768

    def computer_used(self, call: Detect) -> bool:
        return self.is_computer_use_tool_used(cast("list[OpenAIChatCompletionToolParam] | None", call["tools"]), "")  # type: ignore[misc]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L782-L793

    def computer_split(self, call: Split) -> tuple[JsonValue, JsonValue]:
        return cast("tuple[JsonValue, JsonValue]", self._separate_computer_use_tools(cast("list[OpenAIChatCompletionToolParam]", call["tools"]), ""))  # type: ignore[misc]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L835-L866

    def computer_transform(self, call: Shape) -> list[JsonValue]:
        return cast("list[JsonValue]", self._transform_computer_use_tools(cast("list[OpenAIChatCompletionToolParam]", call["tools"])))  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L795-L833

    def json_call(self, call: JsonTool) -> dict[str, JsonValue]:
        return cast("dict[str, JsonValue]", self._create_json_tool_call_for_response_format(call["json_schema"], call["description"]))  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L868-L906

    def apply_transformation(self, call: Apply) -> None:
        self._apply_tool_call_transformation(cast("list[OpenAIChatCompletionToolParam]", call["tools"]), call["model"], {"stream": call["stream"]}, call["optional_params"])  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L998-L1008

    def translate_format(self, call: Trans) -> dict[str, JsonValue]:
        return cast("dict[str, JsonValue]", self._translate_response_format_param(call["value"], call["model"], call["optional_params"], {"stream": call["stream"]}, call["is_thinking_enabled"]))  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L1201-L1269

    def add_schema_properties(self, call: Schema) -> dict[str, JsonValue]:
        return cast("dict[str, JsonValue]", self._add_additional_properties_to_schema(call["schema"]))  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L922-L955

    def output_config_block(self, call: Config) -> JsonValue:
        return JSON.validate_python(self._create_output_config_for_response_format(call["json_schema"], call["name"], call["description"]))  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/converse_transformation.py#L958-L996

    def tool_spec_block(self, call: Spec) -> dict[str, JsonValue]:
        return cast("dict[str, JsonValue]", BedrockToolSpec(name=call["name"], description=call["description"], parameters=call["parameters"], strict=call["strict"], supports_strict_tools=call["supports_strict_tools"]))  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/bedrock.py#L296-L322

    def supported_lists(self) -> JsonValue:
        lists: dict[str, list[str]] = {
            "image": self.get_supported_image_types(),
            "document": self.get_supported_document_types(),
            "video": self.get_supported_video_types(),
        }
        return JSON.validate_python(lists)


def run(input_value: Input) -> JsonValue:
    result: JsonValue
    if input_value["function"] == "tool_choice":
        litellm.drop_params = input_value["global_drop"]
        litellm.model_cost = input_value["model_cost"]
        result = {"block": ConverseToolsConfig().map_choice(input_value)}
    elif input_value["function"] == "detector":
        result = {"used": ConverseToolsConfig().computer_used(input_value)}
    elif input_value["function"] == "separate":
        computer, regular = ConverseToolsConfig().computer_split(input_value)
        result = {"computer": computer, "regular": regular}
    elif input_value["function"] == "transform":
        result = {"tools": ConverseToolsConfig().computer_transform(input_value)}
    elif input_value["function"] == "json_tool":
        result = {"tool": ConverseToolsConfig().json_call(input_value)}
    else:
        result = remaining(input_value)
    return result


def remaining(input_value: Rest) -> JsonValue:
    config = ConverseToolsConfig()
    if input_value["function"] == "apply_tools":
        config.apply_transformation(input_value)
        return {"params": input_value["optional_params"]}
    if input_value["function"] == "translate":
        litellm.model_cost = input_value["model_cost"]
        return {"params": config.translate_format(input_value)}
    if input_value["function"] == "additional":
        return {"schema": config.add_schema_properties(input_value)}
    if input_value["function"] == "output_config":
        return {"config": config.output_config_block(input_value)}
    if input_value["function"] == "tool_spec":
        return {"block": config.tool_spec_block(input_value)}
    return config.supported_lists()
