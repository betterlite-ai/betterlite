from logging import Logger, getLogger
from typing import Literal, NotRequired, TypeAlias, TypedDict
from unittest.mock import patch

from fastapi import HTTPException
from litellm.proxy import proxy_server
from litellm.proxy._types import UpdateKeyRequest, UserAPIKeyAuth
from litellm.proxy.auth.auth_utils import (
    enforce_batch_enqueued_token_limit_is_admin_only,
    enforce_output_token_estimates_are_admin_only,
    get_estimated_output_tokens,
    get_key_mcp_rpm_limit,
    get_key_model_rpm_limit,
    get_key_model_tpm_limit,
    get_key_tag_rpm_limit,
    get_model_rate_limit_from_metadata,
    get_team_mcp_rpm_limit,
)
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError


class DeploymentInput(TypedDict):
    model_name: str
    default_api_key_rpm_limit: NotRequired[JsonValue]
    default_api_key_tpm_limit: NotRequired[JsonValue]


class KeyModelLimitsInput(TypedDict):
    function: Literal["key_model_limits"]
    auth: dict[str, JsonValue]
    model_name: str | None
    deployments: list[DeploymentInput] | None


class EstimatedOutputTokensInput(TypedDict):
    function: Literal["estimated_output_tokens"]
    auth: dict[str, JsonValue]
    model_name: str | None


class RequestInput(TypedDict):
    model_fields_set: list[str]
    metadata: NotRequired[JsonValue]
    default_estimated_output_tokens: NotRequired[JsonValue]
    default_estimated_output_tokens_per_model: NotRequired[JsonValue]


class EnforceOutputEstimatesInput(TypedDict):
    function: Literal["enforce_output_estimates"]
    request: RequestInput
    existing_metadata: dict[str, JsonValue] | None
    caller: dict[str, JsonValue]
    entity: Literal["key", "team"]


class EnforceBatchLimitInput(TypedDict):
    function: Literal["enforce_batch_limit"]
    request: RequestInput
    existing_metadata: dict[str, JsonValue] | None
    caller: dict[str, JsonValue]
    entity: Literal["key", "team"]


class McpRpmLimitInput(TypedDict):
    function: Literal["mcp_rpm_limit"]
    auth: dict[str, JsonValue]
    team_only: bool


class TagRpmLimitInput(TypedDict):
    function: Literal["tag_rpm_limit"]
    auth: dict[str, JsonValue]


class TenantFieldsInput(TypedDict):
    function: Literal["tenant_fields"]
    auth: dict[str, JsonValue]


Input: TypeAlias = (
    KeyModelLimitsInput
    | EstimatedOutputTokensInput
    | EnforceOutputEstimatesInput
    | EnforceBatchLimitInput
    | McpRpmLimitInput
    | TagRpmLimitInput
    | TenantFieldsInput
)


class DeploymentCatalog:
    def __init__(self, deployments: list[DeploymentInput]) -> None:
        self.deployments = deployments

    def get_model_list(self, model_name: str | None = None) -> list[dict[str, JsonValue]]:
        listed: list[dict[str, JsonValue]] = []
        for deployment in self.deployments:
            if model_name is not None and deployment["model_name"] != model_name:
                continue
            params: dict[str, JsonValue] = {}
            rpm = deployment.get("default_api_key_rpm_limit")
            if rpm is not None:
                params["default_api_key_rpm_limit"] = rpm
            tpm = deployment.get("default_api_key_tpm_limit")
            if tpm is not None:
                params["default_api_key_tpm_limit"] = tpm
            listed.append({"model_name": deployment["model_name"], "litellm_params": params})
        return listed


class WarningCapture:
    def __init__(self) -> None:
        self.warnings: list[JsonValue] = []

    def __call__(self, _message: str, field: JsonValue, *_args: JsonValue) -> None:
        self.warnings.append(TypeAdapter(str).validate_python(field))


class Rejection(BaseModel):
    status_code: int
    detail: dict[str, str]


def limits_value(limits: dict[str, int] | None) -> JsonValue:
    if limits is None:
        return None
    return dict[str, JsonValue](limits.items())


def validate_auth(fields: dict[str, JsonValue]) -> UserAPIKeyAuth:
    return TypeAdapter(UserAPIKeyAuth).validate_python(fields)


def update_request(fields: RequestInput) -> UpdateKeyRequest:
    body: dict[str, JsonValue] = {"key": "sk-oracle"}
    model_fields_set = fields["model_fields_set"]
    if "metadata" in model_fields_set:
        body["metadata"] = fields.get("metadata")
    if "default_estimated_output_tokens" in model_fields_set:
        body["default_estimated_output_tokens"] = fields.get("default_estimated_output_tokens")
    if "default_estimated_output_tokens_per_model" in model_fields_set:
        body["default_estimated_output_tokens_per_model"] = fields.get(
            "default_estimated_output_tokens_per_model"
        )
    return TypeAdapter(UpdateKeyRequest).validate_python(body)


