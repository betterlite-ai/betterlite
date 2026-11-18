from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.realtime.transformation import BedrockRealtimeConfig
from litellm.types.realtime import RealtimeResponseTransformInput
from pydantic import JsonValue, TypeAdapter

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


class DispatchStep(TypedDict):
    method: Literal["dispatch"]
    message: str
    state: JsonObject


class ClientMessagesStep(TypedDict):
    method: Literal["client_messages"]
    initial: str | None
    received: list[str | None]


class ParseClientStep(TypedDict):
    method: Literal["parse_client_message"]
    message: str


class PendingStep(TypedDict):
    method: Literal["pending"]
    scope: Scope


class ProviderStep(TypedDict):
    method: Literal["provider_failure"]
    scope: Scope
    class_name: str
    message: str


class AckStep(TypedDict):
    method: Literal["ack"]
    scope: Scope
    client_message: JsonObject
    trace_id: str | None
    stream_error: str | None
    send_error: str | None


class ShouldLogStep(TypedDict):
    method: Literal["should_log"]
    configured: Literal["*"] | list[str] | None
    event: JsonObject


Step: TypeAlias = (
    DispatchStep
    | ClientMessagesStep
    | ParseClientStep
    | PendingStep
    | ProviderStep
    | AckStep
    | ShouldLogStep
)


class Input(TypedDict):
    ids: list[str]
    steps: list[Step]
