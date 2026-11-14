from typing import Final, Literal

from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel

AGENT_365_PROD_API_BASE: Final = "https://agent365.svc.cloud.microsoft"
AGENT_365_PROD_RESOURCE_APP_ID: Final = "ea9ffc3e-8a23-4a7d-836d-234d7c7565c1"
AGENT_365_SCOPE_NAME: Final = "ThreatProtection.Evaluate.All"


class Agent365GuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    tenant_id: str | None = None
    client_id: str | None = None
    client_secret: str | None = None
    unreachable_fallback: Literal["fail_closed", "fail_open"] = "fail_closed"

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Microsoft Agent 365"
