from typing import Literal, TypeAlias

from pydantic import AwareDatetime, BaseModel, Field, JsonValue

from betterlite.types.interactions.content import Content
from betterlite.types.interactions.enums import (
    ResponseModality,
    Status1,
    Status3,
    ThinkingLevel,
    ThinkingSummaries,
)
from betterlite.types.interactions.options import AgentOption, ModelOption
from betterlite.types.interactions.tools import (
    DeepResearchAgentConfig,
    DynamicAgentConfig,
    SpeechConfig,
    Tool,
    ToolChoice,
    Usage,
)


class Turn(BaseModel):
    role: str | None = None
    content: str | list[Content] | None = None


class GenerationConfig(BaseModel):
    temperature: float | None = None
    top_p: float | None = None
    seed: int | None = None
    stop_sequences: list[str] | None = None
    tool_choice: ToolChoice | None = None
    thinking_level: ThinkingLevel | None = None
    thinking_summaries: ThinkingSummaries | None = None
    max_output_tokens: int | None = None
    speech_config: list[SpeechConfig] | None = None


class Interaction(BaseModel):
    model: ModelOption | None = None
    agent: AgentOption | None = None
    id: str
    status: Status1
    created: AwareDatetime | None = None
    updated: AwareDatetime | None = None
    outputs: list[Content] | None = None
    system_instruction: str | None = None
    tools: list[Tool] | None = None
    background: bool | None = None
    object: Literal["interaction"] = "interaction"
    usage: Usage | None = None
    response_modalities: list[ResponseModality] | None = None
    response_format: JsonValue = None
    response_mime_type: str | None = None
    previous_interaction_id: str | None = None
    input: str | list[Content] | list[Turn] | Content | None = None
    generation_config: GenerationConfig | None = None
    agent_config: DynamicAgentConfig | DeepResearchAgentConfig | None = Field(
        default=None,
        discriminator="type",
    )


class CreateModelInteractionParams(BaseModel):
    model: ModelOption
    stream: bool | None = None
    store: bool | None = None
    id: str | None = None
    status: Status3 | None = None
    created: AwareDatetime | None = None
    updated: AwareDatetime | None = None
    outputs: list[Content] | None = None
    system_instruction: str | None = None
    tools: list[Tool] | None = None
    background: bool | None = None
    usage: Usage | None = None
    response_modalities: list[ResponseModality] | None = None
    response_format: JsonValue = None
    response_mime_type: str | None = None
    previous_interaction_id: str | None = None
    input: str | list[Content] | list[Turn] | Content
    generation_config: GenerationConfig | None = None


class CreateAgentInteractionParams(BaseModel):
    agent: AgentOption
    stream: bool | None = None
    store: bool | None = None
    id: str | None = None
    status: Status3 | None = None
    created: AwareDatetime | None = None
    updated: AwareDatetime | None = None
    outputs: list[Content] | None = None
    system_instruction: str | None = None
    tools: list[Tool] | None = None
    background: bool | None = None
    usage: Usage | None = None
    response_modalities: list[ResponseModality] | None = None
    response_format: JsonValue = None
    response_mime_type: str | None = None
    previous_interaction_id: str | None = None
    input: str | list[Content] | list[Turn] | Content
    agent_config: DynamicAgentConfig | DeepResearchAgentConfig | None = Field(
        default=None,
        discriminator="type",
    )


InteractionInput: TypeAlias = str | Content | list[Content] | list[Turn]
InteractionEnvironment: TypeAlias = str | dict[str, JsonValue]
# ast-grep-ignore: no-long-names
InteractionsAPIOptionalRequestParams: TypeAlias = dict[str, JsonValue]
