from typing import Literal

from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class GuardrailsAIGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    api_base: str | None = None
    guard_name: str | None = None
    guardrails_ai_api_input_format: Literal["inputs", "llmOutput"] | None = "llmOutput"

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Guardrails AI"
