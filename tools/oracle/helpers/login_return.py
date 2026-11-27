import sys
from types import ModuleType
from typing import Final, Literal, TypedDict

from litellm.proxy.management_endpoints.ui_sso import (
    SSOAuthenticationHandler,
    _is_same_origin_return_path,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L2784-L2862
)
from pydantic import JsonValue

SERVER_MODULE: Final = "litellm.proxy.proxy_server"


class Input(TypedDict):
    function: Literal["same_origin_path", "validate"]
    return_to: str
    control_plane_url: str | None


class ProxyServer(ModuleType):
    general_settings: dict[str, str | None]


def proxy_module(control_plane_url: str | None) -> ModuleType:
    module = ProxyServer(SERVER_MODULE)
    module.general_settings = {"control_plane_url": control_plane_url}
    return module


def validate_answer(input_value: Input) -> JsonValue:
    saved = sys.modules.get(SERVER_MODULE)
    sys.modules[SERVER_MODULE] = proxy_module(input_value["control_plane_url"])
    try:
        valid = SSOAuthenticationHandler._validate_return_to(input_value["return_to"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L2896-L2924
    finally:
        if saved is None:
            del sys.modules[SERVER_MODULE]
        else:
            sys.modules[SERVER_MODULE] = saved
    return {"valid": valid}


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "same_origin_path":
        return {"safe": _is_same_origin_return_path(input_value["return_to"])}
    return validate_answer(input_value)
