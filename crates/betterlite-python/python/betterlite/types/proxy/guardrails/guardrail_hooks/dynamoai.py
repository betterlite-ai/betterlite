from enum import StrEnum
from typing import Literal, TypedDict

from pydantic import BaseModel, JsonValue
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class DynamoAIMessage(TypedDict):
    role: str
    content: str


class DynamoRequestMetadata(TypedDict):
    endUserId: str | None


class DynamoTextType(StrEnum):
    MODEL_INPUT = "MODEL_INPUT"
    MODEL_RESPONSE = "MODEL_RESPONSE"


class PolicyMethod(StrEnum):
    PII = "PII"
    TOXICITY = "TOXICITY"
    ALIGNMENT = "ALIGNMENT"
    HALLUCINATION = "HALLUCINATION"
    RAG_HALLUCINATION = "RAG_HALLUCINATION"


class PolicyApplicableTo(StrEnum):
    INPUT = "INPUT"
    OUTPUT = "OUTPUT"
    ALL = "ALL"


class DynamoAIRequest(TypedDict, total=False):
    messages: list[dict[str, JsonValue]]
    textType: DynamoTextType | None
    policyIds: list[str]
    modelId: str | None
    clientId: str | None
    metadata: DynamoRequestMetadata | None


class PolicyInfo(TypedDict, total=False):
    id: str
    name: str
    description: str
    method: PolicyMethod
    action: Literal["BLOCK", "WARN", "REDACT", "SANITIZE", "NONE"]
    methodParams: dict[str, JsonValue]
    decisionParams: dict[str, JsonValue]
    applicableTo: PolicyApplicableTo
    created_at: str
    creatorId: str


class PolicyOutputs(TypedDict, total=False):
    action: Literal["BLOCK", "WARN", "REDACT", "SANITIZE", "NONE"]
    message: str | None


class AppliedPolicyDto(TypedDict, total=False):
    policy: PolicyInfo
    outputs: dict[str, JsonValue] | None
    action: str | None


class DynamoAIResponse(TypedDict, total=False):
    text: str
    textType: DynamoTextType
    finalAction: Literal["BLOCK", "WARN", "REDACT", "SANITIZE", "NONE"]
    appliedPolicies: list[AppliedPolicyDto]
    error: str | None


class DynamoAIProcessedResult(TypedDict):
    violations_detected: list[str]
    violation_details: dict[str, JsonValue]


class DynamoAIGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    api_key: str | None = None
    api_base: str | None = None
    policy_ids: list[str] | None = None
    model_id: str | None = None
    guardrail_name: str | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "DynamoAI Guardrails"
