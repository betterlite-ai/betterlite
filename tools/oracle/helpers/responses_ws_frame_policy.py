import asyncio
import json
from collections.abc import Container, Mapping
from datetime import datetime
from typing import Final, Literal, TypeAlias, TypedDict

import litellm
from litellm.exceptions import RateLimitError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import UserAPIKeyAuth
from litellm.responses.streaming_iterator import (
    RESPONSES_WS_LOGGED_EVENT_TYPES,
    RESPONSES_WS_MASKABLE_TEXT_BLOCK_TYPES,
    ProjectQuotaCallback,
    ResponsesWebSocketStreaming,
)
from litellm.types.guardrails import PresidioPerRequestConfig
from litellm.types.responses.streaming_websocket import (
    PresidioGuardrailCallback,
    ResponsesBackendWebSocket,
    ResponsesClientWebSocket,
    ResponsesWebSocketRequestDefaults,
)
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
# the websocket handler takes object-valued maps, so a case's JSON map crosses the boundary here
OBJECTS: Final = TypeAdapter(dict[str, object])
QUOTA_MESSAGE: Final = "quota exceeded"
# metadata under which the relay wraps an output item's encrypted_content, so a set probe can see membership
AFFINITY: Final[dict[str, JsonValue]] = {"encrypted_content_affinity_enabled": True, "model_info": {"id": "m"}}

Verdict: TypeAlias = Literal["accept", "rate_limit", "fail"]


class Defaults(TypedDict):
    fill_missing: dict[str, JsonValue]
    overrides: dict[str, JsonValue]


class QuotaStep(TypedDict):
    kind: Literal["quota"]
    message: str
    authorized_model: str | None
    verdicts: list[Verdict]
    refuse_send: bool


class CreateStep(TypedDict):
    kind: Literal["create"]
    message: str
    authorized_model: str | None
    defaults: Defaults | None
    guardrails: list[dict[str, str]]
    request_data: dict[str, JsonValue]


class UnmaskStep(TypedDict):
    kind: Literal["unmask"]
    message: str
    guardrails: int
    request_data: dict[str, JsonValue]


class CompletedStep(TypedDict):
    kind: Literal["completed"]
    message: str
    guardrails: list[dict[str, str]]
    request_data: dict[str, JsonValue]


class FailureStep(TypedDict):
    kind: Literal["failure"]
    events: list[dict[str, JsonValue]]


class WrapStep(TypedDict):
    kind: Literal["wrap"]
    litellm_metadata: JsonValue
    event: dict[str, JsonValue]


class SetsStep(TypedDict):
    kind: Literal["sets"]
    candidates: list[str]


Step: TypeAlias = QuotaStep | CreateStep | UnmaskStep | CompletedStep | FailureStep | WrapStep | SetsStep


class Input(TypedDict):
    steps: list[Step]


class RejectionError(TypedDict):
    type: str
    message: str


class Rejection(TypedDict):
    type: str
    error: RejectionError


