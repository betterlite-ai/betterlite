import asyncio
from typing import Literal, NotRequired, Protocol, Required, TypedDict

import httpx
from fastapi import HTTPException
from litellm.proxy.guardrails.guardrail_hooks.crowdstrike_aidr.crowdstrike_aidr import (
    CrowdStrikeAIDRGuardrailMissingSecrets as MissingSecrets,
)
from litellm.proxy.guardrails.guardrail_hooks.crowdstrike_aidr.crowdstrike_aidr import (
    CrowdStrikeAIDRHandler,
)
from litellm.types.guardrails import LitellmParams
from litellm.types.proxy.guardrails.guardrail_hooks.crowdstrike_aidr import (
    CrowdStrikeAIDRGuardrailConfigModelOptionalParams as OptionalParams,
)
from litellm.types.utils import GenericGuardrailAPIInputs
from pydantic import JsonValue, TypeAdapter, ValidationError


class GuardrailEntry(Protocol):
    async def apply_guardrail(
        self,
        inputs: GenericGuardrailAPIInputs,
        request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"],
        logging_obj: None = None,
    ) -> GenericGuardrailAPIInputs: ...


class SettingsFields(TypedDict, total=False):
    guardrail_name: Required[str]
    api_key: str
    api_base: str
    fail_on_error: bool
    timeout: float
    buffer_until_moderated: bool
    buffer_release_on_scan: bool
    end_of_stream_only: bool
    sampling_rate: int


class Answer(TypedDict):
    status: int
    body: JsonValue
    headers: dict[str, str]


class Input(TypedDict):
    function: str
    settings: Required[SettingsFields]
    inputs: Required[GenericGuardrailAPIInputs]
    request_data: Required[dict[str, JsonValue]]
    input_type: Required[Literal["request", "response"]]
    params: Required[dict[str, JsonValue]]
    environment: NotRequired[dict[str, str]]
    answer: NotRequired[Answer]


class StreamingFields(TypedDict):
    buffer_until_moderated: bool
    buffer_release_on_scan: bool
    end_of_stream_only: bool
    sampling_rate: int


class Construction(TypedDict):
    api_key: str | None
    api_base: str | None
    fail_on_error: bool
    streaming: StreamingFields
    event_hooks: list[str]


class Projection(TypedDict):
    texts: list[str] | None
    tools: JsonValue
    tool_calls: JsonValue
    structured_messages: JsonValue


class AllowOutcome(TypedDict):
    tag: str


class HttpOutcome(TypedDict):
    tag: str
    status: int


class PlainOutcome(TypedDict):
    tag: str


class MissingSecret(TypedDict):
    tag: str


class ApplyOutput(TypedDict):
    path: str | None
    request: JsonValue
    outcome: AllowOutcome | HttpOutcome | PlainOutcome
    result: Projection | None
    applied_guardrails: list[str]


class Recorder:
    def __init__(self, answer: Answer | None) -> None:
        self.answer = answer
        self.paths: list[str] = []
        self.bodies: list[JsonValue] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.paths.append(request.url.path)
        self.bodies.append(TypeAdapter(JsonValue).validate_json(request.content))
        if self.answer is None:
            raise httpx.ReadTimeout("the mock never answers", request=request)
        return httpx.Response(self.answer["status"], json=self.answer["body"])


def make_handler(settings: SettingsFields) -> CrowdStrikeAIDRHandler:
    return CrowdStrikeAIDRHandler(
        guardrail_name=settings["guardrail_name"],
        api_key=settings.get("api_key"),
        api_base=settings.get("api_base"),
        fail_on_error=settings.get("fail_on_error"),
        streaming_buffer_until_moderated=settings.get("buffer_until_moderated"),
        streaming_buffer_release_on_scan=settings.get("buffer_release_on_scan"),
        streaming_end_of_stream_only=settings.get("end_of_stream_only"),
        streaming_sampling_rate=settings.get("sampling_rate"),
    )


