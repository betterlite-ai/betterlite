from collections.abc import Sequence
from typing import Literal, Required

from pydantic import JsonValue
from typing_extensions import TypedDict

from betterlite.types.llms.bedrock.converse.content import SystemContentBlock
from betterlite.types.llms.bedrock.converse.messages import MessageBlock, ServiceTierBlock
from betterlite.types.llms.bedrock.converse.tools import ToolConfigBlock


class GuardrailConfigBlock(TypedDict, total=False):
    guardrailIdentifier: str
    guardrailVersion: str
    trace: Literal["enabled", "disabled", "enabled_full"]


class InferenceConfig(TypedDict, total=False):
    maxTokens: int
    stopSequences: list[str]
    temperature: float
    topP: float
    topK: int


class PerformanceConfigBlock(TypedDict):
    latency: Literal["optimized", "throughput"]


class JsonSchemaDefinition(TypedDict, total=False):
    schema: Required[str]
    name: str
    description: str


class OutputFormatStructure(TypedDict, total=False):
    jsonSchema: Required[JsonSchemaDefinition]


class OutputFormat(TypedDict):
    type: Literal["json_schema"]
    structure: OutputFormatStructure


class OutputConfigBlock(TypedDict, total=False):
    textFormat: OutputFormat


class CommonRequestObject(TypedDict, total=False):
    additionalModelRequestFields: dict[str, JsonValue]
    additionalModelResponseFieldPaths: Sequence[str]  # ast-grep-ignore: no-long-names
    inferenceConfig: InferenceConfig
    system: list[SystemContentBlock]
    toolConfig: ToolConfigBlock
    guardrailConfig: GuardrailConfigBlock | None
    performanceConfig: PerformanceConfigBlock | None
    serviceTier: ServiceTierBlock | None
    requestMetadata: dict[str, str] | None
    outputConfig: OutputConfigBlock | None


class RequestObject(CommonRequestObject, total=False):
    messages: Required[list[MessageBlock]]


class BedrockInvokeNovaRequest(TypedDict, total=False):
    messages: list[MessageBlock]
    inferenceConfig: InferenceConfig
    system: list[SystemContentBlock]
    toolConfig: ToolConfigBlock
    guardrailConfig: GuardrailConfigBlock | None
