import os
from typing import Final, Protocol, TypedDict, runtime_checkable

import litellm
from litellm.llms.anthropic.pass_through.adapters.transformation import (
    LiteLLMAnthropicMessagesAdapter as MessagesAdapter,
)
from pydantic import ConfigDict, JsonValue, TypeAdapter

ENV: Final = "LITELLM_REASONING_AUTO_SUMMARY"
ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)
ANSWER: Final = TypeAdapter(JsonValue)


class Adapter(MessagesAdapter):
    # the base __init__ has no annotations, so a typed subclass avoids an untyped-call suppression
    def __init__(self) -> None:
        ...


@runtime_checkable
class Translate(Protocol):
    # the request type carries Any, so the call enters through this JSON-facing signature
    def translate_anthropic_to_openai(
        self,
        anthropic_message_request: dict[str, JsonValue],
        *,
        custom_llm_provider: str | None = None,
        preserve_midturn_system: bool = False,
    ) -> tuple[dict[str, JsonValue], dict[str, str]]: ...


VIEW: Final[Translate] = TypeAdapter[Translate](Translate, config=ARBITRARY).validate_python(
    Adapter()
)


class Probe:
    def __init__(self, declared: list[str] | None) -> None:
        self.declared = declared
        self.calls: list[JsonValue] = []

    def __call__(self, model: str, custom_llm_provider: str | None = None) -> list[str] | None:
        entry: JsonValue = {"model": model, "provider": custom_llm_provider}
        self.calls.append(entry)
        return self.declared


class OptionsCall(TypedDict):
    request: dict[str, JsonValue]
    custom_llm_provider: str | None
    auto_summary: bool
    declared: list[str] | None


def options_answer(call: OptionsCall) -> dict[str, JsonValue]:
    probe = Probe(call["declared"])
    litellm.get_supported_openai_params = probe
    litellm.reasoning_auto_summary = call["auto_summary"]
    os.environ.pop(ENV, None)
    kwargs, _mapping = VIEW.translate_anthropic_to_openai(
        call["request"],
        custom_llm_provider=call["custom_llm_provider"],
        preserve_midturn_system=True,
    )
    return {
        "kwargs": kwargs,
        "request": call["request"],
        "probes": probe.calls,
    }


def run(input_value: OptionsCall) -> JsonValue:
    answer = options_answer(input_value)
    return ANSWER.validate_python(answer)
