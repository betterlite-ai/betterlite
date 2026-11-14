from typing import Literal

from pydantic import BaseModel, Field
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class PanwPrismaAirsGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    api_key: str | None = None
    api_base: str | None = None
    profile_name: str | None = None
    app_name: str | None = None
    mask_on_block: bool = False
    mask_request_content: bool = False
    mask_response_content: bool = False
    fallback_on_error: Literal["block", "allow"] = "block"
    timeout: float = Field(default=10.0, ge=1.0, le=60.0)
    # ast-grep-ignore: no-long-names
    experimental_use_latest_role_message_only: bool | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "PANW Prisma AIRS"
