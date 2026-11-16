from typing import Literal, TypedDict

from litellm.proxy.management_endpoints.cache_settings_endpoints import (
    CacheSettingsManager,
    _redact_settings,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/cache_settings_endpoints.py#L262-L275
)


class ParamsEqualInput(TypedDict):
    function: Literal["cache_params_equal"]
    params1: dict[str, object]
    params2: dict[str, object]


class RedactSettingsInput(TypedDict):
    function: Literal["redact_settings"]
    settings: dict[str, object] | None


Input = ParamsEqualInput | RedactSettingsInput


def run(input_value: Input) -> object:
    match input_value["function"]:
        case "cache_params_equal":
            result = CacheSettingsManager._cache_params_equal(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/cache_settings_endpoints.py#L340-L369
                input_value["params1"], input_value["params2"]
            )
        case "redact_settings":
            result = _redact_settings(input_value["settings"])
    return result
