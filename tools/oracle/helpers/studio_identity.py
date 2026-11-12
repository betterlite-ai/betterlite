import os
from typing import NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.gemini.common_utils import GeminiModelInfo, get_api_key_from_env
from litellm.llms.gemini.google_genai.transformation import GoogleGenAIConfig
from pydantic import TypeAdapter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Headers: TypeAlias = dict[str, str]


class Call(TypedDict):
    model: str
    api_key: NotRequired[str]
    auth_map: NotRequired[Headers]
    params_api_key: NotRequired[str]
    params_gemini_api_key: NotRequired[str]
    generic_key: NotRequired[str]
    api_base: NotRequired[str]
    env: NotRequired[Headers]
    headers: NotRequired[Headers]
    models: NotRequired[list[str]]


class Input(TypedDict):
    calls: list[Call]


INPUT = TypeAdapter(Input)


def environment(call: Call) -> Headers:
    env = call.get("env")
    return {} if env is None else env


def litellm_params(call: Call) -> Headers:
    params: Headers = {}
    api_key = call.get("params_api_key")
    if api_key is not None:
        params["api_key"] = api_key
    gemini_api_key = call.get("params_gemini_api_key")
    if gemini_api_key is not None:
        params["gemini_api_key"] = gemini_api_key
    return params


def explicit_key(call: Call) -> str | None:
    auth_map = call.get("auth_map")
    if auth_map is not None:
        # the annotated str never reaches the dict branch the method takes at runtime
        return cast(str | None, auth_map)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/google_genai/transformation.py#L158
    return call.get("api_key")


def resolve(call: Call) -> Json:
    os.environ.clear()
    os.environ.update(environment(call))
    litellm.api_key = call.get("generic_key")
    info = GeminiModelInfo()
    config = GoogleGenAIConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/google_genai/transformation.py#L53
    headers: Headers = config.validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/google_genai/transformation.py#L146-L176
        api_key=explicit_key(call),
        headers=call.get("headers"),
        model=call["model"],
        litellm_params=litellm_params(call),
    )
    requested = call.get("models")
    if requested is None:
        requested = []
    names = info.process_model_name([{"name": name} for name in requested])
    return {
        "api_key": info.get_api_key(call.get("api_key")),
        "env_key": get_api_key_from_env(),
        "api_base": GeminiModelInfo.get_api_base(call.get("api_base")),
        "headers": Object(headers),
        "base_model": GeminiModelInfo.get_base_model(call["model"]),
        "model_names": list[Json](names),
    }


def run(input_value: Json) -> Json:
    parsed = INPUT.validate_python(input_value)
    return {"calls": [resolve(call) for call in parsed["calls"]]}
