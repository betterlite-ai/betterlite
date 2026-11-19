import json
from typing import Final, Literal, Protocol, TypedDict, runtime_checkable

from litellm.llms.anthropic.pass_through.adapters.transformation import (
    AnthropicAdapter,
    LiteLLMAnthropicMessagesAdapter as MessagesAdapter,
)
from litellm.llms.anthropic.pass_through.context_management.result import PolyfillResult
from litellm.types.llms.anthropic import AppliedEdit, CompactionBlock, UsageIteration
from litellm.types.utils import ModelResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter

ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)
ANSWER: Final = TypeAdapter(JsonValue)
RESPONSE: Final = TypeAdapter(ModelResponse)
NO_MESSAGES: Final[list[dict[str, JsonValue]]] = []


@runtime_checkable
class InputView(Protocol):
    # ast-grep-ignore: no-long-names
    def translate_completion_input_params(self, kwargs: dict[str, JsonValue]) -> JsonValue: ...

    # ast-grep-ignore: no-long-names
    def translate_completion_input_params_with_tool_mapping(
        self, kwargs: dict[str, JsonValue], *, custom_llm_provider: str | None = None
    ) -> tuple[JsonValue, dict[str, str]]: ...


@runtime_checkable
class ComposeView(Protocol):
    # the case sends the request record as it was written, so unknown params still copy through
    def translate_anthropic_to_openai(
        self,
        anthropic_message_request: dict[str, JsonValue],
        *,
        custom_llm_provider: str | None = None,
        preserve_midturn_system: bool = False,
    ) -> tuple[JsonValue, dict[str, str]]: ...


@runtime_checkable
class OutputView(Protocol):
    # ast-grep-ignore: no-long-names
    def translate_completion_output_params(
        self,
        response: ModelResponse,
        tool_name_mapping: dict[str, str] | None = None,
        polyfill_result: PolyfillResult | None = None,
    ) -> JsonValue: ...


class Adapter(MessagesAdapter):
    # the base __init__ has no annotations, so a typed subclass avoids an untyped-call suppression
    def __init__(self) -> None:
        ...


INPUT_VIEW: Final[InputView] = TypeAdapter[InputView](
    InputView, config=ARBITRARY
).validate_python(AnthropicAdapter())
COMPOSE_VIEW: Final[ComposeView] = TypeAdapter[ComposeView](
    ComposeView, config=ARBITRARY
).validate_python(Adapter())
OUTPUT_VIEW: Final[OutputView] = TypeAdapter[OutputView](
    OutputView, config=ARBITRARY
).validate_python(AnthropicAdapter())


class PolyfillInput(TypedDict):
    applied_edits: list[AppliedEdit]
    compaction_block: CompactionBlock | None
    iterations_usage: list[UsageIteration] | None


class InputParamsCall(TypedDict):
    operation: Literal["input_params"]
    kwargs: dict[str, JsonValue]
    custom_llm_provider: str | None


class ConvenienceCall(TypedDict):
    operation: Literal["convenience"]
    kwargs: dict[str, JsonValue]
    custom_llm_provider: str | None


class ComposeCall(TypedDict):
    operation: Literal["compose"]
    request: dict[str, JsonValue]
    custom_llm_provider: str | None
    preserve_midturn_system: bool


class ResponseCall(TypedDict):
    operation: Literal["response"]
    response: dict[str, JsonValue]
    tool_name_mapping: dict[str, str] | None
    polyfill_result: PolyfillInput | None


Input = InputParamsCall | ConvenienceCall | ComposeCall | ResponseCall


def polyfill_result(value: PolyfillInput | None) -> PolyfillResult | None:
    if value is None:
        return None
    return PolyfillResult(
        messages=NO_MESSAGES,
        system=None,
        applied_edits=value["applied_edits"],
        compaction_block=value["compaction_block"],
        iterations_usage=value["iterations_usage"],
    )


def answer_input_params(call: InputParamsCall) -> JsonValue:
    kwargs: dict[str, JsonValue] = dict(call["kwargs"])
    request, mapping = INPUT_VIEW.translate_completion_input_params_with_tool_mapping(
        kwargs, custom_llm_provider=call["custom_llm_provider"]
    )
    payload: dict[str, JsonValue] = {
        "request": request,
        "tool_name_mapping": mapping,
        "kwargs": kwargs,
    }
    return ANSWER.validate_json(json.dumps(payload))


def answer_convenience(call: ConvenienceCall) -> JsonValue:
    kwargs: dict[str, JsonValue] = dict(call["kwargs"])
    request = INPUT_VIEW.translate_completion_input_params(kwargs)
    payload: dict[str, JsonValue] = {
        "request": request,
        "tool_name_mapping": None,
        "kwargs": kwargs,
    }
    return ANSWER.validate_json(json.dumps(payload))


def answer_compose(call: ComposeCall) -> JsonValue:
    request: dict[str, JsonValue] = dict(call["request"])
    translated, mapping = COMPOSE_VIEW.translate_anthropic_to_openai(
        request,
        custom_llm_provider=call["custom_llm_provider"],
        preserve_midturn_system=call["preserve_midturn_system"],
    )
    payload: dict[str, JsonValue] = {
        "request": translated,
        "tool_name_mapping": mapping,
        "anthropic_request": request,
    }
    return ANSWER.validate_json(json.dumps(payload))


def answer_response(call: ResponseCall) -> JsonValue:
    translated = OUTPUT_VIEW.translate_completion_output_params(
        RESPONSE.validate_python(call["response"]),
        tool_name_mapping=call["tool_name_mapping"],
        polyfill_result=polyfill_result(call["polyfill_result"]),
    )
    return ANSWER.validate_json(json.dumps(translated))


def run(input_value: Input) -> JsonValue:
    match input_value["operation"]:
        case "input_params":
            return answer_input_params(input_value)
        case "convenience":
            return answer_convenience(input_value)
        case "compose":
            return answer_compose(input_value)
        case "response":
            return answer_response(input_value)
