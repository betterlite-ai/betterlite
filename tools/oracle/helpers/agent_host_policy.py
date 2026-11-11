import asyncio
import os
from collections.abc import Callable
from typing import TypeAlias, cast
from unittest import mock

import litellm
from litellm.llms.gradient_ai.chat.transformation import GradientAIConfig
from litellm.llms.heroku.chat.transformation import HerokuChatConfig
from litellm.llms.hyperbolic.chat.transformation import HyperbolicChatConfig
from litellm.llms.inception.chat.transformation import InceptionChatConfig
from litellm.llms.inception.completion.transformation import InceptionTextCompletionConfig
from litellm.llms.lambda_ai.chat.transformation import LambdaAIChatConfig
from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

CONFIGS: dict[str, object] = {
    "gradient": GradientAIConfig(),
    "heroku": HerokuChatConfig(),
    "hyperbolic": HyperbolicChatConfig(),
    "lambda_ai": LambdaAIChatConfig(),
    "inception": InceptionChatConfig(),
}
INHERITED: frozenset[str] = frozenset({"heroku", "lambda_ai"})


def obj(value: Json | None) -> Object:
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def info(step: Object) -> Json:
    resolved = cast(
        tuple[str | None, str | None],
        CONFIGS[text(step, "host")]._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]
            optional_text(step, "api_base"), optional_text(step, "api_key")
        ),
    )
    return {"base": resolved[0], "key": resolved[1]}


def supported(step: Object) -> Json:
    host = text(step, "host")
    if host in INHERITED:
        return None
    return cast(
        Json,
        CONFIGS[host].get_supported_openai_params(model=""),  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]
    )


def gradient_url(step: Object) -> Json:
    url = GradientAIConfig().get_complete_url(  # pyright: ignore[reportUnknownMemberType]
        api_base=optional_text(step, "api_base"),
        api_key=None,
        model="",
        optional_params={},
        litellm_params={},
    )
    return cast(Json, url)


def gradient_validate(step: Object) -> Json:
    validated = GradientAIConfig().validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        headers=step.get("headers"),  # pyright: ignore[reportArgumentType]
        model="",
        messages=[],
        optional_params={},
        litellm_params={},
        api_key=optional_text(step, "api_key"),
    )
    return cast(Json, validated)


def gradient_params(step: Object) -> Json:
    mapped = GradientAIConfig().map_openai_params(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        non_default_params=obj(step.get("params")),
        optional_params={},
        model="",
        drop_params=step.get("drop_params") is True,
    )
    return cast(Json, mapped)


def heroku_url(step: Object) -> Json:
    url = HerokuChatConfig().get_complete_url(  # pyright: ignore[reportUnknownMemberType]
        api_base=optional_text(step, "api_base"),
        api_key=optional_text(step, "api_key"),
        model="",
        optional_params={},
        litellm_params={},
    )
    return cast(Json, url)


def heroku_request(step: Object) -> Json:
    messages = step.get("messages")
    if not isinstance(messages, list):
        raise TypeError("messages must be a list")
    typed = cast(list[AllMessageValues], messages)
    optional = obj(step.get("optional"))
    config = HerokuChatConfig()
    match step.get("async_mode"):
        case True:
            body = asyncio.run(  # pyright: ignore[reportUnknownVariableType]
                config.async_transform_request(  # pyright: ignore[reportUnknownArgumentType, reportUnknownMemberType]
                    model=text(step, "model"),
                    messages=typed,
                    optional_params=optional,
                    litellm_params={"custom_llm_provider": "heroku"},
                    headers={},
                )
            )
        case _:
            body = config.transform_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
                model=text(step, "model"),
                messages=typed,
                optional_params=optional,
                litellm_params={"custom_llm_provider": "heroku"},
                headers={},
            )
    return cast(Json, body)


def fim_supported(_step: Object) -> Json:
    return cast(
        Json,
        InceptionTextCompletionConfig().get_supported_openai_params(model=""),  # pyright: ignore[reportUnknownMemberType]
    )


def fim_params(step: Object) -> Json:
    mapped = InceptionTextCompletionConfig().map_openai_params(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        non_default_params=obj(step.get("params")),
        optional_params={},
        model="",
        drop_params=False,
    )
    return cast(Json, mapped)


def status_of(error: Exception) -> int | None:
    carried: object = getattr(error, "status_code", None)
    if isinstance(carried, bool) or not isinstance(carried, int):
        return None
    return carried


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "info": info,
    "supported": supported,
    "gradient_url": gradient_url,
    "gradient_validate": gradient_validate,
    "gradient_params": gradient_params,
    "heroku_url": heroku_url,
    "heroku_request": heroku_request,
    "fim_supported": fim_supported,
    "fim_params": fim_params,
}


def run_call(call: Object) -> Json:
    handler = HANDLERS.get(text(call, "op"))
    if handler is None:
        raise ValueError(f"unknown op {text(call, 'op')}")
    try:
        return {"ok": True, "value": handler(call)}
    except Exception as error:  # noqa: BLE001 - every source exception becomes its status
        return {"ok": False, "error": {"status": status_of(error)}}


def run_scenario(scenario: Object) -> Json:
    variables: dict[str, str] = {}
    for name, value in obj(scenario.get("environment")).items():
        if not isinstance(value, str):
            raise TypeError("environment values must be strings")
        variables[name] = value
    calls = scenario.get("calls")
    if not isinstance(calls, list):
        raise TypeError("calls must be a list")
    with (
        mock.patch.dict(os.environ, variables, clear=True),
        mock.patch.object(litellm, "inception_key", optional_text(scenario, "inception_key")),
    ):
        return {"steps": [run_call(obj(call)) for call in calls]}


def run(value: dict[str, Json]) -> Json:
    scenarios = value.get("scenarios")
    if not isinstance(scenarios, list):
        raise TypeError("scenarios must be a list")
    return [run_scenario(obj(scenario)) for scenario in scenarios]
