from typing import TypedDict

import litellm
from litellm.types.router import LiteLLM_Params
from pydantic import JsonValue, TypeAdapter

from .boundary import CHAT
from .environment import install_environment


class RouteCase(TypedDict):
    model: str
    api_base: str | None
    api_key: str | None
    environment_base: str | None
    environment_key: str | None
    environment_use_litellm_proxy: bool | None
    global_use_litellm_proxy: bool
    deployment_use_litellm_proxy: bool


def route_value(case: RouteCase) -> JsonValue:
    install_environment(
        base=case["environment_base"],
        key=case["environment_key"],
        forced=case["environment_use_litellm_proxy"],
        openai_key=None,
    )
    litellm.use_litellm_proxy = case["global_use_litellm_proxy"]
    params = LiteLLM_Params(
        model=case["model"],
        use_litellm_proxy=case["deployment_use_litellm_proxy"],
    )
    should_use = CHAT.should_use_litellm_proxy_by_default(params)
    model, provider, api_key, api_base = CHAT.litellm_proxy_get_custom_llm_provider_info(
        case["model"], case["api_base"], case["api_key"]
    )
    return {
        "should_use": should_use,
        "model": model,
        "provider": provider,
        "api_base": api_base,
        "api_key": api_key,
        "get_api_key": CHAT.get_api_key(case["api_key"]),
    }


def route_cases(params: JsonValue) -> JsonValue:
    cases = TypeAdapter(list[RouteCase]).validate_python(params)
    return {"results": [route_value(case) for case in cases]}
