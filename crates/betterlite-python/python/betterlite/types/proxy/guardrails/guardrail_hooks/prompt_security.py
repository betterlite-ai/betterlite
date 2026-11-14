from typing import Literal

from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class PromptSecurityGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    api_key: str | None = None
    api_base: str | None = None
    file_sanitization_fail_open: bool = True
    block_on_file_modify: bool = True
    streaming_transform_mode: Literal["block_only", "incremental_diff"] | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Prompt Security"
