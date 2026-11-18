from collections.abc import Sequence
from typing import ClassVar, Literal, NotRequired, Protocol, TypedDict

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.realtime_streaming import RealTimeStreaming
from litellm.llms.base_llm.realtime.transformation import RealtimeBackend
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.utils import GenericGuardrailAPIInputs
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import ReadOnly, override

WIRE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
REWRITE_TEXT = "the guardrail rewrite"


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


class Scope(TypedDict, total=False):
    headers: ReadOnly[Sequence[tuple[bytes | str, bytes | str]]]


class ScriptedError(Exception):
    def __init__(
        self, message: str, label: str = "unexpected", detail: JsonValue = None
    ) -> None:
        super().__init__(message)
        self.label = label
        self.detail = detail


class ScriptedBlockError(ScriptedError):
    def __init__(self, message: str, detail: JsonValue, status_code: int) -> None:
        super().__init__(message, "block", detail)
        self.status_code = status_code


class ScriptedValueError(ValueError, ScriptedError):
    def __init__(self, message: str, detail: JsonValue = None) -> None:
        ValueError.__init__(self, message)
        self.label = "value"
        self.detail = detail


def scripted_exception(raised: RaisedDocument | None) -> ScriptedError:
    if raised is None:
        return ScriptedError("")
    message = raised["message"]
    detail = raised.get("detail")
    status_code = raised.get("status_code")
    if status_code is not None:
        return ScriptedBlockError(message, detail, status_code)
    if raised.get("value_error") is True:
        return ScriptedValueError(message, detail)
    return ScriptedError(message, "unexpected", detail)


class Records:
    def __init__(self) -> None:
        self.checks: list[JsonValue] = []
        self.applied: list[JsonValue] = []


class WireLog:
    def __init__(self, failures: list[int], label: str) -> None:
        self.attempts = 0
        self.failures = failures
        self.label = label
        self.sent: list[str] = []

    def begin(self) -> None:
        attempt = self.attempts
        self.attempts += 1
        if attempt in self.failures:
            raise ScriptedError(f"the scripted {self.label} send failed", self.label)

    def take(self) -> list[str]:
        frames = [*self.sent]
        self.sent = []
        return frames


def recorded(text: str) -> JsonValue:
    payload: JsonValue
    try:
        payload = {"kind": "json", "value": WIRE.validate_json(text)}
    except ValidationError:
        payload = {"kind": "text", "text": text}
    return {"wire": "text", "payload": payload}


class ClientSocket(Protocol):
    @property
    def scope(self) -> Scope: ...

    async def send_text(self, data: str) -> None: ...

    async def receive_text(self) -> str: ...

    async def close(self, code: int = 1000, reason: str | None = None) -> None: ...


class Probe(RealTimeStreaming):
    def __init__(
        self,
        client: ClientSocket,
        backend: RealtimeBackend,
        request_data: dict[str, JsonValue],
        user_api_key_dict: JsonValue,
    ) -> None:
        self.websocket = client
        self.backend_ws = backend
        self.request_data = request_data
        self.user_api_key_dict = user_api_key_dict
        self.provider_config = None
        self._force_transcription_model = None
        self._violation_count = 0

    def violation_count(self) -> int:
        return self._violation_count


class ApplyScript:
    _document: GuardrailDocument
    _records: Records

    async def apply_guardrail(
        self,
        inputs: GenericGuardrailAPIInputs,
        request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"],
        logging_obj: Logging | None = None,
    ) -> GenericGuardrailAPIInputs:
        del logging_obj
        record: dict[str, JsonValue] = {
            "inputs": WIRE.validate_python(inputs),
            "request_data": request_data,
            "input_type": input_type,
        }
        self._records.applied.append(record)
        match self._document["outcome"]:
            case "clean":
                return inputs
            case "rewrite":
                return {"texts": [REWRITE_TEXT]}
            case "raise":
                raise scripted_exception(self._document.get("raised"))


class ScriptedGuardrail(ApplyScript, CustomGuardrail):
    def __init__(self, document: GuardrailDocument, records: Records) -> None:
        self._document = document
        self._records = records
        policy = document.get("policy")
        settings: PolicyDocument = {} if policy is None else policy
        CustomGuardrail.__init__(
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
        record: dict[str, JsonValue] = {"event": event_type.value, "data": data}
        self._records.checks.append(record)
        return CustomGuardrail.should_run_guardrail(self, data, event_type)


class ScriptedNativeGuardrail(ScriptedGuardrail):
    use_native_lifecycle_hooks: ClassVar[bool] = True


def callback_for(document: GuardrailDocument, records: Records) -> CustomLogger:
    match document["kind"]:
        case "plain":
            return CustomLogger()
        case "native":
            return ScriptedNativeGuardrail(document, records)
        case "custom":
            return ScriptedGuardrail(document, records)
