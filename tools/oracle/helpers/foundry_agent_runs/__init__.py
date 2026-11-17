import asyncio

import litellm.llms.azure.common_utils
from pydantic import JsonValue

from .answers import config_answer, run_answer
from .fakes import Clock, Factory, Resolver, install_fixtures
from .types import JSON, DispatchCall, Input, JsonObject


def dispatch_answer(call: DispatchCall) -> JsonValue:
    clock = Clock()
    factory = Factory(call["steps"])
    resolver = Resolver(call["token"])
    install_fixtures(clock, factory)
    azure = litellm.llms.azure.common_utils
    azure.get_azure_ad_token = resolver  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L299
    outcome = asyncio.run(config_answer(call, factory, clock))
    return {"resolver": JSON.validate_python(resolver.seen), **outcome}


def run(argument: Input) -> JsonObject:
    answer: JsonObject = {}
    if "runs" in argument:
        answer["runs"] = [run_answer(call) for call in argument["runs"]]
    if "dispatches" in argument:
        answer["dispatches"] = [
            dispatch_answer(call) for call in argument["dispatches"]
        ]
    return answer
