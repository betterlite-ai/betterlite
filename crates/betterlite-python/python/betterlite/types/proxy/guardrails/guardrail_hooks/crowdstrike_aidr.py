from pydantic import BaseModel, Field
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class CrowdStrikeAIDRGuardrailConfigModelOptionalParams(BaseModel):
    # ast-grep-ignore: no-long-names
    streaming_buffer_until_moderated: bool | None = None
    # ast-grep-ignore: no-long-names
    streaming_buffer_release_on_scan: bool | None = None
    streaming_end_of_stream_only: bool | None = None
    streaming_sampling_rate: int | None = Field(default=None, ge=1)


# ast-grep-ignore: no-long-names
class CrowdStrikeAIDRGuardrailConfigModel(
    # ast-grep-ignore: no-long-names
    GuardrailConfigModel[CrowdStrikeAIDRGuardrailConfigModelOptionalParams],
):
    api_key: str | None = None
    api_base: str | None = None
    fail_on_error: bool | None = True

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "CrowdStrike AIDR Guardrail"
