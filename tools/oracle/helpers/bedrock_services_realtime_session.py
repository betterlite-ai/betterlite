import json
import uuid
from datetime import UTC, datetime
from typing import Literal, NotRequired, Protocol, Required, TypedDict
from uuid import UUID

import fastuuid
import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.realtime.transformation import BedrockRealtimeConfig
from pydantic import JsonValue, TypeAdapter
from typing_extensions import ReadOnly

JsonObject = dict[str, JsonValue]
Frames = TypeAdapter(list[JsonObject])
JsonAdapter: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class SessionView(TypedDict):
    id: ReadOnly[Required[str]]
    modalities: ReadOnly[NotRequired[list[str]]]
    model: ReadOnly[NotRequired[str]]


class EventView(TypedDict):
    event_id: ReadOnly[str]
    type: ReadOnly[Literal["session.created", "session.updated"]]
    session: ReadOnly[SessionView]


class InputSchemaWire(TypedDict):
    json: str


class ToolSpecWire(TypedDict):
    name: str
    description: str
    inputSchema: InputSchemaWire


class ToolConfigWire(TypedDict):
    tools: list[ToolSpecWire]


class PromptStartWire(TypedDict):
    promptName: str
    toolUseOutputConfiguration: NotRequired[JsonObject]
    toolConfiguration: NotRequired[ToolConfigWire]


class PromptStartBody(TypedDict):
    promptStart: PromptStartWire


class PromptStartEnvelope(TypedDict):
    event: PromptStartBody


PromptStartAdapter: TypeAdapter[PromptStartEnvelope] = TypeAdapter(PromptStartEnvelope)


class ErrorRun(TypedDict):
    message: str
    status: int
    headers: list[tuple[str, str]]


class Input(TypedDict):
    case: Literal[
        "default_session_configuration",
        "session_update_options",
        "pcm16_input_output_rates",
        "g711_rates",
        "explicit_sample_rate",
        "tool_configuration",
        "system_instructions",
        "created_updated_events",
    ]
    ids: list[str]
    model: str
    trace_id: str
    updates: list[JsonObject]
    urls: list[str | None]
    modalities: list[str] | None
    models: list[str]
    headers: list[tuple[str, str]]
    error: ErrorRun | None


class ErrorView(Protocol):
    status_code: int
    status_code_is_synthesized: bool


class RealtimeConfig(Protocol):
    prompt_name: str
    content_name: str
    audio_content_name: str
    prompt_started: bool
    client_audio_streamed: bool

    def session_configuration_request(
        self, model: str, tools: list[dict[str, JsonValue]] | None = None
    ) -> str: ...

    def transform_session_update_event(self, json_message: dict[str, JsonValue]) -> list[str]: ...

    def session_created_event(self, model: str, logging_obj: Logging) -> EventView: ...

    def session_updated_event(
        self, model: str, logging_obj: Logging, modalities: list[str] | None = None
    ) -> EventView: ...

    def get_complete_url(
        self, api_base: str | None, model: str, api_key: str | None = None
    ) -> str: ...

    def requires_session_configuration(self) -> bool: ...

    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: str | None = None
    ) -> dict[str, str]: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: httpx.Headers
    ) -> ErrorView: ...


CONFIG_TYPE: type[RealtimeConfig] = BedrockRealtimeConfig


class FixedStd:
    def __init__(self, values: list[str]) -> None:
        self.values = [UUID(value) for value in values]
        self.index = 0

    def __call__(self) -> UUID:
        value = self.values[self.index]
        self.index += 1
        return value


class FixedFast:
    def __init__(self, values: list[str]) -> None:
        self.values = [fastuuid.UUID(value) for value in values]
        self.index = 0

    def __call__(self) -> fastuuid.UUID:
        value = self.values[self.index]
        self.index += 1
        return value