def streaming_fields(handler: CrowdStrikeAIDRHandler) -> StreamingFields:
    return {
        "buffer_until_moderated": handler.streaming_buffer_until_moderated,
        "buffer_release_on_scan": handler.streaming_buffer_release_on_scan,
        "end_of_stream_only": handler.streaming_end_of_stream_only,
        "sampling_rate": handler.streaming_sampling_rate,
    }


def project(result: GenericGuardrailAPIInputs) -> Projection:
    projected = TypeAdapter(Projection).validate_python(result)
    return {
        "texts": projected.get("texts"),
        "tools": projected.get("tools"),
        "tool_calls": projected.get("tool_calls"),
        "structured_messages": projected.get("structured_messages"),
    }


class Bucket(TypedDict, total=False):
    applied_guardrails: list[str]


class Envelope(TypedDict, total=False):
    metadata: Bucket
    litellm_metadata: Bucket


def applied_guardrails(request_data: dict[str, JsonValue]) -> list[str]:
    envelope = TypeAdapter(Envelope).validate_python(request_data)
    bucket = envelope.get("litellm_metadata")
    if bucket is None:
        bucket = envelope.get("metadata")
    if bucket is None:
        return []
    names = bucket.get("applied_guardrails")
    if names is None:
        return []
    return names


async def run_entry(
    entry: GuardrailEntry,
    inputs: GenericGuardrailAPIInputs,
    request_data: dict[str, JsonValue],
    input_type: Literal["request", "response"],
) -> GenericGuardrailAPIInputs:
    call = entry.apply_guardrail
    return await call(inputs, request_data, input_type, None)


def apply(args: Input) -> ApplyOutput:
    handler = make_handler(args["settings"])
    recorder = Recorder(args.get("answer"))
    handler.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(recorder.handle))
    request_data = dict(args["request_data"])
    outcome: AllowOutcome | HttpOutcome | PlainOutcome
    result: Projection | None = None
    try:
        returned = asyncio.run(run_entry(handler, args["inputs"], request_data, args["input_type"]))
    except HTTPException as error:
        outcome = {"tag": "http_error", "status": error.status_code}
    except ValidationError:
        outcome = {"tag": "parse_error"}
    except httpx.HTTPError:
        outcome = {"tag": "internal_error"}
    else:
        outcome = {"tag": "allow"}
        result = project(returned)
    path = recorder.paths[0] if recorder.paths else None
    request = recorder.bodies[0] if recorder.bodies else None
    return {
        "path": path,
        "request": request,
        "outcome": outcome,
        "result": result,
        "applied_guardrails": applied_guardrails(request_data),
    }


def construct(args: Input) -> Construction | MissingSecret:
    try:
        handler = make_handler(args["settings"])
    except MissingSecrets:
        return {"tag": "missing_secret"}
    return {
        "api_key": handler.api_key,
        "api_base": handler.api_base,
        "fail_on_error": handler.fail_on_error,
        "streaming": streaming_fields(handler),
        "event_hooks": [hook.value for hook in CrowdStrikeAIDRHandler.get_supported_event_hooks()],
    }


def litellm_params(value: dict[str, JsonValue]) -> LitellmParams:
    nested = value.get("optional_params")
    if nested is None:
        return LitellmParams.model_validate(value)
    model = OptionalParams.model_validate(nested)
    merged = {**value, "optional_params": model}
    return LitellmParams.model_validate(merged)


def streaming_params(args: Input) -> StreamingFields:
    handler = make_handler(args["settings"])
    handler.update_in_memory_litellm_params(litellm_params(args["params"]))
    return streaming_fields(handler)


def run(input_value: Input) -> ApplyOutput | Construction | MissingSecret | StreamingFields:
    function = input_value["function"]
    if function == "apply":
        return apply(input_value)
    if function == "construct":
        return construct(input_value)
    if function == "streaming_params":
        return streaming_params(input_value)
    raise ValueError(f"unsupported function {function}")
