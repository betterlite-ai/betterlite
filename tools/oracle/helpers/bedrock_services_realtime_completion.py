import json
from collections.abc import Mapping, Sequence
from typing import Literal, Protocol, TypeAlias, TypedDict

import fastuuid
from litellm.llms.bedrock.realtime.transformation import BedrockRealtimeConfig, BedrockUsageEvent
from litellm.types.llms.openai import OpenAIRealtimeResponseDelta
from pydantic import JsonValue, TypeAdapter

from bedrock_services_realtime_session import FixedFast

JsonObject = dict[str, JsonValue]
JsonAdapter: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
EventsAdapter: TypeAdapter[list[JsonValue]] = TypeAdapter(list[JsonValue])


class DoneResponse(TypedDict):
    usage: JsonValue


class DoneWire(TypedDict):
    response: DoneResponse


class CallWire(TypedDict):
    type: str
    event_id: str
    response_id: str
    item_id: str
    output_index: int
    call_id: str
    name: str
    arguments: str


DoneAdapter: TypeAdapter[list[DoneWire]] = TypeAdapter(list[DoneWire])
CallAdapter: TypeAdapter[list[CallWire]] = TypeAdapter(list[CallWire])


class StateStep(TypedDict):
    op: Literal["set_state"]
    output_item_id: str | None
    response_id: str | None
    conversation_id: str | None
    delta_type: str | None
    delta_chunks: list[str] | None


class UsageStep(TypedDict):
    op: Literal["usage"]
    usage: JsonObject


class ToolUseStep(TypedDict):
    op: Literal["tool_use"]
    event: JsonObject


class PlainStep(TypedDict):
    op: Literal["content_end", "prompt_end", "take_usage", "leftover"]


Step: TypeAlias = StateStep | UsageStep | ToolUseStep | PlainStep


class Input(TypedDict):
    ids: list[str]
    steps: list[Step]


class RealtimeConfig(Protocol):
    def transform_usage_event(self, usage_event: BedrockUsageEvent) -> None: ...

    def leftover_usage_done_events(self) -> Sequence[Mapping[str, object]]: ...

    def transform_content_end_event(
        self,
        event: dict[str, JsonValue],
        current_output_item_id: str | None,
        current_response_id: str | None,
        current_delta_type: str | None,
        current_delta_chunks: list[OpenAIRealtimeResponseDelta] | None,
    ) -> tuple[Sequence[Mapping[str, object]], list[OpenAIRealtimeResponseDelta] | None]: ...

    def transform_prompt_end_event(
        self,
        event: dict[str, JsonValue],
        current_response_id: str | None,
        current_conversation_id: str | None,
    ) -> tuple[Sequence[Mapping[str, object]], str | None, str | None, str | None]: ...

    def transform_tool_use_event(
        self,
        event: dict[str, JsonValue],
        current_output_item_id: str | None,
        current_response_id: str | None,
    ) -> tuple[Sequence[Mapping[str, object]], str, str]: ...


CONFIG_TYPE: type[RealtimeConfig] = BedrockRealtimeConfig


class Replay:
    def __init__(self, ids: list[str]) -> None:
        self.config = CONFIG_TYPE()
        fastuuid.uuid4 = FixedFast(ids)
        self.item_id: str | None = None
        self.response_id: str | None = None
        self.conversation_id: str | None = None
        self.delta_type: str | None = None
        self.chunks: list[OpenAIRealtimeResponseDelta] | None = None

    def set_state(self, step: StateStep) -> JsonValue:
        self.item_id = step["output_item_id"]
        self.response_id = step["response_id"]
        self.conversation_id = step["conversation_id"]
        self.delta_type = step["delta_type"]
        texts = step["delta_chunks"]
        self.chunks = (
            None
            if texts is None
            else [
                OpenAIRealtimeResponseDelta(
                    type="response.text.delta",
                    content_index=0,
                    delta=text,
                    event_id="chunk",
                    item_id="chunk",
                    output_index=0,
                    response_id="chunk",
                )
                for text in texts
            ]
        )
        return None

    def usage(self, step: UsageStep) -> JsonValue:
        self.config.transform_usage_event(BedrockUsageEvent.model_validate(step["usage"]))
        return None

    def content_end(self) -> JsonValue:
        events, chunks = self.config.transform_content_end_event(
            {"contentEnd": {}},
            self.item_id,
            self.response_id,
            self.delta_type,
            self.chunks,
        )
        self.chunks = chunks
        kept: list[JsonValue] | None = (
            None if chunks is None else [chunk["delta"] for chunk in chunks]
        )
        return {
            "events": EventsAdapter.validate_json(json.dumps(events)),
            "delta_chunks": kept,
        }

    def prompt_end(self) -> JsonValue:
        events, item_id, response_id, delta_type = self.config.transform_prompt_end_event(
            {"promptEnd": {}}, self.response_id, self.conversation_id
        )
        self.item_id = item_id
        self.response_id = response_id
        self.delta_type = delta_type
        return {
            "events": EventsAdapter.validate_json(json.dumps(events)),
            "output_item_id": item_id,
            "response_id": response_id,
            "delta_type": delta_type,
        }

    def take_usage(self) -> JsonValue:
        events, _item_id, _response_id, _delta_type = self.config.transform_prompt_end_event(
            {"promptEnd": {}}, "resp-take", "conv-take"
        )
        done = DoneAdapter.validate_json(json.dumps(events))
        return done[0]["response"]["usage"]

    def leftover(self) -> JsonValue:
        events = self.config.leftover_usage_done_events()
        return {"events": EventsAdapter.validate_json(json.dumps(events))}

    def tool_use(self, step: ToolUseStep) -> JsonValue:
        events, call_id, name = self.config.transform_tool_use_event(
            step["event"], self.item_id, self.response_id
        )
        wires = CallAdapter.validate_json(json.dumps(events))
        shown: list[JsonValue] = [
            {
                "type": wire["type"],
                "event_id": wire["event_id"],
                "response_id": wire["response_id"],
                "item_id": wire["item_id"],
                "output_index": wire["output_index"],
                "call_id": wire["call_id"],
                "name": wire["name"],
                "arguments": JsonAdapter.validate_json(wire["arguments"]),
            }
            for wire in wires
        ]
        return {"events": shown, "tool_call_id": call_id, "tool_name": name}


def run_plain(replay: Replay, step: PlainStep) -> JsonValue:
    if step["op"] == "content_end":
        return replay.content_end()
    if step["op"] == "prompt_end":
        return replay.prompt_end()
    if step["op"] == "take_usage":
        return replay.take_usage()
    return replay.leftover()


def run_step(replay: Replay, step: Step) -> JsonValue:
    if step["op"] == "set_state":
        return replay.set_state(step)
    if step["op"] == "usage":
        return replay.usage(step)
    if step["op"] == "tool_use":
        return replay.tool_use(step)
    return run_plain(replay, step)


def run(input_value: Input) -> JsonValue:
    replay = Replay(input_value["ids"])
    results = [run_step(replay, step) for step in input_value["steps"]]
    return {"results": results}
