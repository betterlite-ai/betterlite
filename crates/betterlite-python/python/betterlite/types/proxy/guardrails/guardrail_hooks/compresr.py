from typing import Literal

from pydantic import BaseModel, JsonValue
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class CompresrGuardrailOptionalParams(BaseModel):
    target_compression_ratio: float | None = None
    coarse: bool | None = None
    min_chars_to_compress: int | None = None
    compress_tool_outputs: bool | None = None
    compress_system: bool | None = None
    compress_history: bool | None = None
    compress_last_user: bool | None = None
    enable_retrieval: bool | None = None
    max_bytes_per_call: int | None = None
    allow_bypass_header: bool | None = None
    dynamic: bool | None = None
    dynamic_min_ratio: float | None = None
    dynamic_max_ratio: float | None = None
    compression_params: dict[str, JsonValue] | None = None


# ast-grep-ignore: no-long-names
class CompresrGuardrailConfigModel(GuardrailConfigModel[CompresrGuardrailOptionalParams]):
    api_key: str | None = None
    api_base: str | None = None
    model: str | None = None
    unreachable_fallback: Literal["fail_closed", "fail_open"] = "fail_closed"

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Compresr (context compression)"
