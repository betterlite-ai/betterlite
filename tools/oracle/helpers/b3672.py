from types import MappingProxyType
from typing import Final, Literal, TypedDict

from litellm.llms.openai.responses.guardrail_translation.handler import (
    OpenAIResponsesHandler,
    _ToolCallShape as Shape,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/guardrail_translation/handler.py#L108-L110
)
from litellm.proxy.policy_engine.pipeline_executor import UndeliverableStreamRewrite
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)


class EventScenario(TypedDict):
    events: list[JsonValue]


class Rewrite(TypedDict):
    output_index: int
    content_index: int
    text: str


class SyncScenario(TypedDict):
    events: list[JsonValue]
    rewrites: list[Rewrite]


class SpreadScenario(TypedDict):
    events: list[JsonValue]
    rewritten_text: str
    guardrail_name: str


class ShapeInput(TypedDict):
    name: str | None
    arguments: str


class DeliverScenario(TypedDict):
    events: list[JsonValue]
    output: list[JsonValue]
    pre_guardrail_tool_calls: list[ShapeInput]
    post_guardrail_tool_calls: list[ShapeInput]
    guardrail_name: str


class EventCall(TypedDict):
    function: Literal["string_so_far"]
    scenarios: list[EventScenario]


class ScanKeyCall(TypedDict):
    function: Literal["scan_key"]
    scenarios: list[EventScenario]


class SyncCall(TypedDict):
    function: Literal["sync"]
    scenarios: list[SyncScenario]


class SpreadCall(TypedDict):
    function: Literal["spread"]
    scenarios: list[SpreadScenario]


class DeliverCall(TypedDict):
    function: Literal["deliver"]
    scenarios: list[DeliverScenario]


Input = EventCall | ScanKeyCall | SyncCall | SpreadCall | DeliverCall


def string_so_far_answer(handler: OpenAIResponsesHandler, scenario: EventScenario) -> JsonValue:
    text = handler.get_streaming_string_so_far(scenario["events"])
    return JSON.validate_python({"string": text})


def scan_key_answer(handler: OpenAIResponsesHandler, scenario: EventScenario) -> JsonValue:
    key = handler.get_streaming_scan_key(scenario["events"])
    if key is None:
        return JSON.validate_python({"scan_key": None})
    return JSON.validate_python(
        {
            "scan_key": {
                "texts": list(key.texts),
                "tool_calls": list(key.tool_calls),
                "stream_ended": key.stream_ended,
                "tool_calls_in_flight": key.tool_calls_in_flight,
            }
        }
    )


def sync_answer(handler: OpenAIResponsesHandler, scenario: SyncScenario) -> JsonValue:
    events = scenario["events"]
    rewrites: dict[tuple[int, int], str] = {}
    for rewrite in scenario["rewrites"]:
        rewrites[(rewrite["output_index"], rewrite["content_index"])] = rewrite["text"]
    sync_events = handler._sync_stream_events_with_rewrites  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/guardrail_translation/handler.py#L1111-L1145
    sync_events(stream_events=events, rewrites_by_position=MappingProxyType(rewrites))
    return JSON.validate_python({"events": events})


def spread_answer(handler: OpenAIResponsesHandler, scenario: SpreadScenario) -> JsonValue:
    events = scenario["events"]
    spread = handler._spread_text_rewrite_over_stream_events  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/guardrail_translation/handler.py#L1059-L1102
    error: str | None = None
    try:
        spread(
            stream_events=events,
            rewritten_text=scenario["rewritten_text"],
            guardrail_name=scenario["guardrail_name"],
        )
    except UndeliverableStreamRewrite as failure:
        error = str(failure)
    return JSON.validate_python({"events": events, "error": error})


def deliver_answer(handler: OpenAIResponsesHandler, scenario: DeliverScenario) -> JsonValue:
    events = scenario["events"]
    output = scenario["output"]
    pre = tuple(
        Shape(name=item["name"], arguments=item["arguments"])
        for item in scenario["pre_guardrail_tool_calls"]
    )
    post = tuple(
        Shape(name=item["name"], arguments=item["arguments"])
        for item in scenario["post_guardrail_tool_calls"]
    )
    deliver = handler._deliver_ended_stream_tool_call_rewrites  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/guardrail_translation/handler.py#L1161-L1247
    envelope: JsonValue = JSON.validate_python({"response": {"output": output}})
    error: str | None = None
    try:
        deliver(
            responses_so_far=[*events, envelope],
            outputs=output,
            pre_guardrail_tool_calls=pre,
            post_guardrail_tool_calls=post,
            guardrail_name=scenario["guardrail_name"],
        )
    except UndeliverableStreamRewrite as failure:
        error = str(failure)
    return JSON.validate_python({"events": events, "output": output, "error": error})


def answers(call: Input, handler: OpenAIResponsesHandler) -> list[JsonValue]:
    if call["function"] == "string_so_far":
        return [string_so_far_answer(handler, item) for item in call["scenarios"]]
    if call["function"] == "scan_key":
        return [scan_key_answer(handler, item) for item in call["scenarios"]]
    if call["function"] == "sync":
        return [sync_answer(handler, item) for item in call["scenarios"]]
    if call["function"] == "spread":
        return [spread_answer(handler, item) for item in call["scenarios"]]
    return [deliver_answer(handler, item) for item in call["scenarios"]]


def run(input_value: Input) -> JsonValue:
    handler = OpenAIResponsesHandler()
    return JSON.validate_python(answers(input_value, handler))
