import ssl
from enum import StrEnum
from typing import TypeAlias


class httpxSpecialProvider(StrEnum):  # noqa: N801  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/custom_http.py#L6
    LoggingCallback = "logging_callback"
    GuardrailCallback = "guardrail_callback"
    Caching = "caching"
    Oauth2Check = "oauth2_check"
    Oauth2Register = "oauth2_register"
    SecretManager = "secret_manager"
    PassThroughEndpoint = "pass_through_endpoint"
    PromptFactory = "prompt_factory"
    SSO_HANDLER = "sso_handler"
    Search = "search"
    MCP = "mcp"
    RAG = "rag"
    ProviderBilling = "provider_billing"
    A2AProvider = "a2a_provider"
    AgentHealthCheck = "agent_health_check"
    AgentKillSwitch = "agent_kill_switch"
    A2A = "a2a"
    PromptManagement = "prompt_management"
    UI = "ui"
    ROICalculator = "roi_calculator"
    Sandbox = "sandbox"
    ModelCostMap = "model_cost_map"
    PasswordBreachCheck = "password_breach_check"
    ASGI = "asgi"
    AgentHarness = "agent_harness"


VerifyTypes: TypeAlias = str | bool | ssl.SSLContext
