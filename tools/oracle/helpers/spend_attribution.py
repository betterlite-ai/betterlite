import os
from collections.abc import Callable
from types import SimpleNamespace
from typing import Literal, Protocol, TypeAlias, TypedDict, Unpack, cast

import litellm
from litellm.proxy.spend_tracking.spend_tracking_utils import (
    _deployment_provider,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_tracking_utils.py#L396
    _extract_usage_for_ocr_call,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_tracking_utils.py#L317
    _get_spend_logs_metadata,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_tracking_utils.py#L165
    _is_configured_model_group,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_tracking_utils.py#L423
    _model_group_provider,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_tracking_utils.py#L412
    _redact_logged_api_key,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_tracking_utils.py#L122
    get_request_model_access_groups,
)
from pydantic import JsonValue, RootModel, TypeAdapter

JSON_VALUE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class KeyInput(TypedDict):
    value: str | None
    already_redacted: bool
    key_alias: str | None


class RedactInput(TypedDict):
    function: Literal["redact"]
    values: list[KeyInput]


class ProjectArguments(TypedDict):
    applied_guardrails: list[str] | None
    batch_models: list[str] | None
    batch_successful_requests: int | None
    batch_failed_requests: int | None
    mcp_tool_call_metadata: dict[str, JsonValue] | None
    vector_store_request_metadata: list[dict[str, JsonValue]] | None
    guardrail_information: list[dict[str, JsonValue]] | None
    usage_object: dict[str, JsonValue] | None
    model_map_information: dict[str, JsonValue] | None
    cold_storage_object_key: str | None
    litellm_overhead_time_ms: float | None
    cost_breakdown: dict[str, JsonValue] | None
    litellm_call_id: str | None
    autorouter_savings: float | None
    autorouter_savings_estimate: dict[str, JsonValue] | None
    autorouter_baseline_observation: str | None
    router_metadata: dict[str, JsonValue] | None
    azure_spillover: dict[str, JsonValue] | None
    used_client_oauth_token: bool | None


class MetadataProjector(Protocol):
    def __call__(
        self, metadata: dict[str, JsonValue] | None, **arguments: Unpack[ProjectArguments]
    ) -> JsonValue: ...


class ProjectInput(TypedDict):
    function: Literal["project"]
    metadata: dict[str, JsonValue] | None
    store_bodies: bool
    arguments: ProjectArguments


class AccessInput(TypedDict):
    function: Literal["access_groups"]
    standard_logging_object: dict[str, JsonValue] | None
    metadata: dict[str, JsonValue] | None
    litellm_metadata: dict[str, JsonValue] | None
    auth_groups: list[str] | None


class OcrInput(TypedDict):
    function: Literal["ocr_usage"]
    usage_info: JsonValue
    attribute_present: bool
    attribute_usage_info: JsonValue


class DeploymentInput(TypedDict):
    model_name: str
    litellm_params: dict[str, JsonValue]


class ProvidersInput(TypedDict):
    function: Literal["providers"]
    deployments: list[DeploymentInput]
    model_group: str
    recognized: list[str]
    team_public: list[str]
    env_proxy: bool
    global_proxy: bool


Input: TypeAlias = RedactInput | ProjectInput | AccessInput | OcrInput | ProvidersInput


class RouterStub:
    def __init__(
        self,
        deployments: list[DeploymentInput],
        recognized: frozenset[str],
        team_public: frozenset[str],
    ) -> None:
        self.deployments = deployments
        self.is_recognized_model = recognized.__contains__
        self.team_public_model_names = team_public

    def get_model_list(self, model_name: str) -> list[DeploymentInput]:
        return [
            deployment for deployment in self.deployments if deployment["model_name"] == model_name
        ]



def bucket(entries: dict[str, JsonValue], groups: list[str] | None) -> dict[str, JsonValue]:
    if groups is None or "user_api_key_auth" in entries:
        return {**entries}
    return {**entries, "user_api_key_auth": {"matched_model_access_groups": groups}}


def redact(input_value: RedactInput) -> JsonValue:
    values = [
        _redact_logged_api_key(
            key["value"], already_redacted=key["already_redacted"], key_alias=key["key_alias"]
        )
        for key in input_value["values"]
    ]
    return JSON_VALUE.validate_python(values)


def project(input_value: ProjectInput) -> JsonValue:
    os.environ["STORE_PROMPTS_IN_SPEND_LOGS"] = "true" if input_value["store_bodies"] else "false"
    project_metadata = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_tracking_utils.py#L165-L187
        MetadataProjector,
        _get_spend_logs_metadata,
    )
    return JSON_VALUE.validate_python(
        project_metadata(input_value["metadata"], **input_value["arguments"])
    )


def access_groups(input_value: AccessInput) -> JsonValue:
    params: dict[str, JsonValue] = {}
    metadata = input_value["metadata"]
    if metadata is not None:
        params["metadata"] = bucket(metadata, input_value["auth_groups"])
    litellm_metadata = input_value["litellm_metadata"]
    if litellm_metadata is not None:
        params["litellm_metadata"] = bucket(litellm_metadata, input_value["auth_groups"])
    kwargs: dict[str, JsonValue] = {
        "standard_logging_object": input_value["standard_logging_object"],
        "litellm_params": params,
    }
    return JSON_VALUE.validate_python(list(get_request_model_access_groups(kwargs)))


def ocr_usage(input_value: OcrInput) -> JsonValue:
    responses: dict[str, JsonValue] = {}
    if input_value["usage_info"] is not None:
        responses["usage_info"] = input_value["usage_info"]
    response_obj = SimpleNamespace()
    if input_value["attribute_present"]:
        response_obj = SimpleNamespace(
            usage_info=RootModel[JsonValue](input_value["attribute_usage_info"])
        )
    extract_usage = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_tracking_utils.py#L317
        Callable[[SimpleNamespace, dict[str, JsonValue]], dict[str, JsonValue]],
        _extract_usage_for_ocr_call,
    )
    return JSON_VALUE.validate_python(extract_usage(response_obj, responses))


def providers(input_value: ProvidersInput) -> JsonValue:
    os.environ["USE_LITELLM_PROXY"] = "true" if input_value["env_proxy"] else "false"
    litellm.use_litellm_proxy = input_value["global_proxy"]
    router = RouterStub(
        input_value["deployments"],
        frozenset(input_value["recognized"]),
        frozenset(input_value["team_public"]),
    )
    resolve_deployment = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_tracking_utils.py#L396
        Callable[[DeploymentInput], str | None],
        _deployment_provider,
    )
    resolve_group = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_tracking_utils.py#L412
        Callable[[str, RouterStub], str | None],
        _model_group_provider,
    )
    recognizes_group = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_tracking_utils.py#L423
        Callable[[str, RouterStub], bool],
        _is_configured_model_group,
    )
    resolved = [resolve_deployment(deployment) for deployment in input_value["deployments"]]
    group_provider = resolve_group(input_value["model_group"], router)
    configured = recognizes_group(input_value["model_group"], router)
    return JSON_VALUE.validate_python(
        {"providers": resolved, "group_provider": group_provider, "configured": configured}
    )


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "redact":
        return redact(input_value)
    if input_value["function"] == "project":
        return project(input_value)
    if input_value["function"] == "access_groups":
        return access_groups(input_value)
    if input_value["function"] == "ocr_usage":
        return ocr_usage(input_value)
    return providers(input_value)
