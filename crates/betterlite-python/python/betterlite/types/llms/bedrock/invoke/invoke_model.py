from pydantic import JsonValue
from typing_extensions import TypedDict


class BedrockToolBlock(TypedDict, total=False):
    toolSpec: dict[str, JsonValue] | None
    systemTool: dict[str, JsonValue] | None
    cachePoint: dict[str, JsonValue] | None


class AmazonDeepSeekR1StreamingResponse(TypedDict):  # ast-grep-ignore: no-long-names
    generation: str
    generation_token_count: int
    stop_reason: str | None
    prompt_token_count: int


# ast-grep-ignore: no-long-names
class BedrockInvokeAnthropicMessagesRequest(TypedDict, total=False):
    anthropic_version: str
    max_tokens: int
    messages: list[JsonValue]
    anthropic_beta: list[str]
    system: JsonValue
    stop_sequences: list[str]
    temperature: float
    top_p: float
    top_k: int
    tools: list[JsonValue]
    tool_choice: dict[str, JsonValue]
    thinking: dict[str, JsonValue]
    metadata: dict[str, JsonValue]
    output_config: dict[str, JsonValue]
    safeguards: list[JsonValue]
    context_management: dict[str, JsonValue]
