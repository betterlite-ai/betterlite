from typing import Literal

from pydantic import BaseModel, JsonValue
from typing_extensions import TypedDict, override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class RepelloAIGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    api_key: str | None = None
    api_base: str | None = None
    asset_id: str | None = None
    unreachable_fallback: Literal["fail_closed", "fail_open"] = "fail_closed"

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "RepelloAI Argus"


class RepelloAIScanData(TypedDict, total=False):
    prompt: str | None
    response: str | None


class RepelloAIAnalyzeRequest(TypedDict, total=False):
    asset_id: str
    scan_data: RepelloAIScanData


class RepelloAIViolatedPolicy(TypedDict, total=False):
    policy_name: str | None
    policy_id: str | None
    action_taken: str | None
    scope: str | None
    details: dict[str, JsonValue] | None
    masked_result: str | None


class RepelloAIAnalyzeResponse(TypedDict, total=False):
    verdict: str | None
    request_id: str | None
    policies_violated: list[RepelloAIViolatedPolicy] | None
    policies_applied: list[dict[str, JsonValue]] | None
