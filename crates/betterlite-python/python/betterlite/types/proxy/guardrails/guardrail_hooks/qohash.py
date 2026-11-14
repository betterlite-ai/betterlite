from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class QostodianNexusConfigModel(GuardrailConfigModel[BaseModel]):
    api_base: str | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Qostodian Nexus"
