from typing import Literal, TypedDict, cast

from litellm.proxy import proxy_server
from litellm.proxy.anthropic_endpoints import gateway_endpoints
from pydantic import JsonValue

JsonObject = dict[str, JsonValue]


class GeneralSettings(TypedDict, total=False):
    enable_claude_code_gateway: bool
    managed_settings: JsonObject


class Input(TypedDict):
    mode: Literal["state", "guard"]
    general_settings: GeneralSettings


def gateway_settings(settings: GeneralSettings) -> dict[str, JsonValue]:
    mapping: dict[str, JsonValue] = {}
    enabled = settings.get("enable_claude_code_gateway")
    if enabled is not None:
        mapping["enable_claude_code_gateway"] = enabled
    managed = settings.get("managed_settings")
    if managed is not None:
        mapping["claude_code_gateway_managed_settings"] = managed
    return mapping


def managed_settings() -> JsonObject | None:
    return cast("JsonObject | None", gateway_endpoints._managed_settings())  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/anthropic_endpoints/gateway_endpoints.py#L118-L122


def run(input_value: Input) -> JsonValue:
    proxy_server.general_settings = gateway_settings(input_value["general_settings"])
    if input_value["mode"] == "guard":
        gateway_endpoints.ensure_gateway_enabled()
        return {"managed_settings": managed_settings()}
    return {
        "enabled": gateway_endpoints._is_gateway_enabled(),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/anthropic_endpoints/gateway_endpoints.py#L107-L108
        "managed_settings": managed_settings(),
    }
