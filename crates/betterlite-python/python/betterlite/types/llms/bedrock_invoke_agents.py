from typing import Final, TypeAlias

from pydantic import JsonValue
from typing_extensions import TypedDict


class InvokeAgentEventHeaders(TypedDict, total=False):
    event_type: str
    content_type: str
    message_type: str


class InvokeAgentUsage(TypedDict):
    inputTokens: int
    outputTokens: int
    model: str | None


class InvokeAgentMetadata(TypedDict, total=False):
    clientRequestId: str | None
    endTime: str | None
    startTime: str | None
    totalTimeMs: int | None
    usage: InvokeAgentUsage | None


class InvokeAgentModelInvocationInput(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    foundationModel: str | None
    inferenceConfiguration: dict[str, JsonValue] | None
    text: str | None
    traceId: str | None
    type: str | None


class InvokeAgentModelInvocationOutput(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    metadata: InvokeAgentMetadata | None
    parsedResponse: dict[str, JsonValue] | None
    rawResponse: dict[str, JsonValue] | None
    reasoningContent: dict[str, JsonValue] | None
    traceId: str | None


class InvokeAgentOrchestrationTrace(TypedDict, total=False):
    # ast-grep-ignore: no-long-names
    modelInvocationInput: InvokeAgentModelInvocationInput | None
    # ast-grep-ignore: no-long-names
    modelInvocationOutput: InvokeAgentModelInvocationOutput | None


class InvokeAgentPreProcessingTrace(TypedDict, total=False):
    # ast-grep-ignore: no-long-names
    modelInvocationInput: InvokeAgentModelInvocationInput | None
    # ast-grep-ignore: no-long-names
    modelInvocationOutput: InvokeAgentModelInvocationOutput | None


class InvokeAgentTrace(TypedDict, total=False):
    orchestrationTrace: InvokeAgentOrchestrationTrace | None
    preProcessingTrace: InvokeAgentPreProcessingTrace | None


class InvokeAgentCallerChain(TypedDict, total=False):
    agentAliasArn: str


class InvokeAgentTracePayload(TypedDict, total=False):
    agentAliasId: str
    agentId: str
    agentVersion: str
    callerChain: list[InvokeAgentCallerChain]
    eventTime: str
    sessionId: str
    trace: InvokeAgentTrace


class InvokeAgentChunkPayload(TypedDict, total=False):
    bytes: str


class InvokeAgentEventPayload(TypedDict, total=False):
    agentAliasId: str | None
    agentId: str | None
    agentVersion: str | None
    callerChain: list[InvokeAgentCallerChain] | None
    eventTime: str | None
    sessionId: str | None
    trace: InvokeAgentTrace | None
    bytes: str | None


class InvokeAgentEvent(TypedDict, total=False):
    headers: InvokeAgentEventHeaders
    payload: InvokeAgentEventPayload | None


InvokeAgentEventList: TypeAlias = list[InvokeAgentEvent]
InvokeAgentTraceEvent: Final = InvokeAgentEvent
InvokeAgentChunkEvent: Final = InvokeAgentEvent
