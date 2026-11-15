import json
from collections.abc import Collection, Sequence
from typing import Literal, NotRequired, Protocol, TypedDict

import fastuuid
from litellm.llms.bedrock.realtime.transformation import BedrockRealtimeConfig, BedrockUsageEvent
from litellm.types.llms.openai import OpenAIRealtimeResponseDelta
from pydantic import JsonValue, TypeAdapter

JsonObject = dict[str, JsonValue]
JsonAdapter: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

# LiteLLM's event TypedDicts carry Any, so this is the widest Any-free type they fit
EventList = Sequence[Collection[str]]


class OutputIds(TypedDict):
    response: str | None
    item: str | None
    conversation: str | None


class ReplyIds(TypedDict):
    response: str | None
    item: str | None


class UsageTokensWire(TypedDict):
    speechTokens: int
    textTokens: int


class UsageTotalWire(TypedDict):
    input: UsageTokensWire
    output: UsageTokensWire


class UsageDetailsWire(TypedDict):
    total: UsageTotalWire


class UsageWire(TypedDict):
    totalInputTokens: int
    totalOutputTokens: int
    totalTokens: int
    details: UsageDetailsWire


class ContentStartCall(TypedDict):
    method: Literal["content_start"]
    event: JsonObject
    ids: OutputIds


class UserSpeechCall(TypedDict):
    method: Literal["user_speech"]
    start: bool


class UserTranscriptCall(TypedDict):
    method: Literal["user_transcript"]
    text: str


class UserCompletedCall(TypedDict):
    method: Literal["user_completed"]


class TextOutputCall(TypedDict):
    method: Literal["text_output"]
    event: JsonObject
    ids: ReplyIds


class AudioOutputCall(TypedDict):
    method: Literal["audio_output"]
    event: JsonObject
    ids: ReplyIds


class UsageCall(TypedDict):
    method: Literal["usage"]
    usage: UsageWire


Call = (
    ContentStartCall
    | UserSpeechCall
    | UserTranscriptCall
    | UserCompletedCall
    | TextOutputCall
    | AudioOutputCall
    | UsageCall
)


class Input(TypedDict):
    ids: list[str]
    calls: list[Call]


class UsageTokenDetailsView(TypedDict):
    audio_tokens: int
    text_tokens: int
    cached_tokens: NotRequired[int]


class UsageView(TypedDict):
    input_tokens: int
    output_tokens: int
    total_tokens: int
    input_token_details: UsageTokenDetailsView
    output_token_details: UsageTokenDetailsView


class DoneResponseView(TypedDict):
    usage: UsageView


class DoneView(TypedDict):
    response: DoneResponseView


DoneAdapter: TypeAdapter[DoneView] = TypeAdapter(DoneView)


class RealtimeConfig(Protocol):
    def transform_content_start_event(
        self,
        event: JsonObject,
        current_response_id: str | None,
        current_output_item_id: str | None,
        current_conversation_id: str | None,
    ) -> tuple[EventList, str | None, str | None, str | None, str | None]: ...

    def transform_user_speech_event(self, *, is_speech_start: bool) -> EventList: ...

    def transform_user_transcript_event(  # ast-grep-ignore
        self, transcript: str
    ) -> EventList: ...

    def user_transcript_completed_events(self) -> EventList: ...  # ast-grep-ignore

    def transform_text_output_event(
        self,
        event: JsonObject,
        current_output_item_id: str | None,
        current_response_id: str | None,
        current_delta_chunks: list[OpenAIRealtimeResponseDelta] | None,
    ) -> tuple[EventList, list[OpenAIRealtimeResponseDelta] | None]: ...

    def transform_audio_output_event(
        self,
        event: JsonObject,
        current_output_item_id: str | None,
        current_response_id: str | None,
    ) -> EventList: ...

    def transform_usage_event(self, usage_event: BedrockUsageEvent) -> None: ...

    def leftover_usage_done_events(self) -> EventList: ...


CONFIG_TYPE: type[RealtimeConfig] = BedrockRealtimeConfig


class FixedFast:
    def __init__(self, values: list[str]) -> None:
        self.values = [fastuuid.UUID(value) for value in values]

    def __call__(self) -> fastuuid.UUID:
        return self.values.pop(0)


def events_view(events: EventList) -> list[JsonValue]:
    views: list[JsonValue] = []
    views.extend(JsonAdapter.validate_json(json.dumps(event)) for event in events)
    return views


def run_content_start(config: RealtimeConfig, call: ContentStartCall) -> JsonObject:
    ids = call["ids"]
    events, response, item, conversation, delta_type = config.transform_content_start_event(
        {"contentStart": call["event"]}, ids["response"], ids["item"], ids["conversation"]
    )
    return {
        "events": events_view(events),
        "response_id": response,
        "item_id": item,
        "conversation_id": conversation,
        "delta_type": delta_type,
    }


def run_text_output(
    config: RealtimeConfig,
    call: TextOutputCall,
    chunks: list[OpenAIRealtimeResponseDelta] | None,
) -> tuple[JsonObject, list[OpenAIRealtimeResponseDelta] | None]:
    ids = call["ids"]
    events, kept = config.transform_text_output_event(
        {"textOutput": call["event"]}, ids["item"], ids["response"], chunks
    )
    result: JsonObject = {
        "events": events_view(events),
        "delta_chunks": JsonAdapter.validate_json(json.dumps(kept)),
    }
    return result, kept


def run_usage(config: RealtimeConfig, call: UsageCall) -> JsonObject:
    config.transform_usage_event(BedrockUsageEvent.model_validate_json(json.dumps(call["usage"])))
    done_events = config.leftover_usage_done_events()
    usage: list[JsonValue] = []
    for event in done_events:
        done = DoneAdapter.validate_json(json.dumps(event))
        usage.append(JsonAdapter.validate_json(json.dumps(done["response"]["usage"])))
    return {"usage": usage}


def run(input_value: Input) -> JsonValue:
    fastuuid.uuid4 = FixedFast(input_value["ids"])
    config = CONFIG_TYPE()
    chunks: list[OpenAIRealtimeResponseDelta] | None = None
    results: list[JsonValue] = []
    for call in input_value["calls"]:
        if call["method"] == "content_start":
            results.append(run_content_start(config, call))
        elif call["method"] == "user_speech":
            speech = config.transform_user_speech_event(is_speech_start=call["start"])
            results.append({"events": events_view(speech)})
        elif call["method"] == "user_transcript":
            transcript = config.transform_user_transcript_event(call["text"])
            results.append({"events": events_view(transcript)})
        elif call["method"] == "user_completed":
            results.append({"events": events_view(config.user_transcript_completed_events())})
        elif call["method"] == "text_output":
            result, chunks = run_text_output(config, call, chunks)
            results.append(result)
        elif call["method"] == "audio_output":
            audio_ids = call["ids"]
            audio_events = config.transform_audio_output_event(
                {"audioOutput": call["event"]}, audio_ids["item"], audio_ids["response"]
            )
            results.append({"events": events_view(audio_events)})
        else:
            results.append(run_usage(config, call))
    return {"calls": results}
