from typing import Final, Literal, Required

from pydantic import JsonValue
from typing_extensions import TypedDict, override

from betterlite.types.llms.bedrock.converse.content import CachePointBlock


class ToolJsonSchemaBlock(TypedDict, total=False):
    type: Literal["object"]
    properties: dict[str, JsonValue]
    required: list[str]
    additionalProperties: bool


class ToolInputSchemaBlock(TypedDict):
    json: ToolJsonSchemaBlock | None


class ToolSpecBlock(TypedDict, total=False):
    inputSchema: Required[ToolInputSchemaBlock]
    name: Required[str]
    description: str
    strict: bool


class SystemToolBlock(TypedDict, total=False):
    name: Required[str]


class ToolBlock(TypedDict, total=False):
    toolSpec: ToolSpecBlock | None
    systemTool: SystemToolBlock | None
    cachePoint: CachePointBlock | None


class BedrockToolParameters(TypedDict, total=False):
    type: Required[Literal["object"]]
    properties: dict[str, JsonValue]
    required: list[str]
    additionalProperties: bool | None


class BedrockToolSpec(dict[str, ToolSpecBlock]):
    @override
    def __init__(
        self,
        *,
        name: str,
        description: str,
        parameters: BedrockToolParameters,
        strict: bool | None,
        supports_strict_tools: bool,
    ) -> None:
        json_schema: Final[ToolJsonSchemaBlock] = {
            "type": parameters["type"],
            "properties": {},
            "required": [],
        }
        if "properties" in parameters:
            json_schema["properties"] = parameters["properties"]
        if "required" in parameters:
            json_schema["required"] = parameters["required"]
        if supports_strict_tools and "additionalProperties" in parameters:
            additional_properties = parameters["additionalProperties"]
            if additional_properties is not None:
                json_schema["additionalProperties"] = additional_properties

        tool_spec: Final[ToolSpecBlock] = {
            "inputSchema": {"json": json_schema},
            "name": name,
            "description": description,
        }
        if supports_strict_tools and strict:
            tool_spec["strict"] = strict

        super().__init__(toolSpec=tool_spec)


class SpecificToolChoiceBlock(TypedDict):
    name: str


class ToolChoiceValuesBlock(TypedDict, total=False):
    any: dict[str, JsonValue]
    auto: dict[str, JsonValue]
    tool: SpecificToolChoiceBlock


class ToolConfigBlock(TypedDict, total=False):
    tools: Required[list[ToolBlock]]
    toolChoice: str | ToolChoiceValuesBlock
