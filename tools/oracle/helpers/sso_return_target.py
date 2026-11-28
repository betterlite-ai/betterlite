from typing import TypedDict

from litellm.proxy import proxy_server
from litellm.proxy.management_endpoints import ui_sso
from pydantic import JsonValue
from starlette.exceptions import HTTPException


class ValidateCall(TypedDict):
    return_to: str
    control_plane_url: str | None


class Validation(TypedDict):
    valid: bool | None
    status: int | None


class Input(TypedDict):
    same_origin: list[str]
    validate: list[ValidateCall]


class Output(TypedDict):
    same_origin: list[bool]
    validate: list[Validation]


def validate_call(call: ValidateCall) -> Validation:
    settings: dict[str, JsonValue] = {}
    if call["control_plane_url"] is not None:
        settings["control_plane_url"] = call["control_plane_url"]
    proxy_server.general_settings = settings
    try:
        checked = ui_sso.SSOAuthenticationHandler._validate_return_to(call["return_to"])  # noqa: SLF001  # pyright: ignore[reportAny, reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L2896-L2924
    except HTTPException as error:
        return {"valid": None, "status": error.status_code}
    finally:
        proxy_server.general_settings = {}
    return {"valid": checked, "status": None}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L2896-L2924


def run(input_value: Input) -> Output:
    same_origin = [
        ui_sso._is_same_origin_return_path(target)  # noqa: SLF001  # pyright: ignore[reportAny, reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L2784-L2797
        for target in input_value["same_origin"]
    ]
    return {
        "same_origin": same_origin,
        "validate": [validate_call(call) for call in input_value["validate"]],
    }
