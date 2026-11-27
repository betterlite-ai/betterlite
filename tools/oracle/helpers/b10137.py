from typing import TypedDict

from litellm.proxy import proxy_server
from pydantic import JsonValue, TypeAdapter

_DUMPED: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Input(TypedDict):
    function: str
    param_name: str | None
    name: str
    value: JsonValue
    is_full_admin: bool
    redact_all: bool


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "redact_secret_values":
        return {"result": proxy_server._redact_secret_values_in_obj(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L18384
            input_value["value"]
        )}
    if function == "redact_param_for_logging":
        return {"result": proxy_server._redact_config_param_value_for_logging(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L18402
            input_value["param_name"], input_value["value"]
        )}
    if function == "redact_setting_value":
        return {"result": proxy_server._redact_general_setting_value(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L18410
            input_value["name"], input_value["value"], input_value["is_full_admin"]
        )}
    if function == "dump_redacted_config":
        dumped = proxy_server._dump_redacted_config(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L18420
            input_value["value"], redact_all_values=input_value["redact_all"]
        )
        if dumped is None:
            return {"result": None}
        return {"result": _DUMPED.validate_json(dumped)}
    raise ValueError(f"unknown b10137 function {function}")
