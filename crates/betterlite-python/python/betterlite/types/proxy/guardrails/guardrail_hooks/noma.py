from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class NomaGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    use_v2: bool | None = False
    api_key: str | None = None
    api_base: str | None = None
    application_id: str | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Noma Security"


class NomaV2GuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    api_key: str | None = None
    api_base: str | None = None
    application_id: str | None = None
    gateway_name: str | None = None
    monitor_mode: bool | None = None
    block_failures: bool | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Noma Security v2"
