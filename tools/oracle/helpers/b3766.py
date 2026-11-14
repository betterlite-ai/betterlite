import asyncio
from typing import Literal, NotRequired, Required, TypedDict

import httpx
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
from pydantic import JsonValue, TypeAdapter, ValidationError
from starlette.exceptions import HTTPException


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


class Sent(TypedDict, total=False):
    texts: list[str]
    tools: JsonValue
    tool_calls: JsonValue
    structured_messages: JsonValue
    model: str | None


class Returned(TypedDict, total=False):
    texts: list[str]
    tools: JsonValue
    tool_calls: JsonValue
    structured_messages: JsonValue


BACK = TypeAdapter(Returned)


class Answer(TypedDict):
    status: int
    body: JsonValue
    headers: dict[str, str]


class Input(TypedDict):
    function: str
    settings: Required[SettingsFields]
    inputs: Required[Sent]
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


def apply(args: Input) -> ApplyOutput:
    handler = make_handler(args["settings"])
    recorder = Recorder(args.get("answer"))
    handler.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(recorder.handle))
    data = dict(args["request_data"])
    inputs = args["inputs"]
    kind = args["input_type"]
    outcome: AllowOutcome | HttpOutcome | PlainOutcome
    result: Projection | None = None
    try:
        out = BACK.validate_python(asyncio.run(handler.apply_guardrail(inputs, data, kind)))  # type: ignore[misc, arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4536
    except HTTPException as error:
        outcome = {"tag": "http_error", "status": error.status_code}
    except ValidationError:
        outcome = {"tag": "parse_error"}
    except httpx.HTTPError:
        outcome = {"tag": "internal_error"}
    else:
        outcome = {"tag": "allow"}
        result = {
            "texts": out.get("texts"),
            "tools": out.get("tools"),
            "tool_calls": out.get("tool_calls"),
            "structured_messages": out.get("structured_messages"),
        }
    path = recorder.paths[0] if recorder.paths else None
    request = recorder.bodies[0] if recorder.bodies else None
    return {
        "path": path,
        "request": request,
        "outcome": outcome,
        "result": result,
        "applied_guardrails": applied_guardrails(data),
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
        "event_hooks": [hook.value for hook in handler.get_supported_event_hooks()],
    }


def streaming_params(args: Input) -> StreamingFields:
    handler = make_handler(args["settings"])
    fields: dict[str, JsonValue | OptionalParams] = dict(args["params"])
    nested = args["params"].get("optional_params")
    if nested is not None:
        fields["optional_params"] = OptionalParams.model_validate(nested)
    handler.update_in_memory_litellm_params(LitellmParams.model_validate(fields))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L1192
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
