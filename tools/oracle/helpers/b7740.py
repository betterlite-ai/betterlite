import os
from typing import TypeAlias

import fastapi.applications
from fastapi.responses import RedirectResponse
from litellm.proxy import utils as proxy_utils

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

DOCS_ENV_KEYS = (
    "DOCS_URL",
    "REDOC_URL",
    "OPENAPI_URL",
    "NO_DOCS",
    "NO_REDOC",
    "NO_OPENAPI",
    "ROOT_REDIRECT_URL",
)
SWAGGER_MARKER = "const ui = SwaggerUIBundle({"
LITELLM_TAGS_EDIT = 'plugins:[LazyLoadPlugin],tagsSorter:"alpha",'
BETTERLITE_TAGS_EDIT = 'tagsSorter:"alpha",'


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def without_lazy_plugin(page: str) -> str:
    marker_start = page.index(SWAGGER_MARKER)
    line_start = page.rindex("\n", 0, marker_start) + 1
    rest = page[marker_start + len(SWAGGER_MARKER) :]
    if not rest.startswith(LITELLM_TAGS_EDIT):
        raise ValueError("LiteLLM's swagger page edit changed")
    rebuilt = SWAGGER_MARKER + BETTERLITE_TAGS_EDIT + rest[len(LITELLM_TAGS_EDIT) :]
    return page[:line_start] + "    " + rebuilt


def swagger_page(title: str, openapi_url: str, scenario: dict[str, Json]) -> str:
    # the proxy_server import builds the whole app; url resolution stays without it
    from litellm.proxy import proxy_server  # noqa: PLC0415

    server_root_path = string_value(scenario.get("server_root_path"), "server_root_path")
    proxy_server.server_root_path = server_root_path  # pyright: ignore[reportAttributeAccessIssue]
    proxy_server.mount_swagger_ui()
    prefix = server_root_path.rstrip("/")
    response = fastapi.applications.get_swagger_ui_html(  # pyright: ignore[reportPrivateImportUsage]
        openapi_url=prefix + openapi_url,
        title=f"{title} - Swagger UI",
        oauth2_redirect_url=prefix + "/docs/oauth2-redirect",
    )
    return without_lazy_plugin(bytes(response.body).decode())


def apply_environment(values: dict[str, Json]) -> None:
    for key in DOCS_ENV_KEYS:
        os.environ.pop(key, None)
    for key, value in values.items():
        if key not in DOCS_ENV_KEYS:
            raise ValueError(f"unknown environment variable: {key}")
        os.environ[key] = string_value(value, key)


def resolution(scenario: dict[str, Json]) -> Json:
    apply_environment(scenario)
    root_redirect_url = os.getenv("ROOT_REDIRECT_URL")
    location = (
        RedirectResponse(url=root_redirect_url).headers["location"]
        if root_redirect_url is not None
        else None
    )
    return {
        "docs_url": proxy_utils._get_docs_url(),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "redoc_url": proxy_utils._get_redoc_url(),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "openapi_url": proxy_utils._get_openapi_url(),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "root_redirect_url": root_redirect_url,
        "root_redirect_location": location,
    }


def run(input_value: dict[str, Json]) -> Json:
    operation = string_value(input_value.get("op"), "op")
    scenarios = input_value.get("scenarios")
    if not isinstance(scenarios, list):
        raise ValueError("scenarios must be a list")
    results: list[Json] = []
    if operation == "swagger_page":
        title = string_value(input_value.get("title"), "title")
        openapi_url = string_value(input_value.get("openapi_url"), "openapi_url")
        results.extend(
            swagger_page(title, openapi_url, object_value(scenario, "scenario"))
            for scenario in scenarios
        )
        return results
    if operation == "docs_url_resolution":
        results.extend(
            resolution(object_value(scenario, "scenario")) for scenario in scenarios
        )
        return results
    raise ValueError(f"unknown op: {operation}")
