import asyncio
import logging
from typing import Literal, TypedDict

import litellm
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.auth import model_checks
from litellm.proxy.auth.model_checks import (
    get_all_fallbacks,
    get_complete_model_list,
    get_key_models,
    get_mcp_server_ids,
    get_team_models,
)
from litellm.router import Router
from litellm.types.router import LiteLLM_Params
from litellm.types.utils import CredentialItem
from pydantic import JsonValue
from typing_extensions import override


class Credential(TypedDict):
    credential_name: str
    credential_values: dict[str, JsonValue]


class DeployedRow(TypedDict):
    model_name: str
    litellm_params: dict[str, JsonValue]
    team_id: str | None
    team_public_model_name: str | None


class GrantsInput(TypedDict):
    function: Literal["key_models", "team_models"]
    key_models: list[str]
    team_models: list[str]
    proxy_model_list: list[str]
    model_access_groups: dict[str, list[str]]
    include_model_access_groups: bool


class CompleteModelListInput(TypedDict):
    function: Literal["complete_model_list"]
    key_models: list[str]
    team_models: list[str]
    proxy_model_list: list[str]
    user_model: str | None
    infer_model_from_keys: bool
    return_wildcard_routes: bool
    model_access_groups: dict[str, list[str]]
    include_model_access_groups: bool
    only_model_access_groups: bool
    team_id: str | None
    deployments: list[DeployedRow]
    catalog: dict[str, list[str]]
    credentials: list[Credential]


class McpServerIdsInput(TypedDict):
    function: Literal["mcp_server_ids"]
    object_permission_id: str | None


class FallbackSettings(TypedDict):
    fallbacks: list[dict[str, JsonValue]]
    context_window_fallbacks: list[dict[str, JsonValue]]
    content_policy_fallbacks: list[dict[str, JsonValue]]


class AllFallbacksInput(TypedDict):
    function: Literal["all_fallbacks"]
    model: str
    fallback_type: str
    settings: FallbackSettings | None


class FallbacksOutput(TypedDict):
    fallbacks: list[str]
    warnings: list[str]


Input = GrantsInput | CompleteModelListInput | McpServerIdsInput | AllFallbacksInput


class Catalog:
    # B2328's get_valid_models: a lookup with parameters reads the credential-keyed entry
    def __init__(self, models_by_provider: dict[str, list[str]]) -> None:
        self.models_by_provider = models_by_provider

    def __call__(
        self,
        check_provider_endpoint: bool | None = None,
        custom_llm_provider: str | None = None,
        litellm_params: LiteLLM_Params | None = None,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> list[str]:
        del check_provider_endpoint, api_key, api_base
        if custom_llm_provider is None:
            return []
        key = custom_llm_provider
        if litellm_params is not None:
            parts = (
                custom_llm_provider,
                litellm_params.api_key,
                litellm_params.api_base,
                litellm_params.litellm_credential_name,
            )
            key = "|".join(str(part) for part in parts)
        return list(self.models_by_provider[key]) if key in self.models_by_provider else []


class WarningLog(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def run(input_value: Input) -> list[str] | FallbacksOutput:
    litellm.suppress_debug_info = True
    match input_value["function"]:
        case "key_models" | "team_models":
            return grants(input_value)
        case "complete_model_list":
            return complete_model_list(input_value)
        case "mcp_server_ids":
            return mcp_server_ids(input_value)
        case "all_fallbacks":
            return all_fallbacks(input_value)


def grants(call: GrantsInput) -> list[str]:
    if call["function"] == "team_models":
        # LiteLLM builds a Python set here, so the sorted answer is the comparable one
        return sorted(
            get_team_models(
                call["team_models"],
                call["proxy_model_list"],
                call["model_access_groups"],
                include_model_access_groups=call["include_model_access_groups"],
            )
        )
    token = UserAPIKeyAuth(models=call["key_models"], team_models=call["team_models"])
    return get_key_models(
        token,
        call["proxy_model_list"],
        call["model_access_groups"],
        include_model_access_groups=call["include_model_access_groups"],
        only_model_access_groups=False,
    )


def complete_model_list(call: CompleteModelListInput) -> list[str]:
    model_checks.get_valid_models = Catalog(call["catalog"])
    litellm.credential_list = [
        CredentialItem(
            credential_name=entry["credential_name"],
            credential_info={},
            credential_values=dict(entry["credential_values"]),
        )
        for entry in call["credentials"]
    ]
    return get_complete_model_list(
        key_models=call["key_models"],
        team_models=call["team_models"],
        proxy_model_list=call["proxy_model_list"],
        user_model=call["user_model"],
        infer_model_from_keys=call["infer_model_from_keys"],
        return_wildcard_routes=call["return_wildcard_routes"],
        llm_router=build_router(call["deployments"]),
        model_access_groups=call["model_access_groups"],
        include_model_access_groups=call["include_model_access_groups"],
        only_model_access_groups=call["only_model_access_groups"],
        team_id=call["team_id"],
    )


def mcp_server_ids(call: McpServerIdsInput) -> list[str]:
    token = UserAPIKeyAuth(object_permission_id=call["object_permission_id"])
    return asyncio.run(get_mcp_server_ids(token))


def all_fallbacks(call: AllFallbacksInput) -> FallbacksOutput:
    settings = call["settings"]
    router: Router | None = None
    if settings is not None:
        router = Router(
            model_list=[],
            fallbacks=settings["fallbacks"],
            context_window_fallbacks=settings["context_window_fallbacks"],
            content_policy_fallbacks=settings["content_policy_fallbacks"],
        )
    warnings = WarningLog()
    logger = logging.getLogger("LiteLLM Proxy")
    logger.addHandler(warnings)
    try:
        fallbacks = get_all_fallbacks(
            model=call["model"], llm_router=router, fallback_type=call["fallback_type"]
        )
    finally:
        logger.removeHandler(warnings)
    return {"fallbacks": fallbacks, "warnings": warnings.messages}


def build_router(deployments: list[DeployedRow]) -> Router | None:
    if not deployments:
        return None
    model_list: list[dict[str, JsonValue]] = []
    for deployment in deployments:
        info: dict[str, JsonValue] = {}
        if deployment["team_id"] is not None:
            info["team_id"] = deployment["team_id"]
        if deployment["team_public_model_name"] is not None:
            info["team_public_model_name"] = deployment["team_public_model_name"]
        model_list.append(
            {
                "model_name": deployment["model_name"],
                "litellm_params": deployment["litellm_params"],
                "model_info": info,
            }
        )
    return Router(model_list=model_list)
