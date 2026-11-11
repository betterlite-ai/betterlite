import copy
import os
from base64 import b64encode
from collections.abc import Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from typing import TypeAlias, cast

import boto3
import botocore.auth
import litellm
from botocore.credentials import Credentials
from litellm.constants import BEDROCK_INVOKE_PROVIDERS_LITERAL as PROVIDERS_LITERAL
from litellm.constants import known_tokenizer_config
from litellm.litellm_core_utils.prompt_templates import huggingface_template_handler
from litellm.llms.bedrock import base_aws_llm
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.chat.invoke_transformations import base_invoke_transformation
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.types.llms.bedrock import AwsAuthParams
from litellm.types.llms.openai import AllMessageValues
from pydantic import BaseModel, Field, JsonValue

Json: TypeAlias = JsonValue
Params: TypeAlias = dict[str, JsonValue]
Provider: TypeAlias = PROVIDERS_LITERAL | None
InvokeConfig = base_invoke_transformation.AmazonInvokeConfig

BUILTINS = copy.deepcopy(known_tokenizer_config)


class BaseOptions(BaseModel):
    non_default: dict[str, Json] = Field(default_factory=dict)
    optional: dict[str, Json] = Field(default_factory=dict)
    config: dict[str, Json] = Field(default_factory=dict)


class ProviderSelection(BaseModel):
    models: list[str]


class UrlCase(BaseModel):
    model: str
    api_base: str | None = None
    stream: bool | None = None
    optional_params: dict[str, Json] = Field(default_factory=dict)
    environment: dict[str, str] = Field(default_factory=dict)


class InvokeUrl(BaseModel):
    cases: list[UrlCase]


class GuardrailCases(BaseModel):
    values: list[Json]


class MetadataMergeCase(BaseModel):
    headers: dict[str, str] = Field(default_factory=dict)
    optional_params: dict[str, Json] = Field(default_factory=dict)
    identity: dict[str, str] = Field(default_factory=dict)
    spend_logs_metadata: dict[str, str] = Field(default_factory=dict)
    policy_fields: list[str] = Field(default_factory=list)


class ErrorCase(BaseModel):
    message: str
    status_code: int
    headers: dict[str, str] = Field(default_factory=dict)


class MetadataMerge(BaseModel):
    merges: list[MetadataMergeCase]
    error: ErrorCase


class Keys(BaseModel):
    access_key_id: str
    secret_access_key: str
    session_token: str | None = None


class SigningCase(BaseModel):
    credentials: Keys
    api_base: str
    now: int
    headers: dict[str, str | None] = Field(default_factory=dict)
    optional_params: dict[str, Json] = Field(default_factory=dict)
    request_data: dict[str, Json] = Field(default_factory=dict)
    model: str | None = None
    stream: bool | None = None
    fake_stream: bool | None = None
    configured_bearer_token: str | None = None
    api_key: str | None = None


class PromptCase(BaseModel):
    model: str
    messages: list[dict[str, Json]]
    provider: str | None = None
    custom_prompt_dict: dict[str, dict[str, Json]] = Field(default_factory=dict)


class PromptCases(BaseModel):
    cases: list[PromptCase]


class LlamaTemplate(BaseModel):
    local_assets: bool = False
    cases: list[PromptCase]


class DefaultSecret:
    def __call__(self, _name: str, default: str | None = None) -> str | None:
        return default


class BedrockBearer:
    def __init__(self, token: str | None) -> None:
        self.token = token

    def __call__(self, name: str) -> str | None:
        return self.token if name == "AWS_BEARER_TOKEN_BEDROCK" else None


class Resolver:
    def __init__(self, credentials: Credentials) -> None:
        self.credentials = credentials

    def __call__(self, _auth_params: AwsAuthParams, _region: str | None) -> Credentials:
        return self.credentials


class FrozenClock:
    def __init__(self, frozen: datetime) -> None:
        self.frozen = frozen

    def __call__(self) -> datetime:
        return self.frozen


class SessionStub:
    def __init__(self) -> None:
        self.region_name: str | None = None


def unreachable_template(hf_model_name: str) -> dict[str, Json]:
    del hf_model_name
    return {"status": "failure"}


