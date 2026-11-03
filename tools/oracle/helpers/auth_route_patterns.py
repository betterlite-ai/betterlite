import logging
from types import SimpleNamespace
from typing import TypeAlias, cast

from litellm._logging import verbose_proxy_logger
from litellm.proxy.auth.auth_utils import (
    get_request_route,
    get_request_route_template,
    normalize_request_route,
)
from starlette.requests import Request

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class ScopeError(RuntimeError):
    pass


class TemplateError(RuntimeError):
    pass


class BrokenRoute:
    @property
    def path(self) -> str:
        raise TemplateError("injected template read failure")


class RequestSource:
    def __init__(self, row: dict[str, Json]) -> None:
        self.row = row
        self.url = SimpleNamespace(path=row["url_path"])

    @property
    def scope(self) -> object:
        if self.row.get("scope") == "error":
            raise ScopeError("injected scope read failure")
        if self.row.get("scope") == "malformed":
            return None
        fields: dict[str, object] = {
            key: self.row[key]
            for key in ("path", "root_path", "app_root_path")
            if key in self.row
        }
        access = self.row.get("template_access")
        if access == "error":
            fields["route"] = BrokenRoute()
        else:
            fields["route"] = SimpleNamespace(
                path=17 if access == "malformed" else self.row.get("template")
            )
        return fields


class Logs(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.levels: list[Json] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.levels.append(record.levelname)


def _request(row: dict[str, Json]) -> Json:
    logs = Logs()
    old_level = verbose_proxy_logger.level
    verbose_proxy_logger.addHandler(logs)
    verbose_proxy_logger.setLevel(logging.DEBUG)
    try:
        request = cast(Request, cast(object, RequestSource(row)))
        route = get_request_route(request)
        template = get_request_route_template(request)
        return {"name": row["name"], "route": route, "template": template, "logs": logs.levels}
    finally:
        verbose_proxy_logger.removeHandler(logs)
        verbose_proxy_logger.setLevel(old_level)


def run(input_value: dict[str, Json]) -> Json:
    if input_value["function"] == "normalize":
        normalize_request_route.cache_clear()
        return [normalize_request_route(route) for route in cast(list[str], input_value["routes"])]
    return [_request(row) for row in cast(list[dict[str, Json]], input_value["requests"])]