def make_logging(trace_id: str) -> Logging:
    messages: list[dict[str, str]] = []
    return Logging(
        model="realtime",
        messages=messages,
        stream=True,
        call_type="realtime",
        start_time=datetime(2020, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="function-1",
        litellm_trace_id=trace_id,
    )


def frame_list(messages: list[str]) -> list[JsonValue]:
    frames = Frames.validate_json(json.dumps(messages))
    return list(frames)


def apply_updates(config: RealtimeConfig, updates: list[JsonObject]) -> list[JsonValue]:
    results: list[JsonValue] = []
    for message in updates:
        frames = frame_list(config.transform_session_update_event(message))
        results.append({"frames": frames})
    return results


def prompt_start_tools(frame: JsonValue) -> JsonObject:
    envelope = PromptStartAdapter.validate_json(json.dumps(frame))
    prompt_start = envelope["event"]["promptStart"]
    result: JsonObject = {"prompt_name": prompt_start["promptName"]}
    if "toolUseOutputConfiguration" in prompt_start:
        result["tool_use_output_configuration"] = prompt_start["toolUseOutputConfiguration"]
    if "toolConfiguration" in prompt_start:
        tools: list[JsonValue] = []
        for tool in prompt_start["toolConfiguration"]["tools"]:
            schema = JsonAdapter.validate_json(tool["inputSchema"]["json"])
            tools.append(
                {"name": tool["name"], "description": tool["description"], "schema": schema}
            )
        result["tools"] = tools
    return result


def tool_updates(config: RealtimeConfig, updates: list[JsonObject]) -> list[JsonValue]:
    results: list[JsonValue] = []
    for message in updates:
        frames = frame_list(config.transform_session_update_event(message))
        results.append({"prompt_start": prompt_start_tools(frames[1])})
    return results


def session_view(event: EventView) -> JsonObject:
    session = event["session"]
    carried = session.get("modalities")
    if carried is None:
        raise ValueError("the session event carries no modalities")
    modalities: list[JsonValue] = list(carried)
    view: JsonObject = {"id": session["id"], "modalities": modalities}
    if "model" in session:
        view["model"] = session["model"]
    return view


def event_view(event: EventView) -> JsonObject:
    return {
        "event_id": event["event_id"],
        "type": event["type"],
        "session": session_view(event),
    }


def validated_headers(
    config: RealtimeConfig, model: str, headers: list[tuple[str, str]]
) -> list[JsonValue]:
    results: list[JsonValue] = []
    for header in headers:
        echoed = config.validate_environment(dict([header]), model)
        pair: list[JsonValue] = [header[0], echoed[header[0]]]
        results.append(pair)
    return results


def run_default(input_value: Input) -> JsonValue:
    uuid.uuid4 = FixedStd(input_value["ids"])
    config = CONFIG_TYPE()
    marker = config.session_configuration_request(input_value["model"])
    configuration = JsonAdapter.validate_json(marker)
    return {
        "prompt_name": config.prompt_name,
        "content_name": config.content_name,
        "audio_content_name": config.audio_content_name,
        "prompt_started": config.prompt_started,
        "client_audio_streamed": config.client_audio_streamed,
        "configuration": configuration,
        "prompt_started_after": config.prompt_started,
    }


def run_created_updated(input_value: Input) -> JsonValue:
    config = CONFIG_TYPE()
    model = input_value["model"]
    logging_obj = make_logging(input_value["trace_id"])
    fastuuid.uuid4 = FixedFast(input_value["ids"])
    created = config.session_created_event(model, logging_obj)
    updated_default = config.session_updated_event(model, logging_obj)
    updated_empty = config.session_updated_event(model, logging_obj, [])
    updated_explicit = config.session_updated_event(
        model, logging_obj, input_value["modalities"]
    )
    model_event = config.session_created_event(input_value["models"][0], logging_obj)
    error_input = input_value["error"]
    if error_input is None:
        raise ValueError("the created_updated_events case needs an error fixture")
    error = config.get_error_class(
        error_input["message"],
        error_input["status"],
        httpx.Headers(error_input["headers"]),
    )
    urls: list[JsonValue] = [
        config.get_complete_url(url, model) for url in input_value["urls"]
    ]
    return {
        "created": event_view(created),
        "updated_default": event_view(updated_default),
        "updated_empty": event_view(updated_empty),
        "updated_explicit": event_view(updated_explicit),
        "model_event": event_view(model_event),
        "requires_session_configuration": config.requires_session_configuration(),
        "complete_urls": urls,
        "validated_headers": validated_headers(config, model, input_value["headers"]),
        "error_status": error.status_code,
        "error_synthesized": error.status_code_is_synthesized,
    }


def run(input_value: Input) -> JsonValue:
    case = input_value["case"]
    if case == "created_updated_events":
        return run_created_updated(input_value)
    if case == "default_session_configuration":
        return run_default(input_value)
    uuid.uuid4 = FixedStd(input_value["ids"])
    config = CONFIG_TYPE()
    if case == "tool_configuration":
        results = tool_updates(config, input_value["updates"])
    else:
        results = apply_updates(config, input_value["updates"])
    return {
        "updates": results,
        "prompt_name": config.prompt_name,
        "prompt_started": config.prompt_started,
        "client_audio_streamed": config.client_audio_streamed,
    }
