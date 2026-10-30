import time
import uuid
from typing import Any, cast


def uuid_shape() -> dict[str, Any]:
    value = uuid.uuid4().hex
    value = f"{int(time.time() * 1000):012x}" + value[12:]
    value = value[:12] + "7" + value[13:16] + "8" + value[17:]
    return {"version": int(value[12], 16), "variant": int(value[16], 16) >> 2}


def run(input_value: dict[str, Any]) -> dict[str, Any]:
    function = cast(str, input_value["function"])
    if function == "uuid":
        return uuid_shape()
    if function == "config":
        values = input_value
        user = values.get("user")
        environment = cast(dict[str, str], values.get("environment", {}))
        config = cast(dict[str, str], values.get("config", {}))
        key = cast(str, values["key"])
        result = user if user is not None else environment.get(
            f"OPIK_{key.upper()}", config.get(key, values.get("default"))
        )
        return {"value": result}
    if function == "metadata":
        values = input_value
        auth = cast(dict[str, Any], values.get("auth", {}))
        request = cast(dict[str, Any], values.get("request", {}))
        requester = cast(dict[str, Any], values.get("requester", {}))
        result = {**auth, **request, **requester}
        return {"metadata": result}
    if function == "payload":
        values = input_value
        response = cast(dict[str, object], values.get("response", {}))
        usage = {
            key: value
            for key, value in cast(dict[str, object], response.get("usage", {})).items()
            if value is not None
        }
        return {
            "type": "llm",
            "model": response.get("model", "unknown-model"),
            "usage": usage,
        }
    raise ValueError(f"unsupported function {function}")
