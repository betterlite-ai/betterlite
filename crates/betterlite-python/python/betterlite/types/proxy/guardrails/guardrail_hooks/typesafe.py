from typing import Literal

from pydantic import Field
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel
from betterlite.types.utils.base import ResponseObject


# ast-grep-ignore: no-long-names
class TypeSafeGuardrailOptionalParams(ResponseObject):
    relevance_threshold: float | None = Field(default=None, ge=0.0, le=1.0)
    min_chars_to_evaluate: int | None = Field(default=None, ge=0)
    max_result_chars_in_state: int | None = Field(default=None, ge=1)


# ast-grep-ignore: no-long-names
class TypeSafeGuardrailConfigModel(GuardrailConfigModel[TypeSafeGuardrailOptionalParams]):
    api_key: str | None = None
    api_base: str | None = None
    model: str | None = None
    unreachable_fallback: Literal["fail_closed", "fail_open"] = "fail_open"

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "TypeSafe (Jev) Compaction"
