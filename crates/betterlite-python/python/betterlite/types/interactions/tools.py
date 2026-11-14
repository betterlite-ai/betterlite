from typing import ClassVar, Final, Literal, TypeAlias

from pydantic import BaseModel, ConfigDict, Field, JsonValue, RootModel

from betterlite.types.interactions.enums import (
    Environment,
    ResponseModality,
    ThinkingSummaries,
    ToolChoiceType,
)


class Function(BaseModel):
    name: str | None = None
    description: str | None = None
    parameters: dict[str, JsonValue] | None = None
    type: Literal["function"]


class CodeExecution(BaseModel):
    type: Literal["code_execution"]


class UrlContext(BaseModel):
    type: Literal["url_context"]


class ComputerUse(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(
        serialize_by_alias=True,
        validate_by_name=True,
    )

    type: Literal["computer_use"]
    environment: Environment | None = None
    excluded_predefined_functions: list[str] | None = Field(
        default=None,
        alias="excludedPredefinedFunctions",
    )


class GoogleSearch(BaseModel):
    type: Literal["google_search"]


class FileSearch(BaseModel):
    file_search_store_names: list[str] | None = None
    top_k: int | None = None
    metadata_filter: str | None = None
    type: Literal["file_search"]


class AllowedTools(BaseModel):
    mode: ToolChoiceType | None = None
    tools: list[str] | None = None


class McpServer(BaseModel):
    type: Literal["mcp_server"]
    name: str | None = None
    url: str | None = None
    headers: dict[str, str] | None = None
    allowed_tools: list[AllowedTools] | None = None


class DynamicAgentConfig(BaseModel):
    type: Literal["dynamic"] = "dynamic"


class DeepResearchAgentConfig(BaseModel):
    type: Literal["deep-research"] = "deep-research"
    thinking_summaries: ThinkingSummaries | None = None


class SpeechConfig(BaseModel):
    voice: str | None = None
    language: str | None = None
    speaker: str | None = None


class ModalityTokens(BaseModel):
    modality: ResponseModality | None = None
    tokens: int | None = None


class Usage(BaseModel):
    total_input_tokens: int | None = None
    input_tokens_by_modality: list[ModalityTokens] | None = None
    total_cached_tokens: int | None = None
    cached_tokens_by_modality: list[ModalityTokens] | None = None
    total_output_tokens: int | None = None
    output_tokens_by_modality: list[ModalityTokens] | None = None
    total_tool_use_tokens: int | None = None
    tool_use_tokens_by_modality: list[ModalityTokens] | None = None
    total_reasoning_tokens: int | None = None
    total_tokens: int | None = None


class ToolChoiceConfig(BaseModel):
    allowed_tools: AllowedTools | None = None


class ToolChoice(RootModel[ToolChoiceType | ToolChoiceConfig]):
    root: ToolChoiceType | ToolChoiceConfig


class Tool(
    RootModel[
        Function
        | GoogleSearch
        | CodeExecution
        | UrlContext
        | ComputerUse
        | McpServer
        | FileSearch
    ]
):
    root: (
        Function
        | GoogleSearch
        | CodeExecution
        | UrlContext
        | ComputerUse
        | McpServer
        | FileSearch
    ) = Field(discriminator="type")


InteractionTool: TypeAlias = Tool
InteractionToolChoiceConfig: Final = ToolChoiceConfig
