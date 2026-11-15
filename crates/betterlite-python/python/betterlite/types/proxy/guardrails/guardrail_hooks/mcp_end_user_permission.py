from pydantic import BaseModel
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


# ast-grep-ignore: no-long-names
class MCPEndUserPermissionGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "MCP End User Permission"
