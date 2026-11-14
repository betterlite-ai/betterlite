from typing import ClassVar, Final, Literal, TypeAlias

from pydantic import BaseModel, ConfigDict, Field, JsonValue
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel

StraikerWebhookEventType: TypeAlias = Literal["pre_call", "post_call"]
StraikerWebhookStreamPhase: TypeAlias = Literal["none", "assembled"]
StraikerWebhookAction: TypeAlias = Literal["NONE", "BLOCKED", "GUARDRAIL_INTERVENED"]

# ast-grep-ignore: no-long-names
STRAIKER_WEBHOOK_SCHEMA_VERSION: Final = "1"


class StraikerWebhookStream(BaseModel):
    phase: StraikerWebhookStreamPhase = "none"
    index: int | None = None


class StraikerWebhookEvent(BaseModel):
    type: StraikerWebhookEventType
    id: str
    stream: StraikerWebhookStream = Field(default_factory=StraikerWebhookStream)


class StraikerWebhookContent(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    texts: list[str] = Field(default_factory=list)
    images: list[str] = Field(default_factory=list)
    structured_messages: list[dict[str, JsonValue]] | None = None
    tools: list[dict[str, JsonValue]] | None = None
    tool_calls: list[dict[str, JsonValue]] | None = None
    finish_reason: str | None = None


class StraikerWebhookUsage(BaseModel):
    input_tokens: int | None = None
    output_tokens: int | None = None


class StraikerWebhookContext(BaseModel):
    call_surface: str
    mode: list[str] | None = None
    model: str | None = None
    model_provider: str | None = None
    destination: str | None = None
    session_id: str | None = None
    litellm_call_id: str | None = None
    litellm_trace_id: str | None = None
    litellm_version: str | None = None


class StraikerWebhookIdentity(BaseModel):
    litellm_key: str | None = None
    litellm_team: str | None = None
    litellm_user_id: str | None = None
    litellm_user_email: str | None = None
    litellm_org_id: str | None = None
    end_user_id: str | None = None


class StraikerWebhookApplication(BaseModel):
    source: str
    name: str | None = None


class StraikerWebhookRequest(BaseModel):
    # ast-grep-ignore: no-long-names
    schema_version: str = STRAIKER_WEBHOOK_SCHEMA_VERSION
    event: StraikerWebhookEvent
    request: StraikerWebhookContent
    response: StraikerWebhookContent | None = None
    context: StraikerWebhookContext
    identity: StraikerWebhookIdentity
    application: StraikerWebhookApplication
    usage: StraikerWebhookUsage | None = None
    metadata: dict[str, JsonValue] | None = None


class StraikerWebhookResponse(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")

    action: StraikerWebhookAction = "NONE"
    blocked_reason: str | None = None
    blocked_by: tuple[str, ...] = ()
    texts: list[str] | None = None
    schema_version: str | None = None
    turn_id: str | None = Field(default=None, alias="turnId")


# ast-grep-ignore: no-long-names
class StraikerGuardrailConfigModelOptionalParams(BaseModel):
    timeout: float | None = Field(default=5.0, gt=0.0)
    max_retries: int | None = Field(default=2, ge=0)
    initial_backoff: float | None = Field(default=0.1, ge=0.0)
    max_backoff: float | None = Field(default=2.0, ge=0.0)
    unreachable_fallback: Literal["fail_open", "fail_closed"] | None = "fail_closed"
    fail_on_error: bool | None = True
    max_payload_bytes: int | None = Field(default=524288, gt=0)
    api_version: Literal["v1", "v3"] | None = None
    agent_ref: str | None = None
    client: str | None = None
    format_hint: Literal["anthropic.messages", "openai.chat"] | None = None
    custom_headers: dict[str, str] | None = None
    metadata: dict[str, str] | None = None
    verbose: bool | None = False


class StraikerGuardrailConfigModel(
    # ast-grep-ignore: no-long-names
    GuardrailConfigModel[StraikerGuardrailConfigModelOptionalParams],
):
    api_key: str = Field(min_length=1, json_schema_extra={"secret": True})
    api_base: str | None = "https://api.prod.straiker.ai"
    default_app: str | None = "LiteLLM Gateway"

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Straiker"
