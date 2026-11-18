import asyncio
import json
from types import TracebackType
from typing import ClassVar, Literal, NotRequired, Self, TypedDict, cast

import litellm
from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.realtime_streaming import RealTimeStreaming
from litellm.litellm_core_utils.realtime_streaming import (
    _ASGIScope as ASGIScope,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L62-L66
)
from litellm.llms.base_llm.realtime.transformation import RealtimeBackend
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.utils import GenericGuardrailAPIInputs
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

WIRE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MAX_ITEMS, MAX_TEXT_BYTES, MAX_CAPTURE_BYTES = 32, 16 * 1024, 1024 * 1024


class RaisedDocument(TypedDict):
    message: str
    detail: NotRequired[JsonValue]
    status_code: NotRequired[int]
    value_error: NotRequired[bool]


class PolicyDocument(TypedDict, total=False):
    violation_message: str
    on_violation: str
    end_session_after_n_fails: int


class GuardrailDocument(TypedDict):
    kind: Literal["custom", "native", "plain"]
    hooks: list[str]
    default_on: bool
    outcome: Literal["clean", "rewrite", "raise"]
    name: NotRequired[str]
    raised: NotRequired[RaisedDocument]
    policy: NotRequired[PolicyDocument]


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


class ScriptedError(Exception):
    def __init__(self, message: str, label: str = "unexpected") -> None:
        super().__init__(message)
        self.label = label


class ScriptedRaiseError(ValueError):
    def __init__(self, message: str, detail: JsonValue) -> None:
        super().__init__(message)
        self.detail = detail


def recorded(text: str) -> JsonValue:
    try:
        payload: JsonValue = {"kind": "json", "value": WIRE.validate_json(text)}
    except ValidationError:
        payload = {"kind": "text", "text": text}
    return {"wire": "text", "payload": payload}


class ScriptedSocket(RealtimeBackend):
    def __init__(self, input_value: Input, timeline: list[JsonValue], name: str) -> None:
        self.timeline = timeline
        self.failures = input_value["backend_failures"]
        if name == "client":
            self.failures = input_value["client_failures"]
        self.name = name
        self.label = "send" if name == "backend" else "client"
        self.close_failure = name == "backend" and input_value["close_failure"]
        self.attempts = self.closes = 0
        self.sent: list[str] = []

    @property
    def scope(self) -> ASGIScope:
        return {"headers": []}

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
                attempt = self.attempts
                self.attempts += 1
                if attempt in self.failures:
                    raise ScriptedError(f"the scripted {self.label} send failed", self.label)
                self.sent.append(message)
                self.timeline.append(self.name)
            case bytes():
                raise ScriptedError("the guardrail runner sends no binary frames", self.label)

    async def send_text(self, data: str) -> None:
        await self.send(data)

    @override
    async def recv(self, decode: bool | None = None) -> str:
        raise ScriptedError(f"the scripted socket never receives (decode={decode})", "socket")

    receive_text = recv

    @override
    async def close(self, code: int = 1000, reason: str | None = None) -> None:
        self.frame = (code, reason)
        self.timeline.append(f"{self.name}-close")
        self.closes += 1
        if self.close_failure:
            raise ScriptedError(f"the scripted {self.name} close failed", "close")


class Session(RealTimeStreaming):
    def __init__(self, input_value: Input) -> None:
        self.timeline: list[JsonValue] = []
        self.checks: list[JsonValue] = []
        self.applied: list[JsonValue] = []
        self.backend = ScriptedSocket(input_value, self.timeline, "backend")
        self.client = ScriptedSocket(input_value, self.timeline, "client")
        self.websocket = self.client
        self.backend_ws = self.backend
        self.request_data = input_value["request_data"]
        self.user_api_key_dict = input_value["user_api_key_dict"]
        self.provider_config = self._force_transcription_model = None
        self._violation_count = 0

    def violation_count(self) -> int:
        return self._violation_count


class ApplyScript:
    _document: GuardrailDocument
    _session: Session

    async def apply_guardrail(
        self,
        inputs: GenericGuardrailAPIInputs,
        request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"],
        logging_obj: Logging | None = None,
    ) -> GenericGuardrailAPIInputs:
        del logging_obj
        record: dict[str, JsonValue] = {
            "inputs": WIRE.validate_python(inputs),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1312-L1318
            "request_data": request_data,
            "input_type": input_type,
        }
        self._session.applied.append(record)
        match self._document["outcome"]:
            case "clean":
                return inputs  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1312-L1318
            case "rewrite":
                return {"texts": ["the guardrail rewrite"]}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1312-L1318
            case "raise":
                raised = self._document.get("raised")
                if raised is None:
                    raise ScriptedError("")
                if raised.get("status_code") is not None or raised.get("value_error") is True:
                    raise ScriptedRaiseError(raised["message"], raised.get("detail"))
                raise ScriptedError(raised["message"])


