from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class OvalixGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    tracker_api_base: str | None = None
    tracker_api_key: str | None = None
    application_id: str | None = None
    pre_checkpoint_id: str | None = None
    post_checkpoint_id: str | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Ovalix Guardrail"
