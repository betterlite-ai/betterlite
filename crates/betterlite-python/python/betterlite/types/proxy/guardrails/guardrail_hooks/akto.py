from typing import Literal

from pydantic import BaseModel, Field
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class AktoConfigModel(GuardrailConfigModel[BaseModel]):
    akto_base_url: str | None = Field(
        default=None,
        json_schema_extra={
            "examples": [
                "http://localhost:9090",
                "https://akto-ingestion.example.com",
            ]
        },
    )
    akto_api_key: str | None = None
    akto_account_id: str | None = None
    akto_vxlan_id: str | None = None
    unreachable_fallback: Literal["fail_closed", "fail_open"] = "fail_closed"
    guardrail_timeout: int | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Akto"
