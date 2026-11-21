import asyncio
import json
from collections.abc import Container, Mapping
from datetime import datetime
from typing import Final, Literal, TypeAlias, TypedDict

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

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
# the websocket handler takes object-valued maps, so a case's JSON map crosses the boundary here
OBJECTS: Final = TypeAdapter(dict[str, object])
QUOTA_MESSAGE: Final = "quota exceeded"

Verdict: TypeAlias = Literal["accept", "rate_limit", "fail"]


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
        await asyncio.sleep(0)
        user = None if user_api_key_dict is None else user_api_key_dict.user_id
        # a bool max_output_tokens reaches the callback as True, which a quota counts as 1
        call: list[JsonValue] = [
            self.index,
            user,
            requested_model,
            int(estimated_input_tokens),
            int(estimated_output_tokens),
        ]
        self.calls.append(call)
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
    def get_presidio_settings_from_request_data(
        self, data: Container[str]
    ) -> PresidioPerRequestConfig | None:
        return None

    @override
    async def check_pii(
        self,
        text: str,
        output_parse_pii: bool,
        presidio_config: PresidioPerRequestConfig | None,
        request_data: Container[str],
    ) -> str:
        await asyncio.sleep(0)
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
        await asyncio.sleep(0)
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
            {
                "type": frame["type"],
                "error_type": frame["error"]["type"],
                "message_is_error_text": frame["error"]["message"] == error_text,
            }
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
            return {
                "error": True,
                "same": False,
                "frame": None,
                "calls": calls,
                "metadata": metadata,
            }
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
        output_item = [
            kind for kind in candidates if self.wrap_view({"type": kind, "item": item}) is not None
        ]
        frame: dict[str, JsonValue] = {
            "logged": RESPONSES_WS_LOGGED_EVENT_TYPES,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1817
            "maskable": sorted(RESPONSES_WS_MASKABLE_TEXT_BLOCK_TYPES),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1825
            "failure": failure,
            "output_item": output_item,
            "delta": sorted(self._DELTA_EVENT_TYPES),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L2241
            "done": sorted(self._OUTPUT_DONE_EVENT_TYPES),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L2254
        }
        return JSON.validate_python(frame)


def connect(
    socket: Socket,
    request_data: dict[str, JsonValue],
    guardrails: list[Guard],
    authorized_model: str | None,
) -> Probe:
    log_obj = Logging(
        model="gpt-4o",
        messages=None,
        stream=True,
        call_type="responses",
        start_time=datetime.fromisoformat("2026-01-01T00:00:00+00:00"),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    return Probe(
        websocket=socket,
        backend_ws=Backend(),
        logging_obj=log_obj,
        request_data=OBJECTS.validate_python(request_data),
        guardrail_callbacks=guardrails,
        authorized_model=authorized_model,
    )


def request_defaults(
    fill_missing: dict[str, JsonValue], overrides: dict[str, JsonValue]
) -> ResponsesWebSocketRequestDefaults:
    return ResponsesWebSocketRequestDefaults(
        fill_missing=OBJECTS.validate_python(fill_missing),
        overrides=OBJECTS.validate_python(overrides),
    )
