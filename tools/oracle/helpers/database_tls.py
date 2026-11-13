from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.proxy.db.db_url_settings import translate_libpq_ssl_params
from litellm_proxy_extras.utils import ProxyExtrasDBManager

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class RecordingResolver:
    def __init__(self) -> None:
        self.calls: list[list[Json]] = []

    def __call__(self, path: str, host: str, port: int) -> str:
        self.calls.append([path, host, port])
        return path


def text_value(item: Json, name: str) -> str:
    if not isinstance(item, str):
        raise ValueError(f"{name} must be a string")
    return item


def optional_text(item: Json, name: str) -> str | None:
    if item is None:
        return None
    return text_value(item, name)


def object_value(item: Json, name: str) -> dict[str, Json]:
    if not isinstance(item, dict):
        raise ValueError(f"{name} must be an object")
    return item


def list_value(item: Json, name: str) -> list[Json]:
    if not isinstance(item, list):
        raise ValueError(f"{name} must be a list")
    return item


def pair_list(item: Json, name: str) -> list[tuple[str, str]]:
    pairs: list[tuple[str, str]] = []
    for entry in list_value(item, name):
        pair = list_value(entry, name)
        if len(pair) != 2:  # noqa: PLR2004 - a pair is exactly a key and a value
            raise ValueError(f"{name} entries must be [key, value] pairs")
        pairs.append((text_value(pair[0], name), text_value(pair[1], name)))
    return pairs


def run_translate(input_value: dict[str, Json]) -> Json:
    url = text_value(input_value["url"], "url")
    resolver = RecordingResolver()
    translated = translate_libpq_ssl_params(url, resolver)
    return cast(Json, {"url": translated, "resolved": resolver.calls})


def run_strip(input_value: dict[str, Json]) -> Json:
    url = text_value(input_value["url"], "url")
    stripped = ProxyExtrasDBManager._strip_prisma_query_params(url)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return cast(Json, stripped)


def run_libpq_tls_params(input_value: dict[str, Json]) -> Json:
    pairs = pair_list(input_value["pairs"], "pairs")
    sslaccept = optional_text(input_value.get("sslaccept"), "sslaccept")
    result = ProxyExtrasDBManager._libpq_tls_params(tuple(pairs), sslaccept)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return cast(Json, [[key, value] for key, value in result])


def run_batch(input_value: dict[str, Json]) -> Json:
    return [scenario(item) for item in list_value(input_value["calls"], "calls")]


def scenario(value: Json) -> Json:
    try:
        return cast(Json, {"ok": True, "value": run(object_value(value, "call"))})
    except Exception:  # noqa: BLE001 - every failure is the compared outcome of one scenario
        return cast(Json, {"ok": False})


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "batch": run_batch,
    "libpq_tls_params": run_libpq_tls_params,
    "strip_prisma_query_params": run_strip,
    "translate_libpq_ssl_params": run_translate,
}


def run(input_value: dict[str, Json]) -> Json:
    function = input_value["function"]
    handler = HANDLERS.get(cast(str, function))
    if handler is None:
        raise ValueError(f"unknown database_tls function: {function}")
    return handler(input_value)