def key_model_limits(call: KeyModelLimitsInput) -> dict[str, JsonValue]:
    deployments = call["deployments"]
    catalog = None if deployments is None else DeploymentCatalog(deployments)
    auth = validate_auth(call["auth"])
    with patch.object(proxy_server, "llm_router", catalog):
        rpm = get_key_model_rpm_limit(auth, model_name=call["model_name"])
        tpm = get_key_model_tpm_limit(auth, model_name=call["model_name"])
    return {"rpm": limits_value(rpm), "tpm": limits_value(tpm)}


def estimated_output_tokens(call: EstimatedOutputTokensInput) -> dict[str, JsonValue]:
    auth = validate_auth(call["auth"])
    capture = WarningCapture()
    logger: Logger = getLogger("LiteLLM Proxy")
    with patch.object(logger, "warning", capture):
        estimate = get_estimated_output_tokens(auth, model_name=call["model_name"])
    return {"estimate": estimate, "warnings": capture.warnings}


def rejection(error: HTTPException) -> dict[str, JsonValue]:
    try:
        validated = Rejection.model_validate(error, from_attributes=True)
    except ValidationError:
        return {"ok": False, "status": error.status_code, "detail_error": False}
    return {
        "ok": False,
        "status": validated.status_code,
        "detail_error": "error" in validated.detail,
    }


def enforce_output_estimates(call: EnforceOutputEstimatesInput) -> dict[str, JsonValue]:
    request = update_request(call["request"])
    caller = validate_auth(call["caller"])
    try:
        enforce_output_token_estimates_are_admin_only(
            data=request,
            existing_metadata=call["existing_metadata"],
            user_api_key_dict=caller,
            entity=call["entity"],
        )
    except HTTPException as error:
        return rejection(error)
    return {"ok": True}


def enforce_batch_limit(call: EnforceBatchLimitInput) -> dict[str, JsonValue]:
    request = update_request(call["request"])
    caller = validate_auth(call["caller"])
    try:
        enforce_batch_enqueued_token_limit_is_admin_only(
            data=request,
            existing_metadata=call["existing_metadata"],
            user_api_key_dict=caller,
            entity=call["entity"],
        )
    except HTTPException as error:
        return rejection(error)
    return {"ok": True}


def tenant_report(call: TenantFieldsInput) -> dict[str, JsonValue]:
    auth = validate_auth(call["auth"])
    return {
        "team_rpm": limits_value(
            get_model_rate_limit_from_metadata(auth, "team_metadata", "model_rpm_limit")
        ),
        "organization_itpm": limits_value(
            get_model_rate_limit_from_metadata(auth, "organization_metadata", "model_itpm_limit")
        ),
        "organization_otpm": limits_value(
            get_model_rate_limit_from_metadata(auth, "organization_metadata", "model_otpm_limit")
        ),
        "project_tpm": limits_value(
            get_model_rate_limit_from_metadata(auth, "project_metadata", "model_tpm_limit")
        ),
        "key_mcp": limits_value(get_key_mcp_rpm_limit(auth)),
        "team_mcp": limits_value(get_team_mcp_rpm_limit(auth)),
        "tag": limits_value(get_key_tag_rpm_limit(auth)),
    }


def mcp_rpm_limit(call: McpRpmLimitInput) -> dict[str, JsonValue]:
    auth = validate_auth(call["auth"])
    limits = get_team_mcp_rpm_limit(auth) if call["team_only"] else get_key_mcp_rpm_limit(auth)
    return {"limits": limits_value(limits)}


def tag_rpm_limit(call: TagRpmLimitInput) -> dict[str, JsonValue]:
    auth = validate_auth(call["auth"])
    return {"limits": limits_value(get_key_tag_rpm_limit(auth))}


def key_limit_lookup(
    call: McpRpmLimitInput | TagRpmLimitInput | TenantFieldsInput,
) -> dict[str, JsonValue]:
    if call["function"] == "mcp_rpm_limit":
        return mcp_rpm_limit(call)
    if call["function"] == "tag_rpm_limit":
        return tag_rpm_limit(call)
    return tenant_report(call)


def run(input_value: Input) -> dict[str, JsonValue]:
    match input_value["function"]:
        case "key_model_limits":
            return key_model_limits(input_value)
        case "estimated_output_tokens":
            return estimated_output_tokens(input_value)
        case "enforce_output_estimates":
            return enforce_output_estimates(input_value)
        case "enforce_batch_limit":
            return enforce_batch_limit(input_value)
        case "mcp_rpm_limit" | "tag_rpm_limit" | "tenant_fields":
            return key_limit_lookup(input_value)
