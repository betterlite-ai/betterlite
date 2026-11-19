from typing import Final, Protocol, TypedDict, runtime_checkable

from litellm.llms.anthropic.pass_through.adapters.transformation import (
    LiteLLMAnthropicMessagesAdapter as MessagesAdapter,
)
from pydantic import ConfigDict, JsonValue, TypeAdapter

ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)
ANSWER: Final = TypeAdapter(JsonValue)


class Adapter(MessagesAdapter):
    # the base __init__ has no annotations, so a typed subclass avoids an untyped-call suppression
    def __init__(self) -> None:
        ...


@runtime_checkable
class Translate(Protocol):
    # the message TypedDicts carry Any, so the call enters through this JSON-facing signature
    def translate_anthropic_messages_to_openai(  # ast-grep-ignore: no-long-names
        self,
        messages: list[JsonValue],
        model: str | None = None,
        *,
        custom_llm_provider: str | None = None,
        preserve_midturn_system: bool = False,
    ) -> list[JsonValue]: ...


VIEW: Final[Translate] = TypeAdapter[Translate](Translate, config=ARBITRARY).validate_python(
    Adapter()
)


class ContentCall(TypedDict):
    messages: list[JsonValue]
    model: str | None
    custom_llm_provider: str | None
    preserve_midturn_system: bool
    supported: list[str]


def run(input_value: ContentCall) -> JsonValue:
    messages = VIEW.translate_anthropic_messages_to_openai(
        input_value["messages"],
        model=input_value["model"],
        custom_llm_provider=input_value["custom_llm_provider"],
        preserve_midturn_system=input_value["preserve_midturn_system"],
    )
    answer: dict[str, JsonValue] = {"messages": messages, "input": input_value["messages"]}
    return ANSWER.validate_python(answer)
