from collections.abc import Mapping
from typing import NotRequired, Protocol, TypeAlias, TypedDict

from litellm.constants import BEDROCK_INVOKE_PROVIDERS_LITERAL as PROVIDERS_LITERAL

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Provider: TypeAlias = PROVIDERS_LITERAL | None


class BaseOptions(TypedDict):
    non_default: dict[str, Json]
    optional: dict[str, Json]
    config: dict[str, Json]


class ProviderSelection(TypedDict):
    models: list[str]


class UrlCase(TypedDict):
    model: str
    api_base: NotRequired[str | None]
    stream: NotRequired[bool | None]
    optional_params: NotRequired[dict[str, Json]]
    environment: NotRequired[dict[str, str]]


class InvokeUrl(TypedDict):
    cases: list[UrlCase]


class GuardrailCases(TypedDict):
    values: list[Json]


class MetadataMergeCase(TypedDict):
    headers: NotRequired[dict[str, str]]
    optional_params: NotRequired[dict[str, Json]]
    identity: NotRequired[dict[str, str]]
    spend_logs_metadata: NotRequired[dict[str, str]]
    policy_fields: NotRequired[list[str]]


class ErrorCase(TypedDict):
    message: str
    status_code: int
    headers: NotRequired[dict[str, str]]


class MetadataMerge(TypedDict):
    merges: list[MetadataMergeCase]
    error: ErrorCase


class Keys(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: NotRequired[str | None]


class SigningCase(TypedDict):
    credentials: Keys
    api_base: str
    now: int
    headers: NotRequired[dict[str, str | None]]
    optional_params: NotRequired[dict[str, Json]]
    request_data: NotRequired[dict[str, Json]]
    model: NotRequired[str | None]
    stream: NotRequired[bool | None]
    fake_stream: NotRequired[bool | None]
    configured_bearer_token: NotRequired[str | None]
    api_key: NotRequired[str | None]


class PromptCase(TypedDict):
    model: str
    messages: list[dict[str, Json]]
    provider: NotRequired[str | None]
    custom_prompt_dict: NotRequired[dict[str, dict[str, Json]]]


class PromptCases(TypedDict):
    cases: list[PromptCase]


class LlamaTemplate(TypedDict):
    local_assets: NotRequired[bool]
    cases: list[PromptCase]


class Steps(TypedDict):
    base_options: NotRequired[BaseOptions]
    provider_family_selection: NotRequired[ProviderSelection]
    invoke_model_url: NotRequired[InvokeUrl]
    guardrail_headers: NotRequired[GuardrailCases]
    guardrail_metadata_merge: NotRequired[MetadataMerge]
    signing_forwarding: NotRequired[SigningCase]
    prompt_template_selection: NotRequired[PromptCases]
    llama_local_template: NotRequired[LlamaTemplate]


class ErrorView(Protocol):
    status_code: int
    message: str
    headers: Mapping[str, str] | None


class InvokeApi(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self, non_default_params: Params, optional_params: Params, model: str, *, drop_params: bool
    ) -> Params: ...

    def _apply_config_to_params(self, config: Params, inference_params: Params) -> None: ...

    def get_bedrock_invoke_provider(self, model: str) -> Provider: ...

    def get_bedrock_model_id(
        self, optional_params: Params, provider: Provider, model: str
    ) -> str: ...

    def get_complete_url(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L109-L117
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: Params,
        litellm_params: Params,
        *,
        stream: bool | None = None,
    ) -> str: ...

    def _get_aws_region_name(self, optional_params: Params, model: str | None = None) -> str: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        messages: list[Json],
        optional_params: Params,
        litellm_params: Params,
    ) -> dict[str, str]: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str]
    ) -> ErrorView: ...

    def sign_request(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L146-L157
        self,
        headers: dict[str, str | None],
        optional_params: Params,
        request_data: Params,
        api_base: str,
        *,
        api_key: str | None = None,
        model: str | None = None,
        stream: bool | None = None,
        fake_stream: bool | None = None,
    ) -> tuple[dict[str, str], bytes | None]: ...

    def convert_messages_to_prompt(
        self,
        model: str,
        messages: list[dict[str, Json]],
        provider: str | None,
        custom_prompt_dict: dict[str, dict[str, Json]],
    ) -> tuple[str, list[Json] | None]: ...
