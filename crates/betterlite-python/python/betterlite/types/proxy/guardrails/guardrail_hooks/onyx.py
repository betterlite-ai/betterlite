from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class OnyxGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    api_base: str | None = None
    api_key: str | None = None
    timeout: float | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Onyx Guardrail"