@contextmanager
def cleared_environment(environment: dict[str, str]) -> Generator[None]:
    original = dict(os.environ)
    os.environ.clear()
    os.environ.update(environment)
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(original)


@contextmanager
def stubbed_session() -> Generator[None]:
    original = boto3.Session
    boto3.Session = SessionStub
    try:
        yield
    finally:
        boto3.Session = original


@contextmanager
def frozen_clock(epoch: int) -> Generator[None]:
    frozen = datetime.fromtimestamp(epoch, tz=UTC).replace(tzinfo=None)
    original = botocore.auth.get_current_datetime
    botocore.auth.get_current_datetime = FrozenClock(frozen)
    try:
        yield
    finally:
        botocore.auth.get_current_datetime = original


@contextmanager
def owned_fields(fields: list[str]) -> Generator[None]:
    original = litellm.bedrock_request_metadata_fields
    litellm.bedrock_request_metadata_fields = fields
    try:
        yield
    finally:
        litellm.bedrock_request_metadata_fields = original


@contextmanager
def signing_harness(bearer: str | None, resolver: Resolver) -> Generator[None]:
    original_secret = base_aws_llm.get_secret
    original_secret_str = base_aws_llm.get_secret_str
    original_session = boto3.Session
    original_resolve = BaseAWSLLM.resolve_credentials
    base_aws_llm.get_secret = DefaultSecret()
    base_aws_llm.get_secret_str = BedrockBearer(bearer)
    boto3.Session = SessionStub
    BaseAWSLLM.resolve_credentials = resolver  # pyright: ignore[reportAttributeAccessIssue]
    try:
        with cleared_environment({}):
            yield
    finally:
        base_aws_llm.get_secret = original_secret
        base_aws_llm.get_secret_str = original_secret_str
        boto3.Session = original_session
        BaseAWSLLM.resolve_credentials = original_resolve


@contextmanager
def unreachable_templates() -> Generator[None]:
    original_config = huggingface_template_handler._get_tokenizer_config  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    original_file = huggingface_template_handler._get_chat_template_file  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    huggingface_template_handler._get_tokenizer_config = unreachable_template  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    huggingface_template_handler._get_chat_template_file = unreachable_template  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        yield
    finally:
        huggingface_template_handler._get_tokenizer_config = original_config  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        huggingface_template_handler._get_chat_template_file = original_file  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def base_options(case: BaseOptions) -> Json:
    config = InvokeConfig()
    mapped = cast(
        Params,
        config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]
            dict(case.non_default), dict(case.optional), "", drop_params=False
        ),
    )
    config._apply_config_to_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        dict(case.config), mapped
    )
    return cast(Json, {"supported": config.get_supported_openai_params(""), "mapped": mapped})


def provider_selection(case: ProviderSelection) -> Json:
    return {
        "providers": [
            InvokeConfig.get_bedrock_invoke_provider(model) for model in case.models
        ]
    }


def invoke_url(case: InvokeUrl) -> Json:
    config = InvokeConfig()
    results = [url_case(config, value) for value in case.cases]
    return {"results": results}


def url_case(config: InvokeConfig, case: UrlCase) -> Json:
    optional = dict(case.optional_params)
    with cleared_environment(dict(case.environment)), stubbed_session():
        try:
            provider = config.get_bedrock_invoke_provider(case.model)
            model_id = BaseAWSLLM.get_bedrock_model_id(  # pyright: ignore[reportUnknownMemberType]
                dict(optional), provider, case.model
            )
            endpoint = config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]
                case.api_base, None, case.model, optional, {}, case.stream
            )
            region = config._get_aws_region_name(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
                optional_params=optional, model=case.model
            )
        except (TypeError, ValueError):
            return {"error": True}
    return {
        "endpoint_url": endpoint,
        "model_id": model_id,
        "provider": provider,
        "region": region,
    }


def guardrail_cases(case: GuardrailCases) -> Json:
    results: list[Json] = []
    for value in case.values:
        try:
            derived = base_invoke_transformation._bedrock_invoke_guardrail_headers(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                value
            )
        except BedrockError as error:
            results.append({"status": error.status_code})
            continue
        results.append(cast(Json, {"headers": derived}))
    return {"results": results}


