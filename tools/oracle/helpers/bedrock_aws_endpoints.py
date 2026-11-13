import os
import tempfile
from collections.abc import Callable
from pathlib import Path
from typing import Literal, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.constants import BEDROCK_INVOKE_PROVIDERS_LITERAL
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
EndpointType: TypeAlias = Literal["runtime", "agent", "agentcore"]


def object_of(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_of(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def environment_of(params: Params) -> dict[str, str]:
    return cast(dict[str, str], params.get("environment", {}))


def optional_string(params: Params, name: str) -> str | None:
    value = params.get(name)
    return value if isinstance(value, str) else None


def write_fixture(folder: str, name: str, text: str) -> str:
    directory = Path(folder) / ".aws"
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / name
    path.write_text(text, encoding="utf-8")
    return str(path)


def fixture_environment(params: Params, folder: str) -> dict[str, str]:
    environment = dict(environment_of(params))
    config = params.get("aws_config")
    credentials = params.get("aws_credentials")
    if not isinstance(config, str) and not isinstance(credentials, str):
        return environment
    environment["HOME"] = folder
    if isinstance(config, str):
        environment.setdefault("AWS_CONFIG_FILE", write_fixture(folder, "config", config))
    if isinstance(credentials, str):
        environment.setdefault(
            "AWS_SHARED_CREDENTIALS_FILE", write_fixture(folder, "credentials", credentials)
        )
    return environment


def model_arn_region(params: Params) -> Json:
    instance = BaseAWSLLM()
    return {
        "regions": [
            instance._get_aws_region_from_model_arn(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
                model if isinstance(model, str) else None
            )
            for model in list_of(params["models"], "models")
        ]
    }


def llm_region_name_case(params: Params) -> Json:
    optional = cast(dict[str, Json], params.get("optional_params", {}))
    instance = BaseAWSLLM()
    with tempfile.TemporaryDirectory() as folder:
        environment = fixture_environment(params, folder)
        with patch.dict(os.environ, environment, clear=True):
            try:
                region = instance._get_aws_region_name(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
                    optional,
                    model=optional_string(params, "model"),
                    model_id=optional_string(params, "model_id"),
                )
            except ValueError:
                return {"error": True}
    return {"region": region}


def llm_region_names(params: Params) -> Json:
    return {
        "regions": [
            llm_region_name_case(object_of(case, "case"))
            for case in list_of(params["cases"], "cases")
        ]
    }


def non_llm_region_names(params: Params) -> Json:
    regions: list[Json] = []
    instance = BaseAWSLLM()
    for case in list_of(params["cases"], "cases"):
        fields = object_of(case, "case")
        with patch.dict(os.environ, environment_of(fields), clear=True):
            try:
                region = instance.get_aws_region_name_for_non_llm_api_calls(
                    optional_string(fields, "explicit")
                )
            except ValueError:
                regions.append({"error": True})
                continue
        regions.append({"region": region})
    return {"regions": regions}


def invalid_region(params: Params) -> Json:
    valid: list[Json] = []
    for value in list_of(params["values"], "values"):
        try:
            BaseAWSLLM._validate_aws_region_name(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
                value if isinstance(value, str) else None
            )
            valid.append(True)
        except ValueError:
            valid.append(False)
    return {"valid": valid}


def bedrock_model_ids(params: Params) -> Json:
    results: list[Json] = []
    for case in list_of(params["cases"], "cases"):
        fields = object_of(case, "case")
        optional = dict(object_of(fields.get("optional_params", {}), "optional_params"))
        provider = optional_string(fields, "provider")
        try:
            model_id = BaseAWSLLM.get_bedrock_model_id(  # pyright: ignore[reportUnknownMemberType]
                optional,
                cast(BEDROCK_INVOKE_PROVIDERS_LITERAL | None, provider),
                cast(str, fields["model"]),
            )
        except (TypeError, ValueError):
            results.append({"error": True})
            continue
        results.append({"model_id": model_id, "optional_params": optional})
    return {"results": results}


def provider_selection(params: Params) -> Json:
    return {
        "providers": [
            {
                "invoke": BaseAWSLLM.get_bedrock_invoke_provider(cast(str, model)),
                "embedding": BaseAWSLLM.get_bedrock_embedding_provider(cast(str, model)),
            }
            for model in list_of(params["models"], "models")
        ]
    }


def default_endpoint(params: Params) -> Json:
    instance = BaseAWSLLM()
    urls: list[Json] = []
    for case in list_of(params["cases"], "cases"):
        fields = object_of(case, "case")
        urls.append(
            instance._select_default_endpoint_url(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
                cast(EndpointType | None, fields.get("endpoint_type")),
                cast(str, fields["region"]),
            )
        )
    return {"urls": urls}


def runtime_endpoint(params: Params) -> Json:
    instance = BaseAWSLLM()
    endpoints: list[Json] = []
    for case in list_of(params["cases"], "cases"):
        fields = object_of(case, "case")
        with patch.dict(os.environ, environment_of(fields), clear=True):
            endpoint_url, proxy_endpoint_url = instance.get_runtime_endpoint(
                api_base=optional_string(fields, "api_base"),
                aws_bedrock_runtime_endpoint=optional_string(
                    fields, "aws_bedrock_runtime_endpoint"
                ),
                aws_region_name=cast(str, fields["region"]),
                endpoint_type=cast(EndpointType | None, fields.get("endpoint_type")),
            )
        endpoints.append([endpoint_url, proxy_endpoint_url])
    return {"endpoints": endpoints}


def sts_endpoint_region(params: Params) -> Json:
    regions: list[Json] = []
    for endpoint in list_of(params["endpoints"], "endpoints"):
        try:
            region = BaseAWSLLM._parse_sts_region_from_endpoint(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
                endpoint if isinstance(endpoint, str) else None
            )
        except ValueError:
            regions.append({"error": True})
            continue
        regions.append({"region": region})
    return {"regions": regions}


def sts_kwargs(params: Params) -> Json:
    instance = BaseAWSLLM()
    results: list[Json] = []
    for case in list_of(params["cases"], "cases"):
        fields = object_of(case, "case")
        endpoint = optional_string(fields, "endpoint")
        configured = optional_string(fields, "configured_region")
        with patch.dict(os.environ, environment_of(fields), clear=True):
            region = BaseAWSLLM._resolve_sts_region(endpoint, configured)  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            kwargs = instance._build_sts_client_kwargs(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
                endpoint,
                cast(bool | str | None, fields.get("verify")),
                configured,
            )
        results.append({"region": region, "kwargs": cast(Json, kwargs)})
    return {"results": results}


HANDLERS: dict[str, Callable[[Params], Json]] = {
    "model_arn_region": model_arn_region,
    "llm_region_name": llm_region_names,
    "non_llm_region_name": non_llm_region_names,
    "invalid_region": invalid_region,
    "bedrock_model_id": bedrock_model_ids,
    "provider_selection": provider_selection,
    "default_endpoint": default_endpoint,
    "runtime_endpoint": runtime_endpoint,
    "sts_endpoint_region": sts_endpoint_region,
    "sts_kwargs": sts_kwargs,
}


def run(input_value: dict[str, Json]) -> Json:
    if len(input_value) != 1:
        raise ValueError("bedrock_aws_endpoints takes exactly one operation")
    operation, payload = next(iter(input_value.items()))
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    _ = litellm
    return handler(cast(Params, payload))
