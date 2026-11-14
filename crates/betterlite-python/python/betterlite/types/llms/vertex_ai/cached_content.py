from typing import Required

from typing_extensions import TypedDict

from betterlite.types.llms.vertex_ai.content import ContentType, SystemInstructions
from betterlite.types.llms.vertex_ai.generation import GenerationConfig
from betterlite.types.llms.vertex_ai.safety import SafetSettingsConfig
from betterlite.types.llms.vertex_ai.tools import ToolConfig, Tools
from betterlite.types.llms.vertex_ai.usage import UsageMetadata


class TTL(TypedDict, total=False):
    seconds: Required[float]
    nano: float


class CachedContent(TypedDict, total=False):
    ttl: TTL
    expire_time: str
    contents: list[ContentType]
    tools: list[Tools]
    createTime: str
    updateTime: str
    usageMetadata: UsageMetadata
    expireTime: str
    name: str
    displayName: str
    model: str
    systemInstruction: ContentType
    toolConfig: ToolConfig


class RequestBody(TypedDict, total=False):
    contents: Required[list[ContentType]]
    system_instruction: SystemInstructions
    tools: Tools
    toolConfig: ToolConfig
    safetySettings: list[SafetSettingsConfig]
    generationConfig: GenerationConfig
    cachedContent: str
    labels: dict[str, str]
    serviceTier: str


class CachedContentRequestBody(TypedDict, total=False):
    contents: Required[list[ContentType]]
    system_instruction: SystemInstructions
    tools: Tools
    toolConfig: ToolConfig
    model: Required[str]
    ttl: str
    displayName: str


# ast-grep-ignore: no-long-names
class CachedContentListAllResponseBody(TypedDict, total=False):
    cachedContents: list[CachedContent]
    nextPageToken: str


# ast-grep-ignore: no-long-names
class VertexAICachedContentResponseObject(TypedDict):
    name: str
    model: str