def metadata_merge(case: MetadataMerge) -> Json:
    config = InvokeConfig()
    merges = [merge_case(config, value) for value in case.merges]
    projected = config.get_error_class(  # pyright: ignore[reportUnknownMemberType]
        case.error.message,
        case.error.status_code,
        dict(case.error.headers),
    )
    projected_headers = cast("dict[str, str] | None", projected.headers)
    headers = {} if projected_headers is None else dict(projected_headers)
    return cast(
        Json,
        {
            "merges": merges,
            "error": {
                "status": projected.status_code,
                "message": projected.message,
                "headers": headers,
            },
        },
    )


def merge_case(config: InvokeConfig, case: MetadataMergeCase) -> Json:
    headers = dict(case.headers)
    optional = dict(case.optional_params)
    bucket: Params = dict(case.identity)
    if case.spend_logs_metadata:
        bucket["spend_logs_metadata"] = dict(case.spend_logs_metadata)
    with owned_fields(list(case.policy_fields)):
        try:
            merged = cast(
                "dict[str, str]",
                config.validate_environment(  # pyright: ignore[reportUnknownMemberType]
                    headers, "", [], optional, {"metadata": bucket}
                ),
            )
        except BedrockError:
            return {"error": True}
    return {"headers": dict(merged)}


def signing_case(case: SigningCase) -> Json:
    credentials = Credentials(
        access_key=case.credentials.access_key_id,
        secret_key=case.credentials.secret_access_key,
        token=case.credentials.session_token,
    )
    config = InvokeConfig()
    with signing_harness(case.configured_bearer_token, Resolver(credentials)), frozen_clock(
        case.now
    ):
        signed = config.sign_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            headers=dict(case.headers),
            optional_params=dict(case.optional_params),
            request_data=dict(case.request_data),
            api_base=case.api_base,
            api_key=case.api_key,
            model=case.model,
            stream=case.stream,
            fake_stream=case.fake_stream,
        )
    signed_headers, body = cast("tuple[dict[str, str], bytes | None]", signed)
    body_bytes = b"" if body is None else body
    return cast(
        Json,
        {
            "method": "POST",
            "url": case.api_base,
            "headers": signed_headers,
            "body": b64encode(body_bytes).decode(),
        },
    )


def prompts(cases: list[PromptCase], *, local_assets: bool) -> Json:
    config = InvokeConfig()
    results = [
        prompt_case(config, case, local_assets=local_assets) for case in cases
    ]
    return {"results": results}


def prompt_case(config: InvokeConfig, case: PromptCase, *, local_assets: bool) -> Json:
    known_tokenizer_config.clear()
    if local_assets:
        known_tokenizer_config.update(copy.deepcopy(BUILTINS))
    with unreachable_templates():
        try:
            rendered = config.convert_messages_to_prompt(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
                case.model,
                cast(list[AllMessageValues], case.messages),
                cast(Provider, case.provider),
                dict(case.custom_prompt_dict),
            )
        except Exception as error:  # noqa: BLE001 - the failure text is compared as a value
            return {"message": str(error)}
    prompt, history = cast("tuple[str, list[Json] | None]", rendered)
    return {"prompt": prompt, "chat_history": history}


def run(input_value: dict[str, Json]) -> Json:
    step, params = next(iter(input_value.items()))
    if step == "base_options":
        result = base_options(BaseOptions.model_validate(params))
    elif step == "provider_family_selection":
        result = provider_selection(ProviderSelection.model_validate(params))
    elif step == "invoke_model_url":
        result = invoke_url(InvokeUrl.model_validate(params))
    elif step == "guardrail_headers":
        result = guardrail_cases(GuardrailCases.model_validate(params))
    elif step == "guardrail_metadata_merge":
        result = metadata_merge(MetadataMerge.model_validate(params))
    elif step == "signing_forwarding":
        result = signing_case(SigningCase.model_validate(params))
    elif step == "prompt_template_selection":
        result = prompts(PromptCases.model_validate(params).cases, local_assets=True)
    elif step == "llama_local_template":
        case = LlamaTemplate.model_validate(params)
        result = prompts(case.cases, local_assets=case.local_assets)
    else:
        raise ValueError(f"unsupported step {step}")
    return result
