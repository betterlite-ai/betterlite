from enum import Enum, StrEnum
from typing import ClassVar, Final, Required, TypedDict

from pydantic import ConfigDict, JsonValue
from typing_extensions import override

from betterlite.types.utils.base import ResponseObject


class SupportedGuardrailIntegrations(Enum):
    APORIA = "aporia"
    BEDROCK = "bedrock"
    DYNAMOAI = "dynamoai"
    GUARDRAILS_AI = "guardrails_ai"
    LAKERA = "lakera"
    LAKERA_V2 = "lakera_v2"
    PRESIDIO = "presidio"
    HIDE_SECRETS = "hide-secrets"
    HIDDENLAYER = "hiddenlayer"
    AIM = "aim"
    CATO_NETWORKS = "cato_networks"
    PANGEA = "pangea"
    CROWDSTRIKE_AIDR = "crowdstrike_aidr"
    LASSO = "lasso"
    PILLAR = "pillar"
    GRAYSWAN = "grayswan"
    PANW_PRISMA_AIRS = "panw_prisma_airs"
    CISCO_AI_DEFENSE = "cisco_ai_defense"
    AZURE_PROMPT_SHIELD = "azure/prompt_shield"
    AZURE_TEXT_MODERATIONS = "azure/text_moderations"
    MODEL_ARMOR = "model_armor"
    OPENAI_MODERATION = "openai_moderation"
    NOMA = "noma"
    NOMA_V2 = "noma_v2"
    TOOL_PERMISSION = "tool_permission"
    ZSCALER_AI_GUARD = "zscaler_ai_guard"
    JAVELIN = "javelin"
    ENKRYPTAI = "enkryptai"
    IBM_GUARDRAILS = "ibm_guardrails"
    LITELLM_CONTENT_FILTER = "litellm_content_filter"
    MCP_SECURITY = "mcp_security"
    ONYX = "onyx"
    PROMPTGUARD = "promptguard"
    XECGUARD = "xecguard"
    PROMPT_SECURITY = "prompt_security"
    GENERIC_GUARDRAIL_API = "generic_guardrail_api"
    QUALIFIRE = "qualifire"
    CUSTOM_CODE = "custom_code"
    OVALIX = "ovalix"
    MICROSOFT_PURVIEW = "microsoft_purview"
    SEMANTIC_GUARD = "semantic_guard"
    MCP_END_USER_PERMISSION = "mcp_end_user_permission"
    BLOCK_CODE_EXECUTION = "block_code_execution"
    AKTO = "akto"
    MCP_JWT_SIGNER = "mcp_jwt_signer"
    LLM_AS_A_JUDGE = "llm_as_a_judge"
    DEEPKEEP = "deepkeep"
    QOSTODIAN_NEXUS = "qostodian_nexus"
    RUBRIK = "rubrik"
    VIGIL_GUARD = "vigil_guard"
    REPELLOAI = "repelloai"
    SINGULR = "singulr"
    HEADROOM = "headroom"
    COMPRESR = "compresr"
    TYPESAFE = "typesafe"
    STRAIKER = "straiker"
    ALICE = "alice"
    AGENT_365 = "agent_365"
    CONDUCT = "conduct"
    DECISION_MODEL = "decision_model"


class Role(StrEnum):
    SYSTEM = "system"
    ASSISTANT = "assistant"
    USER = "user"


default_roles: Final[list[Role]] = [Role.SYSTEM, Role.ASSISTANT, Role.USER]


class GuardrailItemSpec(TypedDict, total=False):
    callbacks: Required[list[str]]
    default_on: bool
    logging_only: bool | None
    enabled_roles: list[Role] | None
    callback_args: dict[str, dict[str, JsonValue]]


class GuardrailItem(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", use_enum_values=True)

    callbacks: list[str]
    default_on: bool
    logging_only: bool | None
    guardrail_name: str
    callback_args: dict[str, dict[str, JsonValue]]
    enabled_roles: list[Role] | None

    @override
    def __init__(
        self,
        callbacks: list[str],
        guardrail_name: str,
        default_on: bool = False,
        logging_only: bool | None = None,
        enabled_roles: list[Role] | None = default_roles,
        callback_args: dict[str, dict[str, JsonValue]] | None = None,
    ) -> None:
        super().__init__(
            callbacks=callbacks,
            default_on=default_on,
            logging_only=logging_only,
            guardrail_name=guardrail_name,
            enabled_roles=enabled_roles,
            callback_args={} if callback_args is None else callback_args,
        )


class LakeraCategoryThresholds(TypedDict, total=False):
    prompt_injection: float
    jailbreak: float
