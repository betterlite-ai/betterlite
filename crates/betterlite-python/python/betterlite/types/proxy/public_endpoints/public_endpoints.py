from collections.abc import Mapping, Sequence
from typing import ClassVar, Literal

from pydantic import BaseModel, ConfigDict, JsonValue


class PublicModelHubInfo(BaseModel):
    docs_title: str
    custom_docs_description: str | None
    litellm_version: str
    useful_links: dict[str, str | dict[str, JsonValue]] | None


class ProviderCredentialField(BaseModel):
    key: str
    label: str
    placeholder: str | None = None
    tooltip: str | None = None
    required: bool = False
    field_type: Literal["text", "password", "select", "upload", "textarea"] = "text"
    options: list[str] | None = None
    default_value: str | None = None


class ProviderCreateInfo(BaseModel):
    provider: str
    provider_display_name: str
    litellm_provider: str
    credential_fields: list[ProviderCredentialField]
    default_model_placeholder: str | None = None


class AgentCredentialField(BaseModel):
    key: str
    label: str
    placeholder: str | None = None
    tooltip: str | None = None
    required: bool = False
    field_type: Literal["text", "password", "select", "upload", "textarea"] = "text"
    options: list[str] | None = None
    default_value: str | None = None
    include_in_litellm_params: bool | None = None
    validation_pattern: str | None = None
    validation_message: str | None = None


class AgentCreateInfo(BaseModel):
    agent_type: str
    agent_type_display_name: str
    description: str | None = None
    logo_url: str | None = None
    credential_fields: list[AgentCredentialField]
    litellm_params_template: dict[str, str] | None = None
    model_template: str | None = None


class EndpointProvider(BaseModel):
    slug: str
    display_name: str


class SupportedEndpoint(BaseModel):
    key: str
    label: str
    endpoint: str
    providers: list[EndpointProvider]


class SupportedEndpointsResponse(BaseModel):
    endpoints: list[SupportedEndpoint]


class AutoRouterPresetTiers(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    SIMPLE: Sequence[str]
    MEDIUM: Sequence[str]
    COMPLEX: Sequence[str]
    REASONING: Sequence[str]


class AutoRouterPresetConfig(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")

    tiers: AutoRouterPresetTiers


class AutoRouterPresetRecord(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")

    label: str
    description: str
    complexity_router_config: AutoRouterPresetConfig


class ComplexityScorerDefaults(BaseModel):
    tier_boundaries: Mapping[str, float]
    token_thresholds: Mapping[str, int]
    dimension_weights: Mapping[str, float]
