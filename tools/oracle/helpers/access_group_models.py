from typing import NotRequired, TypedDict

import litellm
from litellm.proxy.management_helpers import access_group_model_sync
from litellm.router import Router
from pydantic import JsonValue


class Input(TypedDict):
    function: str
    model_list: list[dict[str, JsonValue]]
    deployment_id: NotRequired[str]
    model_name: NotRequired[str]
    model_id: str


def required(value: str | None, function: str, name: str) -> str:
    if value is None:
        raise ValueError(f"the {function} function needs a {name}")
    return value


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    router = Router(model_list=input_value["model_list"])
    function = input_value["function"]
    if function == "config_sourced_sibling":
        deployment_id = required(input_value.get("deployment_id"), function, "deployment_id")
        found = access_group_model_sync._config_sourced_sibling(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/access_group_model_sync.py#L62
            router, deployment_id, input_value["model_id"]
        )
        return {"result": found}
    if function == "served_by_a_config_deployment":
        model_name = required(input_value.get("model_name"), function, "model_name")
        found = access_group_model_sync._served_by_a_config_deployment(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/access_group_model_sync.py#L69
            router, model_name, input_value["model_id"]
        )
        return {"result": found}
    raise ValueError(f"unsupported access_group_models function {function}")
