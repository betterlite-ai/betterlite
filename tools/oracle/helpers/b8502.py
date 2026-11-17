from typing import Literal, TypedDict

from litellm.proxy.management_endpoints.cache_settings_endpoints import (
    CacheSettingsManager,
    _redact_settings,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/cache_settings_endpoints.py#L262-L275
)
from pydantic import JsonValue, TypeAdapter


class ParamsEqualInput(TypedDict):
    function: Literal["cache_params_equal"]
    params1: dict[str, JsonValue]
    params2: dict[str, JsonValue]


class RedactSettingsInput(TypedDict):
    function: Literal["redact_settings"]
    settings: dict[str, JsonValue] | None


Input = ParamsEqualInput | RedactSettingsInput

REDACTED = TypeAdapter(dict[str, str])


def run(input_value: Input) -> bool | dict[str, str]:
    if input_value["function"] == "cache_params_equal":
        return CacheSettingsManager._cache_params_equal(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/cache_settings_endpoints.py#L340-L369
            {**input_value["params1"]}, {**input_value["params2"]}
        )
    return REDACTED.validate_python(_redact_settings(input_value["settings"]))
