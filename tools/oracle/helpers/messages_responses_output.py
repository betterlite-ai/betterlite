import json
from typing import Final, Protocol, TypedDict, runtime_checkable

from litellm.llms.anthropic.pass_through.responses_adapters.transformation import (
    LiteLLMAnthropicToResponsesAPIAdapter as ResponsesAdapter,
)
from litellm.types.llms.anthropic import AnthropicMessagesRequest
from litellm.types.llms.openai import ResponsesAPIResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter

ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)
ANSWER: Final = TypeAdapter(JsonValue)
RESPONSE: Final = TypeAdapter(ResponsesAPIResponse)


class Call(TypedDict):
    request: AnthropicMessagesRequest | None
    response: JsonValue
    include_encrypted_reasoning: bool


@runtime_checkable
class Adapter(Protocol):
    def translate_request(
        self,
        anthropic_request: AnthropicMessagesRequest,
        include_encrypted_reasoning: bool = True,
    ) -> JsonValue: ...

    def translate_response(self, response: ResponsesAPIResponse) -> JsonValue: ...


VIEW: Final[Adapter] = TypeAdapter[Adapter](Adapter, config=ARBITRARY).validate_python(
    ResponsesAdapter()
)


def run(call: Call) -> JsonValue:
    answer: dict[str, JsonValue] = {}
    if call["request"] is not None:
        answer["request"] = VIEW.translate_request(
            call["request"],
            include_encrypted_reasoning=call["include_encrypted_reasoning"],
        )
    if call["response"] is not None:
        answer["response"] = VIEW.translate_response(
            RESPONSE.validate_python(call["response"])
        )
    return ANSWER.validate_json(json.dumps(answer))
