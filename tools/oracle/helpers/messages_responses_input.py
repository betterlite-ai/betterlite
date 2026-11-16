import json
from typing import Final, Protocol, TypedDict, runtime_checkable

from litellm.llms.anthropic.pass_through.responses_adapters.transformation import (
    LiteLLMAnthropicToResponsesAPIAdapter as ResponsesAdapter,
)
from pydantic import ConfigDict, JsonValue, TypeAdapter

ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)
ANSWER: Final = TypeAdapter(JsonValue)


class Message(TypedDict, total=False):
    role: str
    content: JsonValue


class Call(TypedDict):
    messages: list[Message]
    usage: JsonValue


@runtime_checkable
class Projection(Protocol):
    def translate_messages_to_responses_input(  # ast-grep-ignore: no-long-names
        self, messages: list[Message]
    ) -> JsonValue: ...

    def translate_responses_api_usage_to_anthropic_usage(  # ast-grep-ignore: no-long-names
        self, raw_usage: JsonValue
    ) -> JsonValue: ...


VIEW: Final[Projection] = TypeAdapter[Projection](Projection, config=ARBITRARY).validate_python(
    ResponsesAdapter()
)


def run(call: Call) -> JsonValue:
    answer: dict[str, JsonValue] = {
        "input": VIEW.translate_messages_to_responses_input(call["messages"]),
        "usage": VIEW.translate_responses_api_usage_to_anthropic_usage(call["usage"]),
    }
    return ANSWER.validate_json(json.dumps(answer))
