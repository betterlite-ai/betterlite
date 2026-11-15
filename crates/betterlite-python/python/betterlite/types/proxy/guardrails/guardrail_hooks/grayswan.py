from pydantic import BaseModel, Field
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class GraySwanGuardrailConfigModelOptionalParams(BaseModel):
    on_flagged_action: str | None = "passthrough"
    violation_threshold: float | None = Field(default=0.5, ge=0.0, le=1.0)
    reasoning_mode: str | None = None
    policy_id: str | None = None
    categories: dict[str, str] | None = None
    fail_open: bool | None = True
    guardrail_timeout: float | None = 30.0


class GraySwanGuardrailConfigModel(
    # ast-grep-ignore: no-long-names
    GuardrailConfigModel[GraySwanGuardrailConfigModelOptionalParams],
):
    api_key: str | None = None
    api_base: str | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Gray Swan Guardrail"
