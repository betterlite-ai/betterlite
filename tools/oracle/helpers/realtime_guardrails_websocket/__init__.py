import asyncio
import json
from types import TracebackType
from typing import NotRequired, Self, TypedDict

import litellm
from litellm.llms.base_llm.realtime.transformation import RealtimeBackend
from litellm.types.guardrails import GuardrailEventHooks
from pydantic import JsonValue
from typing_extensions import override

from .scripted import (
    ClientSocket,
    GuardrailDocument,
    Probe,
    Records,
    Scope,
    ScriptedError,
    WireLog,
    callback_for,
    recorded,
)

MAX_GUARDRAILS = 32
MAX_RUNS = 32
MAX_TEXT_BYTES = 16 * 1024
MAX_CAPTURE_BYTES = 1024 * 1024
DEADLINE_SECONDS = 120.0


class RunDocument(TypedDict):
    transcript: str
    item_id: NotRequired[str | None]
    pre_block_backend_message: NotRequired[str | None]
    event_hooks: NotRequired[list[str] | None]


class Input(TypedDict):
    request_data: dict[str, JsonValue]
    user_api_key_dict: JsonValue
    guardrails: list[GuardrailDocument]
    callbacks: list[int]
    backend_failures: list[int]
    client_failures: list[int]
    close_failure: bool
    runs: list[RunDocument]


class ScriptedBackend(RealtimeBackend):
    def __init__(
        self, log: WireLog, timeline: list[JsonValue], *, close_failure: bool
    ) -> None:
        self.log = log
        self.timeline = timeline
        self.close_failure = close_failure
        self.closes = 0
        self.exit_state: tuple[
            type[BaseException] | None, BaseException | None, TracebackType | None
        ] = (None, None, None)

    @override
    async def __aenter__(self) -> Self:
        return self

    @override
    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.exit_state = (exc_type, exc_value, traceback)

    @override
    async def send(self, message: str | bytes) -> None:
        match message:
            case str():
                self.log.begin()
                self.log.sent.append(message)
                self.timeline.append("backend")
            case bytes():
                raise ScriptedError("the guardrail runner sends no binary frames", "send")

    @override
    async def recv(self, decode: bool | None = None) -> str | bytes:
        raise ScriptedError(f"the scripted backend never receives (decode={decode})", "socket")

    @override
    async def close(self) -> None:
        self.timeline.append("backend-close")
        self.closes += 1
        if self.close_failure:
            raise ScriptedError("the scripted backend close failed", "close")


class ScriptedClient(ClientSocket):
    def __init__(self, log: WireLog, timeline: list[JsonValue]) -> None:
        self.log = log
        self.timeline = timeline
        self.closes = 0
        self.close_frame: tuple[int, str | None] = (1000, None)

    @property
    @override
    def scope(self) -> Scope:
        return {"headers": []}

    @override
    async def send_text(self, data: str) -> None:
        self.log.begin()
        self.log.sent.append(data)
        self.timeline.append("client")

    @override
    async def receive_text(self) -> str:
        raise ScriptedError("the scripted client never receives", "socket")

    @override
    async def close(self, code: int = 1000, reason: str | None = None) -> None:
        self.close_frame = (code, reason)
        self.closes += 1


class Session:
    def __init__(self, input_value: Input) -> None:
        self.timeline: list[JsonValue] = []
        self.backend_log = WireLog(input_value["backend_failures"], "send")
        self.client_log = WireLog(input_value["client_failures"], "client")
        self.backend = ScriptedBackend(
            self.backend_log, self.timeline, close_failure=input_value["close_failure"]
        )
        self.client = ScriptedClient(self.client_log, self.timeline)
        self.records = Records()
        self.probe = Probe(
            self.client,
            self.backend,
            input_value["request_data"],
            input_value["user_api_key_dict"],
        )

    def snapshot(self, runs: list[JsonValue]) -> dict[str, JsonValue]:
        return {
            "runs": runs,
            "timeline": self.timeline,
            "checks": self.records.checks,
            "applied": self.records.applied,
            "state": {
                "violation_count": self.probe.violation_count(),
                "backend_closes": self.backend.closes,
                "client_closes": self.client.closes,
            },
        }


