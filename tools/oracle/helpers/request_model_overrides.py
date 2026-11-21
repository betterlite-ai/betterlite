import io
import logging
from contextlib import redirect_stdout
from typing import Final, TypedDict

import litellm
from litellm.proxy import proxy_server
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.litellm_pre_call_utils import (
    _apply_credential_overrides_from_model_config,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L2696
    _enforced_params_check,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L2942
    _get_enforced_params,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L2908
    _update_model_if_key_alias_exists,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L2670
    _update_model_if_team_alias_exists,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L2592
)
from litellm.router import Router
from litellm.types.utils import CredentialItem
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

MODEL: Final[TypeAdapter[str | None]] = TypeAdapter(str | None)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class SettingsInput(TypedDict):
    credential_overrides: bool
    enforced_params: list[str] | None
    service_account_params: list[str] | None
    team_stale_alias_bypass: bool


class DeploymentInput(TypedDict):
    model_name: str
    model: str
    custom_llm_provider: str | None
    team_id: str | None
    team_public_model_name: str | None


class CredentialInput(TypedDict):
    name: str
    values: dict[str, JsonValue]


class PrincipalInput(TypedDict):
    team_id: str | None
    team_model_aliases: dict[str, str]
    aliases: dict[str, str]
    metadata: dict[str, JsonValue] | None
    team_metadata: dict[str, JsonValue] | None
    project_metadata: dict[str, JsonValue] | None


class CallInput(TypedDict):
    data: dict[str, JsonValue]
    principal: PrincipalInput
    premium_user: bool


class OverridesInput(TypedDict):
    settings: SettingsInput
    deployments: list[DeploymentInput] | None
    credentials: list[CredentialInput]
    calls: list[CallInput]


class DiagnosticCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.records: list[logging.LogRecord] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.records.append(record)


def build_router(deployments: list[DeploymentInput] | None) -> Router | None:
    if deployments is None:
        return None
    model_list: list[dict[str, JsonValue]] = []
    for deployment in deployments:
        params: dict[str, JsonValue] = {"model": deployment["model"]}
        if deployment["custom_llm_provider"] is not None:
            params["custom_llm_provider"] = deployment["custom_llm_provider"]
        info: dict[str, JsonValue] = {}
        if deployment["team_id"] is not None:
            info["team_id"] = deployment["team_id"]
        if deployment["team_public_model_name"] is not None:
            info["team_public_model_name"] = deployment["team_public_model_name"]
        model_list.append(
            {
                "model_name": deployment["model_name"],
                "litellm_params": params,
                "model_info": info,
            }
        )
    return Router(model_list=model_list)


def credential_items(credentials: list[CredentialInput]) -> list[CredentialItem]:
    items: list[CredentialItem] = []
    for credential in credentials:
        row: dict[str, JsonValue] = {
            "credential_name": credential["name"],
            "credential_info": {},
            "credential_values": dict(credential["values"]),
        }
        items.append(CredentialItem.model_validate(row))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/credentials.py#L18
    return items


def user_api_key_auth(principal: PrincipalInput) -> UserAPIKeyAuth:
    auth = UserAPIKeyAuth()
    auth.team_id = principal["team_id"]
    auth.team_model_aliases = principal["team_model_aliases"]
    auth.aliases = principal["aliases"]
    if principal["metadata"] is not None:
        auth.metadata = principal["metadata"]
    auth.team_metadata = principal["team_metadata"]
    auth.project_metadata = principal["project_metadata"]
    return auth


def general_settings(settings: SettingsInput) -> dict[str, JsonValue]:
    general: dict[str, JsonValue] = {}
    if settings["enforced_params"] is not None:
        general["enforced_params"] = list(settings["enforced_params"])
    if settings["service_account_params"] is not None:
        service: dict[str, JsonValue] = {
            "enforced_params": list(settings["service_account_params"]),
        }
        general["service_account_settings"] = service
    return general


def diagnostic_json(record: logging.LogRecord) -> dict[str, JsonValue] | None:
    message = record.getMessage()
    parts = message.split("'")
    level = record.levelname.lower()
    if message.startswith("Team model alias for model="):
        return {
            "kind": "stale_alias_missing_deployment",
            "level": level,
            "model": parts[1],
            "team_id": parts[3],
            "aliased_target": parts[5],
        }
    if message.startswith("Stale team model alias detected for model="):
        return {
            "kind": "stale_alias_sibling_deployment",
            "level": level,
            "model": parts[1],
            "team_id": parts[3],
        }
    if message.startswith("model_config entry "):
        return {
            "kind": "model_entry_without_credentials",
            "level": level,
            "model": parts[1],
        }
    if message.startswith("model_config references credential "):
        return {
            "kind": "credential_not_found",
            "level": level,
            "credential": parts[1],
        }
    if message.startswith("Applied credential override "):
        return {
            "kind": "credential_applied",
            "level": level,
            "credential": parts[1],
            "model": parts[3],
        }
    return None


def call_value(
    call: CallInput,
    router: Router | None,
    settings: SettingsInput,
    capture: DiagnosticCapture,
) -> JsonValue:
    capture.records.clear()
    data = dict(call["data"])
    auth = user_api_key_auth(call["principal"])
    pre_alias_model = MODEL.validate_python(data.get("model"))
    _update_model_if_team_alias_exists(data=data, user_api_key_dict=auth)
    _update_model_if_key_alias_exists(data=data, user_api_key_dict=auth)
    _apply_credential_overrides_from_model_config(
        data=data,
        user_api_key_dict=auth,
        pre_alias_model_name=pre_alias_model,
        llm_router=router,
    )
    diagnostics: list[JsonValue] = []
    for record in capture.records:
        diagnostic = diagnostic_json(record)
        if diagnostic is not None:
            diagnostics.append(diagnostic)
    resolved: list[str] | None = _get_enforced_params(  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/litellm_pre_call_utils.py#L2908
        general_settings=general_settings(settings),
        user_api_key_dict=auth,
    )
    try:
        _enforced_params_check(
            request_body=data,
            general_settings=general_settings(settings),
            user_api_key_dict=auth,
            premium_user=call["premium_user"],
        )
    except (ValueError, TypeError):
        failure: JsonValue = {
            "error": {"status": None, "type": None, "param": None, "code": None}
        }
        return failure
    result: JsonValue = {
        "data": data,
        "enforced_params": JSON.validate_python(resolved),
        "diagnostics": diagnostics,
    }
    return result


def run(input_value: OverridesInput) -> list[JsonValue]:
    settings = input_value["settings"]
    router = build_router(input_value["deployments"])
    litellm.credential_list = credential_items(input_value["credentials"])
    litellm.enable_model_config_credential_overrides = settings["credential_overrides"]
    proxy_server.llm_router = router
    capture = DiagnosticCapture()
    logger = logging.getLogger("LiteLLM Proxy")
    previous_level = logger.level
    logger.setLevel(logging.DEBUG)
    logger.addHandler(capture)
    results: list[JsonValue] = []
    try:
        with redirect_stdout(io.StringIO()):
            results.extend(
                call_value(call, router, settings, capture) for call in input_value["calls"]
            )
    finally:
        logger.removeHandler(capture)
        logger.setLevel(previous_level)
    return results
