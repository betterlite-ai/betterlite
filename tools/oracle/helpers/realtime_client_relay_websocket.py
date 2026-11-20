import asyncio
import datetime
from types import TracebackType
from typing import Literal, Self, TypedDict

import litellm
from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.realtime_streaming import (
    BackendClose,
    ClientLoopExit,
    RealTimeStreaming,
)
from litellm.llms.base_llm.realtime.transformation import RealtimeBackend
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.utils import GenericGuardrailAPIInputs
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

WIRE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MAX_FRAMES, MAX_TEXT_BYTES, MAX_TOTAL_BYTES = 32, 16 * 1024, 256 * 1024


class CloseDocument(TypedDict):
    code: int
    reason: str


class GuardrailDocument(TypedDict):
    hooks: list[str]
    block_on: str | None


class SetupDocument(TypedDict):
    pending: list[str]
    backend_setup_complete: bool | None
    flushing: bool
    turn_detection_sent: bool
    pending_guardrail_message: str | None
    session_configuration_request: str | None


class LifetimeDocument(TypedDict):
    client_wait: Literal["none", "cancel", "signal"]
    client_signals: bool
    client_outcome: Literal["client_disconnected", "backend_closed"]
    backend_wait: Literal["none", "cancel", "signal"]
    backend_signals: bool
    backend_outcome: Literal["close", "error"]


class Input(TypedDict):
    mode: Literal["ack", "lifetime"]
    messages: list[str]
    setup: SetupDocument
    guardrails: list[GuardrailDocument]
    beta_backend: bool
    close: CloseDocument | None
    lifetime: LifetimeDocument | None
    client_send_failures: list[int]
    client_close_failures: list[int]


class ScriptedError(Exception):
    def __init__(self, message: str, label: str = "unexpected") -> None:
        super().__init__(message)
        self.label = label


def recorded(text: str) -> JsonValue:
    try:
        payload: JsonValue = {"kind": "json", "value": WIRE.validate_json(text)}
    except ValidationError:
        payload = {"kind": "text", "text": text}
    return payload


class ScriptedClient:
    def __init__(self, input_value: Input) -> None:
        self.incoming: list[str] = list(input_value["messages"])
        self.sent: list[str] = []
        self.closes: list[JsonValue] = []
        self.attempts = 0
        self.close_attempts = 0
        self.failures = list(input_value["client_send_failures"])
        self.close_failures = list(input_value["client_close_failures"])

    @property
    def scope(self) -> dict[str, JsonValue]:
        return {"headers": []}

    async def receive_text(self) -> str:
        if not self.incoming:
            raise ScriptedError("the scripted client is exhausted", "client")
        return self.incoming.pop(0)

    async def send_text(self, data: str) -> None:
        attempt = self.attempts
        self.attempts += 1
        if attempt in self.failures:
            raise ScriptedError(f"the scripted client send {attempt} failed", "client")
        self.sent.append(data)

    async def close(self, code: int = 1000, reason: str | None = None) -> None:
        attempt = self.close_attempts
        self.close_attempts += 1
        self.closes.append(WIRE.validate_python([code, reason]))
        if attempt in self.close_failures:
            raise ScriptedError(f"the scripted client close {attempt} failed", "close")


class ScriptedBackend(RealtimeBackend):
    def __init__(self, input_value: Input) -> None:
        del input_value
        self.sent: list[str] = []

    @override
    async def __aenter__(self) -> Self:
        return self

    @override
    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None: ...

    @override
    async def send(self, message: str | bytes) -> None:
        match message:
            case str():
                self.sent.append(message)
            case bytes():
                raise ScriptedError("the client relay sends no binary frames", "backend")

    @override
    async def recv(self, decode: bool | None = None) -> str | bytes:
        raise ScriptedError(f"the scripted backend never receives (decode={decode})", "backend")

    @override
    async def close(self) -> None: ...


class ScriptedGuardrail(CustomGuardrail):
    def __init__(self, document: GuardrailDocument, applied: list[JsonValue]) -> None:
        super().__init__(
            guardrail_name="scripted",
            event_hook=[GuardrailEventHooks(name) for name in document["hooks"]],
            default_on=True,
        )
        self.hooks = list(document["hooks"])
        self.block_on = document["block_on"]
        self.applied = applied

    @override
    def should_run_guardrail(
        self, data: dict[str, JsonValue], event_type: GuardrailEventHooks
    ) -> bool:
        del data
        return event_type.value in self.hooks

    @override
    async def apply_guardrail(
        self,
        inputs: GenericGuardrailAPIInputs,
        request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"],
        logging_obj: Logging | None = None,
    ) -> GenericGuardrailAPIInputs:
        del request_data, input_type, logging_obj
        texts = inputs.get("texts")
        checked: list[str] = [] if texts is None else texts
        self.applied.append(WIRE.validate_python(checked))
        if self.block_on is not None and checked[:1] == [self.block_on]:
            raise ValueError("blocked")
        return {"texts": checked}


