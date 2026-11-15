import logging
import re
from collections.abc import Sequence
from typing import ClassVar, Final

from pydantic import (
    ConfigDict,
    Field,
    JsonValue,
    TypeAdapter,
    ValidationError,
    field_validator,
    model_validator,
)
from typing_extensions import override

from betterlite._router_types_config import CONFIGURABLE_CLIENTSIDE_AUTH_PARAMS
from betterlite.types.llms.bedrock.invoke.batches import AwsSessionTag
from betterlite.types.utils.base import InitValue, ResponseObject
from betterlite.types.utils.pricing import CustomPricingLiteLLMParams
from betterlite.types.utils.response import ModelResponse

LOGGER: Final = logging.getLogger("LiteLLM")
_RESERVED_INIT_KEYS: Final = frozenset({"self", "params", "__class__"})
_HTTP_HEADER_NAME_PATTERN: Final = re.compile(r"^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$")
_FORBIDDEN_AFFINITY_HEADERS: Final = frozenset({
    "api-key", "authorization", "connection", "content-length", "content-type", "cookie",
    "host", "keep-alive", "proxy-authenticate", "proxy-authorization", "proxy-connection",
    "set-cookie", "te", "trailer", "transfer-encoding", "upgrade", "www-authenticate",
    "x-api-key", "x-goog-api-key",
})
_DROP_PARAMS_BOOL: Final[TypeAdapter[bool]] = TypeAdapter(bool)


def _validated_flag(value: JsonValue) -> bool | None:
    try:
        return _DROP_PARAMS_BOOL.validate_python(value)
    except ValidationError:
        return None


def _drop_params_value(value: JsonValue) -> bool | str | None:
    match value:
        case bool():
            return value
        case str():
            flag: Final = _validated_flag(value.strip())
            return value if flag is None else flag
        case int() | float() | list() | dict() | None:
            return _validated_flag(value)


def _validate_provider_affinity_header_name(header: str) -> str:  # ast-grep-ignore: no-long-names
    if not _HTTP_HEADER_NAME_PATTERN.fullmatch(header):
        raise ValueError("provider_affinity_header must be a valid HTTP header name")
    if header.lower() in _FORBIDDEN_AFFINITY_HEADERS:
        raise ValueError(
            "provider_affinity_header cannot be an authentication, cookie, or transport header"
        )
    return header


