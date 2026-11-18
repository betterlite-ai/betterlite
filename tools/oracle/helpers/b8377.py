import os
from typing import Final, Literal, NotRequired, TypedDict

import litellm
from litellm.proxy import proxy_server
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

PARAMS: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
_PICKED: Final[list[str]] = []


class Input(TypedDict):
    operation: Literal["env_refs", "usage_family", "env_has_target", "config_error"]
    params: NotRequired[dict[str, JsonValue]]
    config: NotRequired[dict[str, JsonValue]]
    environment: dict[str, str]


# litellm builds the chosen client eagerly, so the recorders keep the oracle off a Redis socket.
class _PlainRecorder(proxy_server.RedisCache):
    def __init__(self, *args: JsonValue, **kwargs: JsonValue) -> None:
        del args, kwargs
        _PICKED.append("standalone")


class _ClusterRecorder(proxy_server.RedisClusterCache):
    def __init__(self, *args: JsonValue, **kwargs: JsonValue) -> None:
        del args, kwargs
        _PICKED.append("cluster")


proxy_server.RedisCache = _PlainRecorder
proxy_server.RedisClusterCache = _ClusterRecorder


def apply_environment(values: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(values)
    os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"


def resolved_env_refs(params: dict[str, JsonValue]) -> dict[str, JsonValue]:
    resolved = proxy_server._resolve_coordination_redis_env_refs(params)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L4867
    model = proxy_server.CoordinationRedisParams.model_validate(resolved)
    return PARAMS.validate_json(model.model_dump_json(exclude_none=True))


def chosen_family(params: dict[str, JsonValue]) -> str:
    model = proxy_server.CoordinationRedisParams.model_validate(params)
    dumped = PARAMS.validate_json(model.model_dump_json(exclude_none=True))
    _PICKED.clear()
    proxy_server._build_redis_usage_cache(dumped)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L4875
    return _PICKED[-1]


def config_error_kind(config: dict[str, JsonValue]) -> str:
    proxy_config = proxy_server.ProxyConfig()
    try:
        proxy_config._init_coordination_redis(config)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L5862
    except ValueError as error:
        message = str(error)
        if "must be a mapping" in message:
            return "not_mapping"
        if "needs a connection target" in message:
            return "missing_target"
        return "other"
    return "none"


def run(input_value: Input) -> JsonValue:
    apply_environment(input_value["environment"])
    match input_value["operation"]:
        case "env_refs":
            return resolved_env_refs(input_value["params"])
        case "usage_family":
            return chosen_family(input_value["params"])
        case "env_has_target":
            return proxy_server._environment_has_redis_connection_target()  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L4894
        case "config_error":
            return config_error_kind(input_value["config"])
