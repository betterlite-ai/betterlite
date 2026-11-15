from typing import Literal

from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class BaseOpenAIModerationGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    model: Literal["omni-moderation-latest", "text-moderation-latest"] | None = (
        "omni-moderation-latest"
    )


# ast-grep-ignore: no-long-names
class OpenAIModerationGuardrailConfigModel(BaseOpenAIModerationGuardrailConfigModel):
    api_key: str | None = None
    api_base: str | None = "https://api.openai.com/v1"
    streaming_end_of_stream_only: bool | None = False
    streaming_sampling_rate: int | None = 5

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "OpenAI Moderation"
