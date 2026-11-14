from typing import Literal

from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class QualifireGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    api_key: str | None = None
    api_base: str | None = None
    evaluation_id: str | None = None
    prompt_injections: bool | None = None
    hallucinations_check: bool | None = None
    grounding_check: bool | None = None
    pii_check: bool | None = None
    content_moderation_check: bool | None = None
    tool_selection_quality_check: bool | None = None
    assertions: list[str] | None = None
    on_flagged: Literal["block", "monitor"] | None = "block"

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Qualifire"
