import asyncio
from importlib import import_module
from pathlib import Path
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, cast

import litellm
from litellm.proxy._types import UserAPIKeyAuth
from litellm.responses.streaming_iterator import ProjectQuotaCallback
from litellm.types.responses.streaming_websocket import (
    PresidioGuardrailCallback,
    ResponsesWebSocketRequestDefaults,
)
from pydantic import JsonValue

litellm.suppress_debug_info = True

AFFINITY: Final[dict[str, JsonValue]] = {
    "encrypted_content_affinity_enabled": True,
    "model_info": {"id": "m"},
}

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


Step: TypeAlias = (
    QuotaStep | CreateStep | UnmaskStep | CompletedStep | FailureStep | WrapStep | SetsStep
)


class Input(TypedDict):
    steps: list[Step]


class Socket(Protocol):
    sent: list[str]


class SocketFactory(Protocol):
    def __call__(self, *, refuse: bool) -> Socket: ...


class GuardFactory(Protocol):
    def __call__(
        self, index: int, replacements: dict[str, str], calls: list[JsonValue]
    ) -> PresidioGuardrailCallback: ...


class QuotaFactory(Protocol):
    def __call__(
        self, index: int, verdict: Verdict, calls: list[JsonValue]
    ) -> ProjectQuotaCallback: ...


class Probe(Protocol):
    user_api_key_dict: UserAPIKeyAuth | None
    quota_callbacks: tuple[ProjectQuotaCallback, ...]
    output_guardrail_callbacks: list[PresidioGuardrailCallback]
    request_defaults: ResponsesWebSocketRequestDefaults | None

    async def quota_view(
        self, socket: Socket, calls: list[JsonValue], message: str
    ) -> JsonValue: ...
    async def create_view(self, message: str, calls: list[JsonValue]) -> JsonValue: ...
    def unmask_view(self, message: str, calls: list[JsonValue]) -> JsonValue: ...
    async def completed_view(self, message: str, calls: list[JsonValue]) -> JsonValue: ...
    def failure_status(self, event: dict[str, JsonValue]) -> JsonValue: ...
    def wrap_view(self, event: dict[str, JsonValue]) -> JsonValue: ...
    def sets_view(self, candidates: list[str]) -> JsonValue: ...


class Runtime(Protocol):
    Socket: SocketFactory
    Guard: GuardFactory
    Quota: QuotaFactory

    def connect(
        self,
        socket: Socket,
        request_data: dict[str, JsonValue],
        guardrails: list[PresidioGuardrailCallback],
        authorized_model: str | None,
    ) -> Probe: ...
    def request_defaults(
        self, fill_missing: dict[str, JsonValue], overrides: dict[str, JsonValue]
    ) -> ResponsesWebSocketRequestDefaults: ...


__path__ = [str(Path(__file__).with_suffix(""))]

RUNTIME: Final[Runtime] = cast(Runtime, import_module("responses_ws_frame_policy.runtime"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1887


async def quota_answer(step: QuotaStep) -> JsonValue:
    calls: list[JsonValue] = []
    socket = RUNTIME.Socket(refuse=step["refuse_send"])
    probe = RUNTIME.connect(socket, {}, [], step["authorized_model"])
    probe.user_api_key_dict = UserAPIKeyAuth(user_id="u-1")
    probe.quota_callbacks = tuple(
        RUNTIME.Quota(index, verdict, calls) for index, verdict in enumerate(step["verdicts"])
    )
    return await probe.quota_view(socket, calls, step["message"])


async def create_answer(step: CreateStep) -> JsonValue:
    calls: list[JsonValue] = []
    inputs = [
        RUNTIME.Guard(index, pairs, calls) for index, pairs in enumerate(step["guardrails"])
    ]
    probe = RUNTIME.connect(
        RUNTIME.Socket(refuse=False), step["request_data"], inputs, step["authorized_model"]
    )
    defaults = step["defaults"]
    if defaults is not None:
        probe.request_defaults = RUNTIME.request_defaults(
            defaults["fill_missing"], defaults["overrides"]
        )
    return await probe.create_view(step["message"], calls)


async def completed_answer(step: CompletedStep) -> JsonValue:
    calls: list[JsonValue] = []
    probe = RUNTIME.connect(RUNTIME.Socket(refuse=False), step["request_data"], [], None)
    probe.output_guardrail_callbacks = [
        RUNTIME.Guard(index, pairs, calls) for index, pairs in enumerate(step["guardrails"])
    ]
    return await probe.completed_view(step["message"], calls)


async def unmask_answer(step: UnmaskStep) -> JsonValue:
    calls: list[JsonValue] = []
    inputs = [RUNTIME.Guard(index, {}, calls) for index in range(step["guardrails"])]
    probe = RUNTIME.connect(RUNTIME.Socket(refuse=False), step["request_data"], inputs, None)
    return probe.unmask_view(step["message"], calls)


def local_answer(step: FailureStep | WrapStep | SetsStep) -> JsonValue:
    if step["kind"] == "failure":
        probe = RUNTIME.connect(RUNTIME.Socket(refuse=False), {}, [], None)
        return [probe.failure_status(event) for event in step["events"]]
    if step["kind"] == "wrap":
        metadata: dict[str, JsonValue] = {"litellm_metadata": step["litellm_metadata"]}
        probe = RUNTIME.connect(RUNTIME.Socket(refuse=False), metadata, [], None)
        return probe.wrap_view(step["event"])
    affinity: dict[str, JsonValue] = {"litellm_metadata": AFFINITY}
    probe = RUNTIME.connect(RUNTIME.Socket(refuse=False), affinity, [], None)
    return probe.sets_view(step["candidates"])


async def answer(step: Step) -> JsonValue:
    if step["kind"] == "quota":
        return await quota_answer(step)
    if step["kind"] == "create":
        return await create_answer(step)
    if step["kind"] == "completed":
        return await completed_answer(step)
    if step["kind"] == "unmask":
        return await unmask_answer(step)
    return local_answer(step)


# one loop runs every step, as one runtime does on the Rust side
def run(input_value: Input) -> JsonValue:
    with asyncio.Runner() as runner:
        return [runner.run(answer(step)) for step in input_value["steps"]]
