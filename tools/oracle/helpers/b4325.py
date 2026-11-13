from typing import Protocol, TypeAlias, cast

from litellm.caching.redis_cache import (
    RedisCache,
    _cluster_node_pubsub_client,  # pyright: ignore[reportPrivateUsage]
)
from redis.asyncio import SSLConnection
from redis.asyncio.cluster import ClusterNode, NodesManager, RedisCluster

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def integer_value(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def optional_string(value: Json, name: str) -> str | None:
    if value is None:
        return None
    return string_value(value, name)


def optional_number(value: Json, name: str) -> float | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def optional_bool(value: Json, name: str) -> bool:
    if value is None:
        return False
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


class _Pool(Protocol):
    connection_class: type[object]
    connection_kwargs: dict[str, object]


class _PoolClient(Protocol):
    connection_pool: _Pool


class _EmptyNodes:
    startup_nodes: dict[str, object]

    def __init__(self) -> None:
        self.startup_nodes = {}


class _EmptyCluster(RedisCluster):
    def __init__(self) -> None:  # pyright: ignore[reportMissingSuperCall]
        self.nodes_manager = cast(NodesManager, _EmptyNodes())

    def get_default_node(self) -> ClusterNode:
        return cast(ClusterNode, None)


def _startup_nodes(input_value: dict[str, Json]) -> list[ClusterNode]:
    raw = input_value.get("startup_nodes")
    if raw is None:
        return []
    if not isinstance(raw, list):
        raise ValueError("startup_nodes must be a list")
    nodes: list[ClusterNode] = []
    for entry in raw:
        node = object_value(entry, "startup node")
        nodes.append(
            ClusterNode(
                string_value(node.get("host"), "host"),
                integer_value(node.get("port"), "port"),
            )
        )
    return nodes


def _cluster_pubsub_node(input_value: dict[str, Json]) -> Json:
    nodes = _startup_nodes(input_value)
    client: _PoolClient
    if not nodes:
        # RedisCluster rejects an empty node list, so this path gets a stub with no default node.
        client = cast(_PoolClient, _cluster_node_pubsub_client(_EmptyCluster()))
    else:
        cluster = RedisCluster(
            startup_nodes=nodes,
            username=optional_string(input_value.get("username"), "username"),
            password=optional_string(input_value.get("password"), "password"),
            ssl=optional_bool(input_value.get("ssl"), "ssl"),
            socket_timeout=optional_number(input_value.get("socket_timeout"), "socket_timeout"),
        )
        client = cast(_PoolClient, _cluster_node_pubsub_client(cluster))
    pool = client.connection_pool
    kwargs = pool.connection_kwargs
    return {
        "host": cast(Json, kwargs.get("host")),
        "port": cast(Json, kwargs.get("port")),
        "username": cast(Json, kwargs.get("username")),
        "has_password": kwargs.get("password") is not None,
        "ssl": pool.connection_class is SSLConnection,
        "socket_timeout": cast(Json, kwargs.get("socket_timeout")),
    }


def _redis_kwargs(settings: dict[str, Json]) -> dict[str, object]:
    return {name: value for name, value in settings.items() if value is not None}


def _cache_key(settings: dict[str, Json]) -> str:
    cache = object.__new__(RedisCache)
    cache.redis_kwargs = _redis_kwargs(settings)
    return cache._get_async_client_cache_key()  # pyright: ignore[reportPrivateUsage]


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    match function:
        case "cluster_pubsub_node":
            return _cluster_pubsub_node(input_value)
        case "client_key_same":
            left = object_value(input_value.get("left"), "left")
            right = object_value(input_value.get("right"), "right")
            return {"same": _cache_key(left) == _cache_key(right)}
        case _:
            raise ValueError(f"unknown function {function}")