async def run_once(state: Session, document: RunDocument) -> dict[str, JsonValue]:
    hooks = document.get("event_hooks")
    event_hooks = None if hooks is None else [GuardrailEventHooks(name) for name in hooks]
    result: dict[str, JsonValue] = {"blocked": None, "error": None}
    try:
        result["blocked"] = await state.probe.run_realtime_guardrails(
            document["transcript"],
            item_id=document.get("item_id"),
            pre_block_backend_message=document.get("pre_block_backend_message"),
            event_hooks=event_hooks,
        )
    except ScriptedError as error:
        result["error"] = error.label
    result["backend"] = [recorded(text) for text in state.backend_log.take()]
    result["client"] = [recorded(text) for text in state.client_log.take()]
    return result


async def session(input_value: Input) -> JsonValue:
    state = Session(input_value)
    objects = [callback_for(document, state.records) for document in input_value["guardrails"]]
    callbacks = [objects[index] for index in input_value["callbacks"]]
    litellm.callbacks = [*callbacks]
    runs: list[JsonValue] = []
    for document in input_value["runs"]:
        outcome = await run_once(state, document)
        runs.append(outcome)
        if outcome["error"] is not None:
            break
    return state.snapshot(runs)


async def bounded(input_value: Input) -> JsonValue:
    return await asyncio.wait_for(session(input_value), DEADLINE_SECONDS)


def run_sizes(runs: list[RunDocument]) -> list[int]:
    sizes: list[int] = []
    for run in runs:
        sizes.append(len(run["transcript"].encode("utf-8")))
        item_id = run.get("item_id")
        if item_id is not None:
            sizes.append(len(item_id.encode("utf-8")))
        pre_block = run.get("pre_block_backend_message")
        if pre_block is not None:
            sizes.append(len(pre_block.encode("utf-8")))
    return sizes


def guardrail_sizes(guardrails: list[GuardrailDocument]) -> list[int]:
    sizes: list[int] = []
    for document in guardrails:
        name = document.get("name")
        if name is not None:
            sizes.append(len(name.encode("utf-8")))
        policy = document.get("policy")
        if policy is not None:
            message = policy.get("violation_message")
            if message is not None:
                sizes.append(len(message.encode("utf-8")))
        raised = document.get("raised")
        if raised is not None:
            sizes.append(len(raised["message"].encode("utf-8")))
    return sizes


def text_sizes(input_value: Input) -> list[int]:
    sizes = run_sizes(input_value["runs"])
    sizes.extend(guardrail_sizes(input_value["guardrails"]))
    request = json.dumps(input_value["request_data"], ensure_ascii=False, separators=(",", ":"))
    sizes.append(len(request.encode("utf-8")))
    principal = input_value["user_api_key_dict"]
    if principal is not None:
        principal_text = json.dumps(principal, ensure_ascii=False, separators=(",", ":"))
        sizes.append(len(principal_text.encode("utf-8")))
    return sizes


def past_limits(input_value: Input) -> bool:
    guardrails = len(input_value["guardrails"])
    callbacks = len(input_value["callbacks"])
    runs = len(input_value["runs"])
    backend = len(input_value["backend_failures"])
    client = len(input_value["client_failures"])
    return max(guardrails, callbacks) > MAX_GUARDRAILS or max(runs, backend, client) > MAX_RUNS


def checked(input_value: Input) -> None:
    if past_limits(input_value):
        raise ValueError("the scripted guardrails input is past its limits")
    guardrails = input_value["guardrails"]
    if any(index >= len(guardrails) for index in input_value["callbacks"]):
        raise ValueError("a callback index is past the guardrails")
    sizes = text_sizes(input_value)
    if any(size > MAX_TEXT_BYTES for size in sizes) or sum(sizes) > MAX_CAPTURE_BYTES:
        raise ValueError("the scripted guardrails input is past its byte limit")


def run(input_value: Input) -> JsonValue:
    checked(input_value)
    return asyncio.run(bounded(input_value))
