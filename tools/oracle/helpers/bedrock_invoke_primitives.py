import copy
import os
from base64 import b64encode
from collections.abc import Callable, Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from functools import partial
from types import SimpleNamespace
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import botocore.auth
import litellm
from botocore.credentials import Credentials
from litellm.constants import BEDROCK_INVOKE_PROVIDERS_LITERAL, known_tokenizer_config
from litellm.litellm_core_utils.prompt_templates import huggingface_template_handler
from litellm.llms.bedrock import base_aws_llm
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.chat.invoke_transformations import base_invoke_transformation
from litellm.llms.bedrock.common_utils import BedrockError

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Provider: TypeAlias = BEDROCK_INVOKE_PROVIDERS_LITERAL | None
InvokeConfig = base_invoke_transformation.AmazonInvokeConfig

BUILTINS = copy.deepcopy(known_tokenizer_config)


def object_of(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_of(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_of(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def int_of(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def optional_string(fields: dict[str, Json], name: str) -> str | None:
    value = fields.get(name)
    return value if isinstance(value, str) else None


def optional_bool(fields: dict[str, Json], name: str) -> bool | None:
    value = fields.get(name)
    return value if isinstance(value, bool) else None


def strings_of(value: Json, name: str) -> dict[str, str]:
    return {
        key: string_of(entry, f"{name}.{key}")
        for key, entry in object_of(value, name).items()
    }


def nullable_strings_of(value: Json, name: str) -> dict[str, str | None]:
    return {
        key: entry if isinstance(entry, str) else None
        for key, entry in object_of(value, name).items()
    }


def base_options(params: Json) -> Json:
    fields = object_of(params, "base_options")
    config = InvokeConfig()
    mapped = cast(
        "Params",
        config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]
            object_of(fields.get("non_default", {}), "non_default"),
            dict(object_of(fields.get("optional", {}), "optional")),
            "",
            drop_params=False,
        ),
    )
    config._apply_config_to_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        object_of(fields.get("config", {}), "config"), mapped
    )
    return cast(Json, {"supported": config.get_supported_openai_params(""), "mapped": mapped})


def provider_selection(params: Json) -> Json:
    fields = object_of(params, "provider_family_selection")
    return {
        "providers": [
            InvokeConfig.get_bedrock_invoke_provider(string_of(model, "model"))
            for model in list_of(fields["models"], "models")
        ]
    }


def invoke_url(params: Json) -> Json:
    fields = object_of(params, "invoke_model_url")
    config = InvokeConfig()
    results = [
        url_case(config, object_of(value, "case"))
        for value in list_of(fields["cases"], "cases")
    ]
    return {"results": results}


def url_case(config: InvokeConfig, case: dict[str, Json]) -> Json:
    model = string_of(case["model"], "model")
    optional = dict(object_of(case.get("optional_params", {}), "optional_params"))
    environment = strings_of(case.get("environment", {}), "environment")
    api_base = optional_string(case, "api_base")
    with (
        patch.dict(os.environ, environment, clear=True),
        patch("boto3.Session", lambda: SimpleNamespace(region_name=None)),
    ):
        try:
            provider = config.get_bedrock_invoke_provider(model)
            model_id = BaseAWSLLM.get_bedrock_model_id(  # pyright: ignore[reportUnknownMemberType]
                dict(optional), provider, model
            )
            endpoint = config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]
                api_base, None, model, optional, {}, optional_bool(case, "stream")
            )
            region = config._get_aws_region_name(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
                optional_params=optional, model=model
            )
        except (TypeError, ValueError):
            return {"error": True}
    return {
        "endpoint_url": endpoint,
        "model_id": model_id,
        "provider": provider,
        "region": region,
    }


def guardrail_cases(params: Json) -> Json:
    fields = object_of(params, "guardrail_headers")
    results: list[Json] = []
    for value in list_of(fields["values"], "values"):
        try:
            derived = base_invoke_transformation._bedrock_invoke_guardrail_headers(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                value
            )
        except BedrockError as error:
            results.append({"status": error.status_code})
            continue
        results.append(cast(Json, {"headers": derived}))
    return {"results": results}


def metadata_merge(params: Json) -> Json:
    fields = object_of(params, "guardrail_metadata_merge")
    config = InvokeConfig()
    merges = [
        merge_case(config, object_of(value, "case"))
        for value in list_of(fields["merges"], "merges")
    ]
    error = object_of(fields["error"], "error")
    projected = config.get_error_class(  # pyright: ignore[reportUnknownMemberType]
        string_of(error["message"], "message"),
        int_of(error["status_code"], "status_code"),
        strings_of(error.get("headers", {}), "headers"),
    )
    projected_headers = cast("dict[str, str] | None", projected.headers)
    return {
        "merges": merges,
        "error": {
            "status": projected.status_code,
            "message": projected.message,
            "headers": dict(projected_headers or {}),
        },
    }


def merge_case(config: InvokeConfig, case: dict[str, Json]) -> Json:
    headers = strings_of(case.get("headers", {}), "headers")
    optional = dict(object_of(case.get("optional_params", {}), "optional_params"))
    bucket: Params = dict(strings_of(case.get("identity", {}), "identity"))
    spend = strings_of(case.get("spend_logs_metadata", {}), "spend_logs_metadata")
    if spend:
        bucket["spend_logs_metadata"] = dict(spend)
    policy_fields = [
        string_of(field, "policy_field")
        for field in list_of(case.get("policy_fields", []), "policy_fields")
    ]
    with patch.object(litellm, "bedrock_request_metadata_fields", policy_fields):
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


