from typing import ClassVar, Literal, TypeAlias

from pydantic import ConfigDict, JsonValue

from betterlite.types.llms.openai.chat.chat_response import ChatCompletionUsageBlock
from betterlite.types.utils.base import ResponseObject


# ast-grep-ignore: no-long-names
class AnthropicResponseContentBlockText(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    type: Literal["text"]
    text: str


# ast-grep-ignore: no-long-names
class AnthropicResponseContentBlockToolUse(ResponseObject):
    type: Literal["tool_use"]
    id: str
    name: str
    input: dict[str, JsonValue]
    provider_specific_fields: dict[str, JsonValue] | None = None


# ast-grep-ignore: no-long-names
class AnthropicResponseContentBlockThinking(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    type: Literal["thinking"]
    thinking: str
    signature: str | None


# ast-grep-ignore: no-long-names
class AnthropicResponseContentBlockRedactedThinking(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    type: Literal["redacted_thinking"]
    data: str


class AnthropicResponseUsageBlock(ResponseObject):
    input_tokens: int
    output_tokens: int


class AnthropicOutputTokensDetails(ResponseObject):
    thinking_tokens: int | None = None


AnthropicFinishReason: TypeAlias = Literal[
    "end_turn", "max_tokens", "stop_sequence", "tool_use", "refusal"
]


class AnthropicResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    id: str
    type: Literal["message"]
    role: Literal["assistant"]
    content: list[
        # ast-grep-ignore: no-long-names
        AnthropicResponseContentBlockText
        # ast-grep-ignore: no-long-names
        | AnthropicResponseContentBlockToolUse
        # ast-grep-ignore: no-long-names
        | AnthropicResponseContentBlockThinking
        # ast-grep-ignore: no-long-names
        | AnthropicResponseContentBlockRedactedThinking
    ]
    model: str
    stop_reason: AnthropicFinishReason | None
    stop_sequence: str | None
    usage: AnthropicResponseUsageBlock


# ast-grep-ignore: no-long-names
class AnthropicChatCompletionUsageBlock(ChatCompletionUsageBlock, total=False):
    cache_creation_input_tokens: int
    cache_read_input_tokens: int
