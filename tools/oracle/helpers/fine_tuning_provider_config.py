import os
from typing import Literal, NotRequired, Protocol, TypedDict, cast

from litellm.proxy.fine_tuning_endpoints import endpoints
from pydantic import JsonValue


class Endpoints(Protocol):
    def set_fine_tuning_config(self, config: JsonValue) -> None: ...

    def get_fine_tuning_provider_config(self, custom_llm_provider: str) -> JsonValue: ...


FINE_TUNING = cast(Endpoints, endpoints)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/fine_tuning_endpoints/endpoints.py#L32-L59


class Step(TypedDict):
    action: Literal["set", "get"]
    config: NotRequired[JsonValue]
    provider: NotRequired[str]


class Input(TypedDict):
    env: dict[str, str]
    steps: list[Step]


def run(input_value: Input) -> JsonValue:
    os.environ.update(input_value["env"])
    results: list[JsonValue] = []
    for step in input_value["steps"]:
        if step["action"] == "set":
            FINE_TUNING.set_fine_tuning_config(step.get("config"))
            results.append(None)
        else:
            provider = step.get("provider")
            if provider is None:
                raise ValueError("a get step carries no provider")
            results.append(FINE_TUNING.get_fine_tuning_provider_config(provider))
    return {"results": results}
