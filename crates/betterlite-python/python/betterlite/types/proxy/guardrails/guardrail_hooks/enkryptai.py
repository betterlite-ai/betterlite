from pydantic import BaseModel, JsonValue
from typing_extensions import TypedDict, override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class EnkryptAIPolicyViolationDetail(TypedDict, total=False):
    violating_policy: str
    explanation: str


class EnkryptAIPIIDetail(TypedDict, total=False):
    pii: dict[str, JsonValue]


class EnkryptAIToxicityDetail(TypedDict, total=False):
    toxic: float | None
    severe_toxic: float | None
    obscene: float | None
    threat: float | None
    insult: float | None
    identity_hate: float | None


class EnkryptAIKeywordDetail(TypedDict, total=False):
    detected_keywords: list[str]


class EnkryptAIBiasDetail(TypedDict, total=False):
    bias_detected: bool


class EnkryptAIResponseSummary(TypedDict, total=False):
    toxicity: list[str] | None
    policy_violation: int | None
    pii: int | None
    keyword_detected: int | None
    bias: int | None
    prompt_injection: int | None
    jailbreak: int | None


class EnkryptAIResponseDetails(TypedDict, total=False):
    policy_violation: EnkryptAIPolicyViolationDetail | None
    pii: EnkryptAIPIIDetail | None
    toxicity: EnkryptAIToxicityDetail | None
    keyword_detected: EnkryptAIKeywordDetail | None
    bias: EnkryptAIBiasDetail | None
    prompt_injection: dict[str, JsonValue] | None
    jailbreak: dict[str, JsonValue] | None


class EnkryptAIResponse(TypedDict, total=False):
    summary: EnkryptAIResponseSummary
    details: EnkryptAIResponseDetails


class EnkryptAIProcessedResult(TypedDict):
    attacks_detected: list[str]
    attack_details: dict[
        str,
        EnkryptAIPolicyViolationDetail
        | EnkryptAIPIIDetail
        | EnkryptAIToxicityDetail
        | EnkryptAIKeywordDetail
        | EnkryptAIBiasDetail
        | dict[str, JsonValue],
    ]


class EnkryptAIGuardrailConfigs(BaseModel):
    api_key: str | None = None
    api_base: str | None = None
    policy_name: str | None = None
    deployment_name: str | None = None
    detectors: dict[str, JsonValue] | None = None
    block_on_violation: bool | None = True


class EnkryptAIGuardrailConfigModel(GuardrailConfigModel[BaseModel], EnkryptAIGuardrailConfigs):
    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "EnkryptAI"