class Status(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int


REJECTION: Final = TypeAdapter(Rejection)


class BrokenQuotaError(Exception):
    pass


class Quota(ProjectQuotaCallback):
    def __init__(self, index: int, verdict: Verdict, calls: list[JsonValue]) -> None:
        self.index = index
        self.verdict: Verdict = verdict
        self.calls = calls

    @override
    async def enforce_project_io_token_quota_for_frame(
        self,
        user_api_key_dict: UserAPIKeyAuth | None,
        requested_model: str | None,
        estimated_input_tokens: int,
        estimated_output_tokens: int,
    ) -> None:
        user = None if user_api_key_dict is None else user_api_key_dict.user_id
        # a bool max_output_tokens reaches the callback as True, which a quota counts as 1
        self.calls.append(
            [self.index, user, requested_model, int(estimated_input_tokens), int(estimated_output_tokens)]
        )
        if self.verdict == "rate_limit":
            raise RateLimitError(QUOTA_MESSAGE, llm_provider="", model="")
        if self.verdict == "fail":
            raise BrokenQuotaError(QUOTA_MESSAGE)


class Unmask:
    def __init__(self, index: int, calls: list[JsonValue]) -> None:
        self.index = index
        self.calls = calls

    def __call__(self, text: str, pii_tokens: Mapping[str, str]) -> str:
        self.calls.append([self.index, text])
        unmasked = text
        for token, original in pii_tokens.items():
            unmasked = unmasked.replace(token, original)
        return unmasked


class Guard(PresidioGuardrailCallback):
    def __init__(self, index: int, replacements: dict[str, str], calls: list[JsonValue]) -> None:
        self.index = index
        self.replacements = replacements
        self.calls = calls
        # the relay reads the unmasker by this attribute name off the first guardrail
        self._unmask_pii_text = Unmask(index, calls)

    @override
    def get_presidio_settings_from_request_data(self, data: Container[str]) -> PresidioPerRequestConfig | None:
        return None

    @override
    async def check_pii(
        self,
        text: str,
        output_parse_pii: bool,
        presidio_config: PresidioPerRequestConfig | None,
        request_data: Container[str],
    ) -> str:
        self.calls.append([self.index, text, output_parse_pii, "metadata" in request_data])
        masked = text
        for found, replacement in self.replacements.items():
            masked = masked.replace(found, replacement)
        return masked


class Socket(ResponsesClientWebSocket):
    def __init__(self, *, refuse: bool) -> None:
        self.refuse = refuse
        self.sent: list[str] = []

    @override
    async def send_text(self, data: str) -> None:
        self.sent.append(data)
        if self.refuse:
            raise RuntimeError("the case asks the socket to refuse the frame")

    @override
    async def receive_text(self) -> str:
        await asyncio.sleep(0)
        return ""


class Backend(ResponsesBackendWebSocket):
    @override
    async def recv(self, decode: bool = True) -> str | bytes:
        await asyncio.sleep(0)
        return ""

    @override
    async def send(self, message: str) -> None:
        await asyncio.sleep(0)

    @override
    async def close(self) -> None:
        await asyncio.sleep(0)


class Probe(ResponsesWebSocketStreaming):
    async def quota_view(self, socket: Socket, calls: list[JsonValue], message: str) -> JsonValue:
        result: JsonValue
        try:
            result = await self._enforce_or_reject_frame(message)
        except BrokenQuotaError:
            result = "error"
        error_text = str(RateLimitError(QUOTA_MESSAGE, llm_provider="", model=""))
        sent = [REJECTION.validate_json(text) for text in socket.sent]
        frames: list[JsonValue] = [
            {"type": frame["type"], "error_type": frame["error"]["type"],
             "message_is_error_text": frame["error"]["message"] == error_text}
            for frame in sent
        ]
        return {"result": result, "calls": calls, "frames": frames}

    async def create_view(self, message: str, calls: list[JsonValue]) -> JsonValue:
        try:
            masked = await self._mask_response_create(message)
        except AttributeError:
            return self.frame_view(None, message, calls)
        return self.frame_view(masked, message, calls)

    def unmask_view(self, message: str, calls: list[JsonValue]) -> JsonValue:
        try:
            unmasked = self._unmask_response_event(message)
        except AttributeError:
            return self.frame_view(None, message, calls)
        return self.frame_view(unmasked, message, calls)

    async def completed_view(self, message: str, calls: list[JsonValue]) -> JsonValue:
        try:
            masked = await self._mask_response_completed(message)
        except AttributeError:
            return self.frame_view(None, message, calls)
        return self.frame_view(masked, message, calls)

    # same tells apart the very string LiteLLM was given from an equal one it serialized again
    def frame_view(self, output: str | None, message: str, calls: list[JsonValue]) -> JsonValue:
        metadata = JSON.validate_python(self.request_data.get("metadata"))
        if output is None:
            return {"error": True, "same": False, "frame": None, "calls": calls, "metadata": metadata}
        same = output is message
        frame = None if same else JSON.validate_json(output)
        return {"error": False, "same": same, "frame": frame, "calls": calls, "metadata": metadata}

    def failure_status(self, event: dict[str, JsonValue]) -> JsonValue:
        self.messages.clear()
        self.messages.append(OBJECTS.validate_python(event))
        failure = self._failure_exception()
        return None if failure is None else Status.model_validate(failure).status_code

    def wrap_view(self, event: dict[str, JsonValue]) -> JsonValue:
        text = json.dumps(event)
        wrapped = self._wrap_response_event(text)
        return None if wrapped is text else JSON.validate_json(wrapped)

    def sets_view(self, candidates: list[str]) -> JsonValue:
        failure = [kind for kind in candidates if self.failure_status({"type": kind}) is not None]
        item: dict[str, JsonValue] = {"encrypted_content": "x"}
        output_item = [kind for kind in candidates if self.wrap_view({"type": kind, "item": item}) is not None]
        return JSON.validate_python({"logged": RESPONSES_WS_LOGGED_EVENT_TYPES,
                                     "maskable": sorted(RESPONSES_WS_MASKABLE_TEXT_BLOCK_TYPES),
                                     "failure": failure, "output_item": output_item,
                                     "delta": sorted(self._DELTA_EVENT_TYPES),
                                     "done": sorted(self._OUTPUT_DONE_EVENT_TYPES)})


def connect(
    socket: Socket,
    request_data: dict[str, JsonValue],
    guardrails: list[Guard],
    authorized_model: str | None,
) -> Probe:
    log_obj = Logging(model="gpt-4o", messages=None, stream=True, call_type="responses",
                      start_time=datetime.fromisoformat("2026-01-01T00:00:00+00:00"),
                      litellm_call_id="call-1", function_id="fn-1")
    return Probe(websocket=socket, backend_ws=Backend(), logging_obj=log_obj,
                 request_data=OBJECTS.validate_python(request_data), guardrail_callbacks=guardrails,
                 authorized_model=authorized_model)


async def quota_answer(step: QuotaStep) -> JsonValue:
    calls: list[JsonValue] = []
    socket = Socket(refuse=step["refuse_send"])
    probe = connect(socket, {}, [], step["authorized_model"])
    probe.user_api_key_dict = UserAPIKeyAuth(user_id="u-1")
    probe.quota_callbacks = tuple(Quota(index, verdict, calls) for index, verdict in enumerate(step["verdicts"]))
    return await probe.quota_view(socket, calls, step["message"])


async def create_answer(step: CreateStep) -> JsonValue:
    calls: list[JsonValue] = []
    inputs = [Guard(index, pairs, calls) for index, pairs in enumerate(step["guardrails"])]
    probe = connect(Socket(refuse=False), step["request_data"], inputs, step["authorized_model"])
    defaults = step["defaults"]
    if defaults is not None:
        probe.request_defaults = ResponsesWebSocketRequestDefaults(
            fill_missing=OBJECTS.validate_python(defaults["fill_missing"]),
            overrides=OBJECTS.validate_python(defaults["overrides"]),
        )
    return await probe.create_view(step["message"], calls)


async def completed_answer(step: CompletedStep) -> JsonValue:
    calls: list[JsonValue] = []
    probe = connect(Socket(refuse=False), step["request_data"], [], None)
    outputs: list[PresidioGuardrailCallback] = [
        Guard(index, pairs, calls) for index, pairs in enumerate(step["guardrails"])
    ]
    probe.output_guardrail_callbacks = outputs
    return await probe.completed_view(step["message"], calls)


def local_answer(step: FailureStep | WrapStep | SetsStep) -> JsonValue:
    if step["kind"] == "failure":
        probe = connect(Socket(refuse=False), {}, [], None)
        return [probe.failure_status(event) for event in step["events"]]
    if step["kind"] == "wrap":
        probe = connect(Socket(refuse=False), {"litellm_metadata": step["litellm_metadata"]}, [], None)
        return probe.wrap_view(step["event"])
    probe = connect(Socket(refuse=False), {"litellm_metadata": AFFINITY}, [], None)
    return probe.sets_view(step["candidates"])


async def answer(step: Step) -> JsonValue:
    if step["kind"] == "quota":
        return await quota_answer(step)
    if step["kind"] == "create":
        return await create_answer(step)
    if step["kind"] == "completed":
        return await completed_answer(step)
    if step["kind"] == "unmask":
        calls: list[JsonValue] = []
        unmaskers = [Guard(index, {}, calls) for index in range(step["guardrails"])]
        probe = connect(Socket(refuse=False), step["request_data"], unmaskers, None)
        return probe.unmask_view(step["message"], calls)
    return local_answer(step)


# one loop runs every step, as one runtime does on the Rust side
def run(input_value: Input) -> JsonValue:
    with asyncio.Runner() as runner:
        return [runner.run(answer(step)) for step in input_value["steps"]]