class ScriptedGuardrail(
    ApplyScript,
    cast("type[CustomGuardrail]", CustomGuardrail)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L190-L210
):
    def __init__(self, document: GuardrailDocument, session: Session) -> None:
        self._document = document
        self._session = session
        policy = document.get("policy")
        settings: PolicyDocument = {} if policy is None else policy
        CustomGuardrail.__init__(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L190-L210
            self,
            guardrail_name=document.get("name"),
            event_hook=[GuardrailEventHooks(name) for name in document["hooks"]],
            default_on=document["default_on"],
            end_session_after_n_fails=settings.get("end_session_after_n_fails"),
            on_violation=settings.get("on_violation"),
            realtime_violation_message=settings.get("violation_message"),
        )

    @override
    def should_run_guardrail(
        self, data: dict[str, JsonValue], event_type: GuardrailEventHooks
    ) -> bool:
        self._session.checks.append({"event": event_type.value, "data": data})
        return CustomGuardrail.should_run_guardrail(self, data, event_type)  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1058-L1062


class ScriptedNativeGuardrail(ScriptedGuardrail):
    use_native_lifecycle_hooks: ClassVar[bool] = True


def callback_for(document: GuardrailDocument, session: Session) -> CustomLogger:
    if document["kind"] == "plain":
        return CustomLogger()
    constructor = ScriptedNativeGuardrail if document["kind"] == "native" else ScriptedGuardrail
    return constructor(document, session)


async def session(input_value: Input) -> JsonValue:
    state = Session(input_value)
    objects = [callback_for(document, state) for document in input_value["guardrails"]]
    litellm.callbacks = [objects[index] for index in input_value["callbacks"]]
    runs: list[JsonValue] = []
    for document in input_value["runs"]:
        hooks = document.get("event_hooks")
        event_hooks = None if hooks is None else [GuardrailEventHooks(name) for name in hooks]
        outcome: dict[str, JsonValue] = {"blocked": None, "error": None}
        try:
            outcome["blocked"] = await state.run_realtime_guardrails(
                document["transcript"],
                item_id=document.get("item_id"),
                pre_block_backend_message=document.get("pre_block_backend_message"),
                event_hooks=event_hooks,
            )
        except ScriptedError as error:
            outcome["error"] = error.label
        outcome["backend"] = [recorded(text) for text in state.backend.sent]
        outcome["client"] = [recorded(text) for text in state.client.sent]
        state.backend.sent, state.client.sent = [], []
        runs.append(outcome)
        if outcome["error"] is not None:
            break
    return {
        "runs": runs,
        "timeline": state.timeline,
        "checks": state.checks,
        "applied": state.applied,
        "state": {
            "violation_count": state.violation_count(),
            "backend_closes": state.backend.closes,
            "client_closes": state.client.closes,
        },
    }


def text_sizes(input_value: Input) -> list[int]:
    texts: list[str | None] = []
    for document in input_value["runs"]:
        texts += [document["transcript"], document.get("item_id")]
        texts.append(document.get("pre_block_backend_message"))
    for guardrail in input_value["guardrails"]:
        policy = guardrail.get("policy")
        raised = guardrail.get("raised")
        texts += [
            guardrail.get("name"),
            None if policy is None else policy.get("violation_message"),
            None if raised is None else raised["message"],
        ]
    for value in (input_value["request_data"], input_value["user_api_key_dict"]):
        if value is not None:
            texts.append(json.dumps(value, ensure_ascii=False, separators=(",", ":")))
    return [0 if text is None else len(text.encode("utf-8")) for text in texts]


def run(input_value: Input) -> JsonValue:
    guardrails = len(input_value["guardrails"])
    counts = (len(input_value["runs"]), len(input_value["backend_failures"]),
              len(input_value["client_failures"]))
    if max(guardrails, len(input_value["callbacks"])) > MAX_ITEMS or max(counts) > MAX_ITEMS:
        raise ValueError("the scripted guardrails input is past its limits")
    if any(index >= guardrails for index in input_value["callbacks"]):
        raise ValueError("a callback index is past the guardrails")
    sizes = text_sizes(input_value)
    if any(size > MAX_TEXT_BYTES for size in sizes) or sum(sizes) > MAX_CAPTURE_BYTES:
        raise ValueError("the scripted guardrails input is past its byte limit")
    return asyncio.run(asyncio.wait_for(session(input_value), 120.0))
