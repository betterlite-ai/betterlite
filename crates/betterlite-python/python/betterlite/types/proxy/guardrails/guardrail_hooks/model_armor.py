from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class ModelArmorGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    template_id: str | None = None
    project_id: str | None = None
    location: str | None = None
    credentials: str | None = None
    api_endpoint: str | None = None
    fail_on_error: bool | None = True
    sanitize_error_detail: bool | None = True

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Google Cloud Model Armor"
