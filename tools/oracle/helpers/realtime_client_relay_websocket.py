import asyncio
import datetime as dt
from types import TracebackType
from typing import Literal, Self, TypedDict

import litellm
from litellm.integrations.custom_guardrail import CustomGuardrail
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

Wait = Literal["none", "cancel", "signal"]


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
    client_wait: Wait
    client_signals: bool
    client_outcome: Literal["client_disconnected", "backend_closed"]
    backend_wait: Wait
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


WIRE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MAX_FRAMES, MAX_TEXT_BYTES, MAX_TOTAL_BYTES = 32, 16 * 1024, 256 * 1024


class ScriptedError(Exception): ...


class _RelayMeta(type): ...


def recorded(text: str) -> JsonValue:
    try:
        return {"kind": "json", "value": WIRE.validate_json(text)}
    except ValidationError:
        return {"kind": "text", "text": text}


class ScriptedClient:
    def __init__(self, input_value: Input) -> None:
        self.incoming: list[str] = list(input_value["messages"])
        self.sent: list[str] = []
        self.closes: list[JsonValue] = []
        self.attempts = self.close_attempts = 0
        self.failures = list(input_value["client_send_failures"])
        self.close_failures = list(input_value["client_close_failures"])

    @property
    def scope(self) -> dict[str, JsonValue]:
        return {"headers": []}

    async def receive_text(self) -> str:
        if not self.incoming:
            raise ScriptedError("the scripted client is exhausted")
        return self.incoming.pop(0)

    async def send_text(self, data: str) -> None:
        self.attempts += 1
        if self.attempts - 1 in self.failures:
            raise ScriptedError(f"the scripted client send {self.attempts - 1} failed")
        self.sent.append(data)

    async def close(self, code: int = 1000, reason: str | None = None) -> None:
        self.close_attempts += 1
        self.closes.append([code, reason])
        if self.close_attempts - 1 in self.close_failures:
            raise ScriptedError(f"the scripted client close {self.close_attempts - 1} failed")


class ScriptedBackend(RealtimeBackend):
    def __init__(self) -> None:
        self.sent: list[str] = []

    @override
    async def __aenter__(self) -> Self:
        return self

    @override
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None: ...

    @override
    async def send(self, message: str | bytes) -> None:
        match message:
            case str():
                self.sent.append(message)
            case bytes():
                raise ScriptedError("the client relay sends no binary frames")

    @override
    async def recv(self, decode: bool | None = None) -> str | bytes:
        raise ScriptedError(f"the scripted backend never receives (decode={decode})")

    @override
    async def close(self) -> None: ...


class ScriptedGuardrail(CustomGuardrail, metaclass=_RelayMeta):
    def __init__(self, document: GuardrailDocument, applied: list[JsonValue]) -> None:
        super().__init__(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L190
            guardrail_name="scripted", default_on=True,
            event_hook=[GuardrailEventHooks(name) for name in document["hooks"]],
        )
        self.hooks = list(document["hooks"])
        self.block_on = document["block_on"]
        self.applied = applied

    @override
    def should_run_guardrail(
        self, data: dict[str, JsonValue], event_type: GuardrailEventHooks,
    ) -> bool:
        del data
        return event_type.value in self.hooks

    @override
    async def apply_guardrail(
        self, inputs: GenericGuardrailAPIInputs, request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"], logging_obj: Logging | None = None,
    ) -> GenericGuardrailAPIInputs:
        del request_data, input_type, logging_obj
        texts = inputs.get("texts")  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4536
        checked: list[str] = [] if texts is None else texts
        self.applied.append(WIRE.validate_python(checked))
        if self.block_on is not None and checked[:1] == [self.block_on]:
            raise ValueError("blocked")
        return {"texts": checked}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4536


