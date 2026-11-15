from typing import Literal

from pydantic import BaseModel, Field
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class ConductGuardrailConfigModelOptionalParams(BaseModel):
    workspace_id: str | None = None
    tool_name: str | None = "llm_call"
    timeout: float | None = Field(default=8.0, gt=0.0)
    unreachable_fallback: Literal["fail_open", "fail_closed"] | None = "fail_closed"


# ast-grep-ignore: no-long-names
class ConductGuardrailConfigModel(GuardrailConfigModel[ConductGuardrailConfigModelOptionalParams]):
    api_key: str = Field(min_length=1)
    api_base: str | None = "https://api.conductai.ai"

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Conduct Guard"