class CredentialLiteLLMParams(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    api_key: str | None = None
    api_base: str | None = None
    api_version: str | None = None
    azure_ad_token: str | None = None
    tenant_id: str | None = None
    client_id: str | None = None
    client_secret: str | None = None
    azure_scope: str | None = None
    azure_username: str | None = None
    azure_password: str | None = None
    vertex_project: str | None = None
    vertex_location: str | None = None
    vertex_credentials: str | dict[str, JsonValue] | None = None
    region_name: str | None = None

    gcs_bucket_name: str | None = None
    bucket_name: str | None = None

    aws_access_key_id: str | None = None
    aws_secret_access_key: str | None = None
    aws_session_token: str | None = None
    aws_region_name: str | None = None
    aws_session_name: str | None = None
    aws_profile_name: str | None = None
    aws_role_name: str | None = None
    aws_web_identity_token: str | None = None
    aws_sts_endpoint: str | None = None
    aws_external_id: str | None = None
    aws_session_tags: Sequence[AwsSessionTag] | None = None
    aws_bedrock_runtime_endpoint: str | None = None
    aws_bedrock_project_id: str | None = None
    s3_bucket_name: str | None = None
    s3_endpoint_url: str | None = None
    s3_region_name: str | None = None
    s3_access_key_id: str | None = None
    s3_secret_access_key: str | None = None
    s3_encryption_key_id: str | None = None
    s3_bucket_owner: str | None = None
    aws_batch_role_arn: str | None = None
    s3_output_bucket_name: str | None = None
    bedrock_tags: list[JsonValue] | None = None
    watsonx_region_name: str | None = None


class GenericLiteLLMParams(CredentialLiteLLMParams, CustomPricingLiteLLMParams):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", arbitrary_types_allowed=True)

    @override
    def __init__(self, /, **data: InitValue) -> None:
        # ResponseObject's self is not positional-only, so a reserved "self" key is dropped here
        data.pop("self", None)
        ResponseObject.__init__(self, **data)

    custom_llm_provider: str | None = None
    tpm: int | None = None
    rpm: int | None = None
    itpm: int | None = None
    otpm: int | None = None
    timeout: float | str | None = None
    stream_timeout: float | str | None = None
    max_retries: int | None = None
    drop_params: bool | str | None = None
    organization: str | None = None
    # ast-grep-ignore: no-long-names
    configurable_clientside_auth_params: CONFIGURABLE_CLIENTSIDE_AUTH_PARAMS = None
    litellm_credential_name: str | None = None
    provider_affinity_header: str | None = None

    litellm_trace_id: str | None = None

    max_file_size_mb: float | None = None

    default_api_key_tpm_limit: int | None = None
    default_api_key_rpm_limit: int | None = None

    max_budget: float | None = None
    budget_duration: str | None = None
    keepalive_seconds: float | None = None
    allow_client_keepalive_override: bool | None = False  # ast-grep-ignore: no-long-names
    use_in_pass_through: bool | None = False
    use_litellm_proxy: bool | None = False
    use_chat_completions_api: bool | None = None
    use_xai_oauth: bool | None = Field(
        default=False,
        description="Use stored xAI OAuth credentials when no xAI API key is configured.",
    )
    merge_reasoning_content_in_choices: bool | None = False  # ast-grep-ignore: no-long-names
    model_info: dict[str, JsonValue] | None = None
    mock_response: str | ModelResponse | Exception | JsonValue | None = None

    tags: list[str] | None = None
    tag_regex: list[str] | None = None

    auto_router_config_path: str | None = None
    auto_router_config: str | None = None
    auto_router_default_model: str | None = None
    auto_router_embedding_model: str | None = None
    auto_router_max_input_chars: int | None = None
    auto_router_routing_compression: str | None = None  # ast-grep-ignore: no-long-names
    auto_router_model_compression: str | None = None

    complexity_router_config: dict[str, JsonValue] | None = None
    complexity_router_default_model: str | None = None  # ast-grep-ignore: no-long-names

    adaptive_router_default_model: str | None = None
    adaptive_router_config: dict[str, JsonValue] | None = None
    quality_router_config: dict[str, JsonValue] | None = None
    quality_router_default_model: str | None = None

    vector_store_id: str | None = None
    milvus_text_field: str | None = None
    milvus_db_name: str | None = None
    milvus_partition_names: list[str] | None = None
    valkey_host: str | None = None
    valkey_port: int | None = None
    valkey_password: str | None = None
    valkey_ssl: bool | None = None
    valkey_text_field: str | None = None
    valkey_embedding_field: str | None = None

    @field_validator("provider_affinity_header")
    @classmethod
    # ast-grep-ignore: no-long-names
    def validate_provider_affinity_header(cls, value: str | None) -> str | None:
        if value is None:
            return None
        return _validate_provider_affinity_header_name(value)

    @model_validator(mode="before")
    @classmethod
    def preprocess_input_data(cls, data: JsonValue) -> JsonValue:
        match data:
            case dict():
                filtered: dict[str, JsonValue] = {
                    key: value for key, value in data.items() if key not in _RESERVED_INIT_KEYS
                }
                retries: Final = filtered.get("max_retries")
                match retries:
                    case str():
                        filtered["max_retries"] = int(retries)
                    case bool() | int() | float() | list() | dict() | None:
                        pass
                return filtered
            case bool() | int() | float() | str() | list() | None:
                pass
        return data

    @field_validator("drop_params", mode="before")
    @classmethod
    def coerce_drop_params(cls, value: JsonValue) -> bool | str | None:
        normalized: Final = _drop_params_value(value)
        if normalized is None and value is not None:
            LOGGER.warning("drop_params=%r is not a flag value, treating it as unset", value)
        return normalized


class LiteLLMParams(GenericLiteLLMParams):
    model: str
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", arbitrary_types_allowed=True)


class UpdateLiteLLMParams(GenericLiteLLMParams):
    model: str | None = None
