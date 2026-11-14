from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class PillarGuardrailConfigModelOptionalParams(BaseModel):
    on_flagged_action: str | None = "monitor"
    async_mode: bool | None = None
    persist_session: bool | None = None
    include_scanners: bool | None = True
    include_evidence: bool | None = True
    fallback_on_error: str | None = None
    timeout: float | None = None


# ast-grep-ignore: no-long-names
class PillarGuardrailConfigModel(GuardrailConfigModel[PillarGuardrailConfigModelOptionalParams]):
    api_key: str | None = None
    api_base: str | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Pillar Guardrail"
