import asyncio
from collections.abc import Callable
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

import fastuuid
import litellm
from bedrock_services_realtime_session import FixedFast, make_logging
from litellm.constants import (
    BEDROCK_REALTIME_COMMITTED_FAILURE_SCOPE_KEY as FAILURE_KEY,
)
from litellm.constants import (
    BEDROCK_REALTIME_PENDING_SESSION_UPDATE_SCOPE_KEY as PENDING_KEY,
)
from litellm.constants import (
    BEDROCK_REALTIME_SESSION_COMMITTED_SCOPE_KEY as COMMITTED_KEY,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.realtime import handler as bedrock_handler
from litellm.llms.bedrock.realtime.transformation import BedrockRealtimeConfig
from litellm.types.realtime import RealtimeResponseTransformInput
from pydantic import JsonValue, TypeAdapter, ValidationError

MODEL = "amazon.nova-sonic-v1:0"

JsonObject = dict[str, JsonValue]
JsonAdapter: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
StateAdapter: TypeAdapter[RealtimeResponseTransformInput] = TypeAdapter(
    RealtimeResponseTransformInput
)


class RealtimeConfig(Protocol):
    def transform_realtime_response(
        self,
        message: str,
        model: str,
        logging_obj: Logging,
        state: RealtimeResponseTransformInput,
    ) -> JsonValue: ...


CONFIG_TYPE: type[RealtimeConfig] = BedrockRealtimeConfig  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/transformation.py#L1275-L1281


class Scope(TypedDict):
    pending_session_update: JsonValue
    session_committed: JsonValue
    committed_failure: JsonValue


class Step(TypedDict):
    method: Literal[
        "dispatch",
        "client_messages",
        "parse_client_message",
        "pending",
        "provider_failure",
        "ack",
        "should_log",
    ]
    message: NotRequired[str]
    state: NotRequired[JsonObject]
    trace_id: NotRequired[str | None]
    initial: NotRequired[str | None]
    received: NotRequired[list[str | None]]
    scope: NotRequired[Scope]
    class_name: NotRequired[str]
    client_message: NotRequired[JsonObject]
    stream_error: NotRequired[str | None]
    send_error: NotRequired[str | None]
    configured: NotRequired[Literal["*"] | list[str] | None]
    event: NotRequired[JsonObject]


class Input(TypedDict):
    ids: list[str]
    steps: list[Step]


class ProviderError(Exception): ...


class AccessDeniedException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ConflictException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class InternalServerException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ModelErrorException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ModelNotReadyException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ModelStreamErrorException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ModelTimeoutException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ResourceNotFoundException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ServiceQuotaExceededException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ServiceUnavailableException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ThrottlingException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ValidationException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class UnknownThrottlingException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


PROVIDER_ERRORS = (
    AccessDeniedException,
    ConflictException,
    InternalServerException,
    ModelErrorException,
    ModelNotReadyException,
    ModelStreamErrorException,
    ModelTimeoutException,
    ResourceNotFoundException,
    ServiceQuotaExceededException,
    ServiceUnavailableException,
    ThrottlingException,
    ValidationException,
    UnknownThrottlingException,
)


class FakeClient:
    def __init__(
        self,
        scope: dict[str, JsonValue],
        frames: list[str | None],
        send_error: str | None,
    ) -> None:
        self.scope = scope
        self.frames = frames
        self.send_error = send_error
        self.receive_calls = 0
        self.sent: list[str] = []

    async def receive_text(self) -> str:
        self.receive_calls += 1
        frame = self.frames.pop(0) if self.frames else None
        if frame is None:
            raise RuntimeError("the client sent no more frames")
        return frame

    async def send_text(self, data: str) -> None:
        self.sent.append(data)
        if self.send_error is not None:
            raise RuntimeError(self.send_error)

    async def close(self, _code: int = 1000, _reason: str | None = None) -> None: ...


class FakeStream:
    def __init__(self, error: str | None) -> None:
        self.error = error
        self.awaited = 0

    async def await_output(self) -> tuple[None, None]:
        self.awaited += 1
        if self.error is not None:
            raise RuntimeError(self.error)
        return (None, None)

    @property
    def input_stream(self) -> None:
        return None


def scope_of(scope: Scope) -> dict[str, JsonValue]:
    return {
        PENDING_KEY: scope["pending_session_update"],
        COMMITTED_KEY: scope["session_committed"],
        FAILURE_KEY: scope["committed_failure"],
    }


def record_of(scope: dict[str, JsonValue]) -> JsonObject:
    return {
        "pending_session_update": scope.get(PENDING_KEY),
        "session_committed": scope.get(COMMITTED_KEY),
        "committed_failure": scope.get(FAILURE_KEY),
    }


def provider_error(class_name: str, message: str) -> ProviderError:
    error_class = next(error for error in PROVIDER_ERRORS if error.__name__ == class_name)
    return error_class(message)


def dispatch_step(config: RealtimeConfig, step: Step, logging_obj: Logging) -> JsonObject:
    state = StateAdapter.validate_python(step["state"])
    try:
        response = config.transform_realtime_response(step["message"], MODEL, logging_obj, state)
    except ValidationError:
        return {"response": None, "raised": "invalid_payload"}
    except AttributeError:
        return {"response": None, "raised": "not_an_object"}
    except TypeError:
        return {"response": None, "raised": "event_not_an_object"}
    return {"response": response, "raised": None}


async def collect_messages(client: FakeClient, initial: str | None) -> JsonObject:
    generator = bedrock_handler._client_messages(client, initial)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportArgumentType]  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L167-L175
    messages = [message async for message in generator]
    after_end: str | None = await anext(generator, None)
    result: JsonObject = {
        "messages": JsonAdapter.validate_python(messages),
        "receive_calls": client.receive_calls,
        "after_end": after_end,
    }
    return result


