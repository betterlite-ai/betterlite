from pydantic import BaseModel, JsonValue
from typing_extensions import TypedDict, override

from betterlite.types.proxy.guardrails.guardrail_hooks.azure.base import (
    AzureContentSafetyConfigModel,
)
from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class AzurePromptShieldGuardrailRequestBody(TypedDict):
    userPrompt: str
    documents: list[str]


class UserPromptAnalysis(TypedDict, total=False):
    attackDetected: bool


# ast-grep-ignore: no-long-names
class AzurePromptShieldGuardrailResponse(TypedDict):
    userPromptAnalysis: UserPromptAnalysis
    documentsAnalysis: list[dict[str, JsonValue]]


# ast-grep-ignore: no-long-names
class AzurePromptShieldGuardrailConfigModel(
    AzureContentSafetyConfigModel,
    GuardrailConfigModel[BaseModel],
):
    cost_tier: str | None = None
    price_per_1000_text_records: float | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Azure Content Safety Prompt Shield"