class Probe(RealTimeStreaming):
    def __init__(self, input_value: Input) -> None:
        self.client = ScriptedClient(input_value)
        self.backend = ScriptedBackend(input_value)
        self.applied: list[JsonValue] = []
        self.timeline: list[JsonValue] = []
        super().__init__(
            websocket=self.client,
            backend_ws=self.backend,
            logging_obj=Logging(
                model="scripted",
                messages=[],
                stream=False,
                call_type="realtime",
                start_time=datetime.datetime.now(),
                litellm_call_id="scripted",
                function_id="scripted",
            ),
            provider_config=None,
            model="scripted",
            request_data={},
            backend_uses_beta_protocol=input_value["beta_backend"],
            event_normalizer=None,
        )
        self.configure(input_value["setup"])

    def configure(self, setup: SetupDocument) -> None:
        if setup["backend_setup_complete"] is not None:
            self._backend_setup_complete = setup["backend_setup_complete"]
        self._flushing_pending_messages_until_setup = setup["flushing"]
        self._guardrail_turn_detection_update_sent = setup["turn_detection_sent"]
        self._pending_guardrail_message = setup["pending_guardrail_message"]
        self.session_configuration_request = setup["session_configuration_request"]
        self._pending_messages_until_setup = list(setup["pending"])
        self._pending_messages_byte_total = sum(
            len(message.encode("utf-8")) for message in setup["pending"]
        )

    def snapshot(self, mode: str, exit_name: str | None, result: JsonValue) -> JsonValue:
        document: dict[str, object] = {
            "mode": mode,
            "exit": exit_name,
            "result": result,
            "client": [recorded(text) for text in self.client.sent],
            "backend": [recorded(text) for text in self.backend.sent],
            "closes": self.client.closes,
            "pending": self._pending_messages_until_setup,
            "applied": self.applied,
            "timeline": self.timeline,
            "flags": {
                "turn_detection_sent": self._guardrail_turn_detection_update_sent,
                "setup_complete": self._backend_setup_complete,
                "flushing": self._flushing_pending_messages_until_setup,
                "pending_guardrail": self._pending_guardrail_message,
            },
        }
        return WIRE.validate_python(document)


class LifetimeProbe(Probe):
    def __init__(self, input_value: Input) -> None:
        super().__init__(input_value)
        lifetime = input_value["lifetime"]
        close = input_value["close"]
        if lifetime is None or close is None:
            raise ScriptedError(
                "a lifetime run needs both a lifetime script and a close document", "input"
            )
        self.lifetime = lifetime
        self.close_document = close
        self.cancellation = asyncio.Event()
        self.signal = asyncio.Event()

    @override
    async def client_ack_messages(self) -> ClientLoopExit:
        if self.lifetime["client_wait"] == "cancel":
            try:
                await self.cancellation.wait()
            except asyncio.CancelledError:
                self.timeline.append("client-cancel")
                raise
        elif self.lifetime["client_wait"] == "signal":
            await self.signal.wait()
            self.timeline.append("client-signal")
        if self.lifetime["client_signals"]:
            self.timeline.append("client-notify")
            self.signal.set()
        self.timeline.append("client-finish")
        if self.lifetime["client_outcome"] == "client_disconnected":
            return ClientLoopExit.CLIENT_DISCONNECTED
        return ClientLoopExit.BACKEND_CLOSED

    @override
    async def backend_to_client_send_messages(self) -> BackendClose:
        if self.lifetime["backend_wait"] == "cancel":
            try:
                await self.cancellation.wait()
            except asyncio.CancelledError:
                self.timeline.append("backend-cancel")
                raise
        elif self.lifetime["backend_wait"] == "signal":
            await self.signal.wait()
            self.timeline.append("backend-signal")
        if self.lifetime["backend_signals"]:
            self.timeline.append("backend-notify")
            self.signal.set()
        if self.lifetime["backend_outcome"] == "error":
            self.timeline.append("backend-error")
            raise ScriptedError("the scripted backend direction failed", "backend")
        self.timeline.append("backend-finish")
        return BackendClose(
            code=self.close_document["code"], reason=self.close_document["reason"]
        )


async def session(input_value: Input) -> JsonValue:
    probe: Probe
    if input_value["mode"] == "lifetime":
        probe = LifetimeProbe(input_value)
    else:
        probe = Probe(input_value)
    callbacks: list[CustomLogger] = [
        ScriptedGuardrail(document, probe.applied) for document in input_value["guardrails"]
    ]
    previous = litellm.callbacks
    litellm.callbacks = callbacks
    try:
        if input_value["mode"] == "lifetime":
            outcome: JsonValue = None
            try:
                await probe.bidirectional_forward()
                outcome = {"ok": True}
            except ScriptedError as error:
                outcome = {"error": error.label}
            return probe.snapshot("lifetime", None, outcome)
        exit_value = await probe.client_ack_messages()
        exit_name = (
            "client_disconnected"
            if exit_value is ClientLoopExit.CLIENT_DISCONNECTED
            else "backend_closed"
        )
        return probe.snapshot("ack", exit_name, None)
    finally:
        litellm.callbacks = previous


def run(input_value: Input) -> JsonValue:
    if (
        len(input_value["messages"]) > MAX_FRAMES
        or len(input_value["setup"]["pending"]) > MAX_FRAMES
    ):
        raise ValueError("the scripted client relay input is past its limits")
    sizes = [
        len(text.encode("utf-8"))
        for text in input_value["messages"] + input_value["setup"]["pending"]
    ]
    if any(size > MAX_TEXT_BYTES for size in sizes) or sum(sizes) > MAX_TOTAL_BYTES:
        raise ValueError("the scripted client relay input is past its limits")
    if input_value["mode"] == "lifetime" and (
        input_value["lifetime"] is None or input_value["close"] is None
    ):
        raise ValueError("a lifetime run needs both a lifetime script and a close document")
    return asyncio.run(asyncio.wait_for(session(input_value), 120.0))
