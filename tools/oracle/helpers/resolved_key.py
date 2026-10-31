from typing import Any, Literal, TypedDict

from litellm.proxy._types import (
    LiteLLM_VerificationTokenView,
    LitellmUserRoles,
    UserAPIKeyAuth,
    hash_token,
    user_api_key_has_admin_view,
)
from litellm.proxy.auth.handle_jwt import JWTHandler

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict):
    function: Literal[
        "role", "view", "auth", "hash_token", "is_jwt",
        "health_principal", "jobs_principal", "service_account", "admin_view",
    ]
    value: Json


def object_value(value: Json, name: str) -> dict[str, Any]:  # pyright: ignore[reportExplicitAny]
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def run(input_value: Input) -> Json:
    function = input_value["function"]
    value = input_value["value"]
    result: Json
    match function:
        case "role":
            role = LitellmUserRoles(string_value(value, "value"))
            result = {
                "value": str(role),
                "description": role.description,
                "ui_label": role.ui_label,
                "is_internal_user_role": role.is_internal_user_role,
            }
        case "view":
            view = LiteLLM_VerificationTokenView(**object_value(value, "value"))  # pyright: ignore[reportAny]
            result = view.model_dump(mode="json")
        case "auth":
            auth = UserAPIKeyAuth(**object_value(value, "value"))  # pyright: ignore[reportAny]
            result = auth.model_dump(mode="json")
        case "hash_token":
            result = {"hash": hash_token(string_value(value, "value"))}
        case "is_jwt":
            token = None if value is None else string_value(value, "value")
            result = {"is_jwt": JWTHandler.is_jwt(token=token)}
        case "health_principal":
            health = UserAPIKeyAuth.get_litellm_internal_health_check_user_api_key_auth()
            result = health.model_dump(mode="json")
        case "jobs_principal":
            jobs = UserAPIKeyAuth.get_litellm_internal_jobs_user_api_key_auth()
            result = jobs.model_dump(mode="json")
        case "service_account":
            service_auth = UserAPIKeyAuth(**object_value(value, "value"))  # pyright: ignore[reportAny]
            result = {"result": service_auth.is_team_service_account}
        case "admin_view":
            view_auth = UserAPIKeyAuth(**object_value(value, "value"))  # pyright: ignore[reportAny]
            result = {"result": user_api_key_has_admin_view(view_auth)}
    return result
