from typing import TypeAlias, cast

import httpx
from litellm.llms.valkey import common_utils
from litellm.llms.valkey.vector_stores import transformation
from litellm.types.vector_stores import VectorStoreSearchOptionalRequestParams
from redis.commands.search.result import Result

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _integer(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def _number(value: Json, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def _boolean(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def _optional_number(value: Json, name: str) -> float | None:
    return None if value is None else _number(value, name)


def _optional_string(value: Json, name: str) -> str | None:
    return None if value is None else _string(value, name)


def _url(call: dict[str, Json]) -> Json:
    return common_utils.build_valkey_url(
        host=_string(call.get("host"), "host"),
        port=str(_integer(call.get("port"), "port")),
        password=_optional_string(call.get("password"), "password"),
        ssl=_boolean(call.get("ssl", False), "ssl"),
    )


def _pack(call: dict[str, Json]) -> Json:
    values = call.get("vector")
    if not isinstance(values, list):
        raise ValueError("vector must be a list")
    return list(common_utils.pack_vector([_number(item, "vector item") for item in values]))


def _knn(call: dict[str, Json]) -> Json:
    litellm_params = _object(call.get("litellm_params", {}), "litellm_params")
    params = transformation._ValkeySearchParams.model_validate(litellm_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    optional_params = cast(
        VectorStoreSearchOptionalRequestParams,
        _object(call.get("optional_params", {}), "optional_params"),
    )
    query = transformation.ValkeyVectorStoreConfig._knn_query(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        optional_params,
        embedding_field=params.embedding_field,
        text_field=params.text_field,
    )
    return cast(Json, query.get_args())


def _results(call: dict[str, Json]) -> Json:
    litellm_params = _object(call.get("litellm_params", {}), "litellm_params")
    params = transformation._ValkeySearchParams.model_validate(litellm_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    documents = call.get("documents")
    if not isinstance(documents, list):
        raise ValueError("documents must be a list")
    raw: list[Json] = [len(documents)]
    for document in documents:
        entry = _object(document, "document")
        pairs: list[Json] = []
        for name, value in _object(entry.get("fields", {}), "fields").items():
            pairs.append(name)
            pairs.append(value)
        raw.append(_string(entry.get("id"), "id"))
        raw.append(pairs)
    result = Result(raw, hascontent=True)
    response = transformation.ValkeyVectorStoreConfig._to_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        result,
        _string(call.get("query_text"), "query_text"),
        params.text_field,
    )
    return cast(Json, response)


def _timeout(value: Json) -> list[float]:
    call = _object(value, "timeout")
    kind = _string(call.get("kind"), "kind")
    timeout: float | httpx.Timeout | None
    if kind == "none":
        timeout = None
    elif kind == "seconds":
        timeout = _number(call.get("seconds"), "seconds")
    else:
        timeout = httpx.Timeout(
            None,
            connect=_optional_number(call.get("connect"), "connect"),
            read=_optional_number(call.get("read"), "read"),
        )
    connect, read = transformation.ValkeyVectorStoreConfig._socket_timeouts(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        timeout
    )
    return [connect, read]


def _timeouts(call: dict[str, Json]) -> Json:
    values = call.get("timeouts")
    if not isinstance(values, list):
        raise ValueError("timeouts must be a list")
    return cast(Json, [_timeout(value) for value in values])


def _run_call(value: Json) -> Json:
    call = _object(value, "call")
    kind = _string(call.get("call"), "call")
    payload = _object(call.get("input"), "input")
    try:
        if kind == "url":
            return {"ok": True, "value": _url(payload)}
        if kind == "pack":
            return {"ok": True, "value": _pack(payload)}
        if kind == "knn":
            return {"ok": True, "value": _knn(payload)}
        if kind == "results":
            return {"ok": True, "value": _results(payload)}
        if kind == "timeouts":
            return {"ok": True, "value": _timeouts(payload)}
    except Exception:  # noqa: BLE001 - one call's failure is the outcome the case compares
        return {"ok": False}
    raise ValueError(f"unknown call {kind}")


def run(input_value: dict[str, Json]) -> Json:
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    return [_run_call(value) for value in calls]
