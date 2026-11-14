from collections.abc import Mapping, Sequence
from typing import Literal

from pydantic import BaseModel, JsonValue
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class ContentBlock(BaseModel):
    type: str | None = None
    text: str | None = None


class ToolCallFunction(BaseModel):
    name: str
    arguments: str


class ToolCall(BaseModel):
    id: str
    type: str = "function"
    function: ToolCallFunction


class AssistantMessage(BaseModel):
    role: Literal["assistant"] = "assistant"
    content: str | Sequence[ContentBlock] | None = None
    tool_calls: Sequence[ToolCall] | None = None


class SingulrGuardrailPayload(BaseModel):
    correlation_id: str | None = None
    model_name: str | None = None
    model_provider_name: str | None = None
    guardrail_scope: str | None = None
    messages: Sequence[Mapping[str, JsonValue]] | None = None
    images: Sequence[str] | None = None
    tools: Sequence[Mapping[str, JsonValue]] | None = None
    response: AssistantMessage | None = None
    metadata: Mapping[str, str] | None = None


class SingulrMcpGuardrailPayload(BaseModel):
    model_name: str | None = None
    guardrail_scope: str | None = None
    tool_name: str | None = None
    tool_arguments: JsonValue = None
    mcp_server_name: str | None = None
    tool_result: Sequence[str] | None = None
    metadata: Mapping[str, str] | None = None


class SingulrGuardrailResponse(BaseModel):
    should_block: bool = False
    blocking_due_to: str | None = None


class SingulrGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    singulr_api_key: str | None = None
    singulr_api_base: str | None = None
    singulr_application_id: str | None = None
    singulr_guardrail_id: str | None = None
    block_on_error: bool | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Singulr"