def client_messages_step(
    _config: RealtimeConfig, step: Step, _logging_obj: Logging
) -> JsonObject:
    client = FakeClient({}, list(step["received"]), None)
    return asyncio.run(collect_messages(client, step["initial"]))


def parse_client_step(_config: RealtimeConfig, step: Step, _logging_obj: Logging) -> JsonObject:
    parsed = bedrock_handler._parse_client_message(step["message"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L201-L205
    return {"parsed": JsonAdapter.validate_python(dict(parsed))}


def pending_step(_config: RealtimeConfig, step: Step, _logging_obj: Logging) -> JsonObject:
    scope = scope_of(step["scope"])
    try:
        pending = bedrock_handler._pending_session_update(scope)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L178-L190
    except BedrockError as error:
        return {"pending": None, "status": error.status_code, "message": str(error)}
    return {"pending": pending, "status": None, "message": None}


def provider_step(_config: RealtimeConfig, step: Step, _logging_obj: Logging) -> JsonObject:
    failure = provider_error(step["class_name"], step["message"])
    scope = scope_of(step["scope"])
    try:
        bedrock_handler._raise_provider_failure(scope, failure)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportArgumentType]  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L193-L198
    except BedrockError as error:
        recorded = scope.get(FAILURE_KEY)
        return {"status": error.status_code, "message": str(error), "recorded": recorded, "unchanged": False}
    except ProviderError as error:
        recorded = scope.get(FAILURE_KEY)
        return {"status": None, "message": str(error), "recorded": recorded, "unchanged": True}


def ack_step(config: RealtimeConfig, step: Step, _logging_obj: Logging) -> JsonObject:
    scope = scope_of(step["scope"])
    client = FakeClient(scope, [], step["send_error"])
    stream = FakeStream(step["stream_error"])
    trace_id = step["trace_id"]
    logging_obj = None if trace_id is None else make_logging(trace_id)
    try:
        acked = asyncio.run(bedrock_handler._ack_session_update(client, stream, config, MODEL, logging_obj, step["client_message"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportArgumentType]  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L208-L232
    except RuntimeError:
        return {"acked": None, "raised": "stream", "scope": record_of(scope), "sent": None}
    except ValidationError:
        return {"acked": None, "raised": "modalities", "scope": record_of(scope), "sent": None}
    sent: JsonValue = None
    if client.sent:
        sent = JsonAdapter.validate_json(client.sent[0])
    result: JsonObject = {
        "acked": acked,
        "raised": None,
        "scope": record_of(scope),
        "sent": sent,
    }
    return result


def should_log_step(_config: RealtimeConfig, step: Step, _logging_obj: Logging) -> JsonObject:
    litellm.logged_real_time_event_types = step["configured"]
    logged = bedrock_handler._should_log_event(step["event"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L78-L86
    return {"logged": logged}


StepHandler: TypeAlias = Callable[[RealtimeConfig, Step, Logging], JsonObject]

HANDLERS: dict[str, StepHandler] = {
    "dispatch": dispatch_step,
    "client_messages": client_messages_step,
    "parse_client_message": parse_client_step,
    "pending": pending_step,
    "provider_failure": provider_step,
    "ack": ack_step,
    "should_log": should_log_step,
}


def run(input_value: Input) -> JsonValue:
    fastuuid.uuid4 = FixedFast(input_value["ids"])
    config = CONFIG_TYPE()
    logging_obj = make_logging("dispatch")
    steps: list[JsonValue] = [
        HANDLERS[step["method"]](config, step, logging_obj)
        for step in input_value["steps"]
    ]
    return {"steps": steps}