class Resolver:
    def __init__(self, credentials: Credentials) -> None:
        self.credentials = credentials

    def __call__(self, _auth_params: object, _region: object) -> Credentials:
        return self.credentials


def secret_lookup(_name: str, default: str | None = None) -> str | None:
    return default


def bearer_lookup(name: str, token: str | None) -> str | None:
    return token if name == "AWS_BEARER_TOKEN_BEDROCK" else None


@contextmanager
def clock(epoch: int) -> Generator[None]:
    frozen = datetime.fromtimestamp(epoch, tz=UTC).replace(tzinfo=None)
    with patch.object(botocore.auth, "get_current_datetime", lambda: frozen):
        yield


@contextmanager
def harness(bearer: str | None, resolver: Resolver) -> Generator[None]:
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(base_aws_llm, "get_secret", secret_lookup),
        patch.object(base_aws_llm, "get_secret_str", partial(bearer_lookup, token=bearer)),
        patch("boto3.Session", lambda: SimpleNamespace(region_name=None)),
        patch.object(BaseAWSLLM, "resolve_credentials", resolver),
    ):
        yield


def credentials_of(value: Json) -> Credentials:
    fields = object_of(value, "credentials")
    return Credentials(
        access_key=string_of(fields["access_key_id"], "access_key_id"),
        secret_key=string_of(fields["secret_access_key"], "secret_access_key"),
        token=optional_string(fields, "session_token"),
    )


def signing_case(params: Json) -> Json:
    fields = object_of(params, "signing_forwarding")
    api_base = string_of(fields["api_base"], "api_base")
    headers = nullable_strings_of(fields.get("headers", {}), "headers")
    optional = object_of(fields.get("optional_params", {}), "optional_params")
    request_data = object_of(fields.get("request_data", {}), "request_data")
    config = InvokeConfig()
    bearer = optional_string(fields, "configured_bearer_token")
    context = harness(bearer, Resolver(credentials_of(fields["credentials"])))
    with context, clock(int_of(fields["now"], "now")):
        signed = config.sign_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            headers=headers,
            optional_params=optional,
            request_data=request_data,
            api_base=api_base,
            api_key=optional_string(fields, "api_key"),
            model=optional_string(fields, "model"),
            stream=optional_bool(fields, "stream"),
            fake_stream=optional_bool(fields, "fake_stream"),
        )
    signed_headers, body = cast("tuple[dict[str, str], bytes | None]", signed)
    return cast(
        Json,
        {
            "method": "POST",
            "url": api_base,
            "headers": signed_headers,
            "body": b64encode(body or b"").decode(),
        },
    )


def _unreachable(hf_model_name: str) -> dict[str, Json]:
    del hf_model_name
    return {"status": "failure"}


def prompts(params: Json, *, local_assets: bool) -> Json:
    fields = object_of(params, "prompt_cases")
    config = InvokeConfig()
    results = [
        prompt_case(config, object_of(value, "case"), local_assets=local_assets)
        for value in list_of(fields["cases"], "cases")
    ]
    return {"results": results}


def prompt_case(
    config: InvokeConfig, case: dict[str, Json], *, local_assets: bool
) -> Json:
    model = string_of(case["model"], "model")
    messages = list_of(case["messages"], "messages")
    provider = case.get("provider")
    custom = object_of(case.get("custom_prompt_dict", {}), "custom_prompt_dict")
    known_tokenizer_config.clear()
    if local_assets:
        known_tokenizer_config.update(copy.deepcopy(BUILTINS))
    with (
        patch.object(huggingface_template_handler, "_get_tokenizer_config", _unreachable),
        patch.object(huggingface_template_handler, "_get_chat_template_file", _unreachable),
    ):
        try:
            rendered = config.convert_messages_to_prompt(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
                model,
                cast("list[AllMessageValues]", messages),
                cast(Provider, provider if isinstance(provider, str) else None),
                custom,
            )
        except Exception as error:  # noqa: BLE001 - the failure text is compared as a value
            return {"message": str(error)}
    prompt, history = cast("tuple[str, list[Json] | None]", rendered)
    return {"prompt": prompt, "chat_history": history}


def run(input_value: dict[str, Json]) -> Json:
    step, params = next(iter(input_value.items()))
    handlers: dict[str, Callable[[Json], Json]] = {
        "base_options": base_options,
        "provider_family_selection": provider_selection,
        "invoke_model_url": invoke_url,
        "guardrail_headers": guardrail_cases,
        "guardrail_metadata_merge": metadata_merge,
        "signing_forwarding": signing_case,
    }
    handler = handlers.get(step)
    if handler is not None:
        return handler(params)
    if step == "prompt_template_selection":
        return prompts(params, local_assets=True)
    if step == "llama_local_template":
        fields = object_of(params, "llama_local_template")
        return prompts(params, local_assets=fields.get("local_assets") is True)
    raise ValueError(f"unsupported step {step}")
