from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class DeepKeepGuardrailConfigModelOptionalParams(BaseModel):
    unreachable_fallback: str | None = "fail_closed"


class DeepKeepGuardrailConfigModel(
    # ast-grep-ignore: no-long-names
    GuardrailConfigModel[DeepKeepGuardrailConfigModelOptionalParams],
):
    api_key: str | None = None
    api_base: str | None = None
    deepkeep_firewall_id: str | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "DeepKeep AI Firewall"