class Probe(RealTimeStreaming):
    def __init__(self, input_value: Input) -> None:
        self.client = ScriptedClient(input_value)
        self.backend = ScriptedBackend()
        self.applied: list[JsonValue] = []
        self.timeline: list[JsonValue] = []
        super().__init__(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L127
            websocket=self.client, backend_ws=self.backend,
            logging_obj=Logging(
                model="scripted", messages=[],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L576
                stream=False, call_type="realtime",
                start_time=dt.datetime.now(tz=dt.UTC),
                litellm_call_id="scripted", function_id="scripted",
            ),
            model="scripted", backend_uses_beta_protocol=input_value["beta_backend"],
        )
        setup = input_value["setup"]
        if setup["backend_setup_complete"] is not None:
            self._backend_setup_complete = setup["backend_setup_complete"]
        self._flushing_pending_messages_until_setup = setup["flushing"]
        self._guardrail_turn_detection_update_sent = setup["turn_detection_sent"]
        self._pending_guardrail_message = setup["pending_guardrail_message"]
        self.session_configuration_request = setup["session_configuration_request"]
        pending = setup["pending"]
        self._pending_messages_until_setup = list(pending)
        self._pending_messages_byte_total = sum(len(text.encode("utf-8")) for text in pending)

    def snapshot(self, mode: str, exit_name: str | None, result: JsonValue) -> JsonValue:
        document = {
            "mode": mode, "exit": exit_name, "result": result,
            "client": [recorded(text) for text in self.client.sent],
            "backend": [recorded(text) for text in self.backend.sent],
            "closes": self.client.closes,
            "pending": WIRE.validate_python(self._pending_messages_until_setup),
            "applied": self.applied, "timeline": self.timeline,
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
        lifetime, close = input_value["lifetime"], input_value["close"]
        if lifetime is None or close is None:
            raise ScriptedError("a lifetime run needs both a lifetime script and a close document")
        self.lifetime = lifetime
        self.close_document = close
        self.cancellation, self.signal = asyncio.Event(), asyncio.Event()

    async def wait_for_turn(self, wait: Wait, label: str, *, signals: bool) -> None:
        if wait == "cancel":
            try:
                await self.cancellation.wait()
            except asyncio.CancelledError:
                self.timeline.append(f"{label}-cancel")
                raise
        elif wait == "signal":
            await self.signal.wait()
            self.timeline.append(f"{label}-signal")
        if signals:
            self.timeline.append(f"{label}-notify")
            self.signal.set()

    @override
    async def client_ack_messages(self) -> ClientLoopExit:
        wait = self.lifetime["client_wait"]
        await self.wait_for_turn(wait, "client", signals=self.lifetime["client_signals"])
        self.timeline.append("client-finish")
        if self.lifetime["client_outcome"] == "client_disconnected":
            return ClientLoopExit.CLIENT_DISCONNECTED
        return ClientLoopExit.BACKEND_CLOSED

    @override
    async def backend_to_client_send_messages(  # ast-grep-ignore: no-long-names
        self,
    ) -> BackendClose:
        wait = self.lifetime["backend_wait"]
        await self.wait_for_turn(wait, "backend", signals=self.lifetime["backend_signals"])
        return self.backend_close()

    def backend_close(self) -> BackendClose:
        if self.lifetime["backend_outcome"] == "error":
            self.timeline.append("backend-error")
            raise ScriptedError("the scripted backend direction failed")
        self.timeline.append("backend-finish")
        return BackendClose(code=self.close_document["code"], reason=self.close_document["reason"])


async def session(input_value: Input) -> JsonValue:
    probe = LifetimeProbe(input_value) if input_value["mode"] == "lifetime" else Probe(input_value)
    previous = litellm.callbacks  # type: ignore[misc]  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L181
    litellm.callbacks = [
        ScriptedGuardrail(document, probe.applied) for document in input_value["guardrails"]
    ]
    try:
        if input_value["mode"] == "lifetime":
            try:
                await probe.bidirectional_forward()
                return probe.snapshot("lifetime", None, {"ok": True})
            except ScriptedError:
                return probe.snapshot("lifetime", None, {"error": "backend"})
        exit_value = await probe.client_ack_messages()
        disconnected = exit_value is ClientLoopExit.CLIENT_DISCONNECTED
        exit_name = "client_disconnected" if disconnected else "backend_closed"
        return probe.snapshot("ack", exit_name, None)
    finally:
        litellm.callbacks = previous


def run(input_value: Input) -> JsonValue:
    messages, pending = input_value["messages"], input_value["setup"]["pending"]
    sizes = [len(text.encode("utf-8")) for text in messages + pending]
    if max(len(messages), len(pending)) > MAX_FRAMES or any(
        size > MAX_TEXT_BYTES for size in sizes
    ) or sum(sizes) > MAX_TOTAL_BYTES:
        raise ValueError("the scripted client relay input is past its limits")
    lifetime, close = input_value["lifetime"], input_value["close"]
    if input_value["mode"] == "lifetime" and (lifetime is None or close is None):
        raise ValueError("a lifetime run needs both a lifetime script and a close document")
    return asyncio.run(asyncio.wait_for(session(input_value), 120.0))
