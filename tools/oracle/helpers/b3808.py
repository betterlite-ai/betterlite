import io
import tempfile
from contextlib import redirect_stdout
from pathlib import Path
from typing import Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.constants import (
    DEFAULT_SEMANTIC_GUARD_SIMILARITY_THRESHOLD as DEFAULT_THRESHOLD,
)
from litellm.proxy.guardrails.guardrail_hooks.semantic_guard.route_loader import (
    SemanticGuardRouteLoader,
)
from pydantic import JsonValue, TypeAdapter
from semantic_router.route import Route

litellm.suppress_debug_info = True


class RouteView(TypedDict):
    name: str
    description: str
    utterances: list[str]
    score_threshold: float


class BuildRoutes(TypedDict):
    function: Literal["build_routes"]
    route_templates: NotRequired[list[str]]
    custom_routes_file: NotRequired[str | None]
    missing_routes_file: NotRequired[bool]
    custom_routes: NotRequired[list[dict[str, JsonValue]]]
    global_threshold: NotRequired[float]


class ListTemplates(TypedDict):
    function: Literal["list_templates"]


Input: TypeAlias = BuildRoutes | ListTemplates

ROUTE: TypeAdapter[RouteView] = TypeAdapter(RouteView)


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "list_templates":
        return {"templates": template_names()}
    return build_routes(input_value)


def template_names() -> list[JsonValue]:
    names: list[JsonValue] = []
    names.extend(SemanticGuardRouteLoader.list_builtin_templates())
    return names


def build_routes(case: BuildRoutes) -> JsonValue:
    # build_routes logs at INFO to sys.stdout, which carries the oracle's JSON line
    with tempfile.TemporaryDirectory() as directory, redirect_stdout(io.StringIO()):
        routes = SemanticGuardRouteLoader.build_routes(
            route_templates=case.get("route_templates"),
            custom_routes_file=routes_path(case, Path(directory)),
            custom_routes=case.get("custom_routes"),
            global_threshold=threshold(case),
        )
        return {"routes": [route_value(route) for route in routes]}


def threshold(case: BuildRoutes) -> float:
    value = case.get("global_threshold")
    return DEFAULT_THRESHOLD if value is None else value


def routes_path(case: BuildRoutes, root: Path) -> str | None:
    if case.get("missing_routes_file"):
        return str(root / "absent-routes.yaml")
    content = case.get("custom_routes_file")
    if content is None:
        return None
    path = root / "custom-routes.yaml"
    _ = path.write_text(content, encoding="utf-8")
    return str(path)


def route_value(route: Route) -> JsonValue:
    view = ROUTE.validate_json(route.model_dump_json())
    utterances: list[JsonValue] = []
    utterances.extend(view["utterances"])
    return {
        "name": view["name"],
        "description": view["description"],
        "utterances": utterances,
        "score_threshold": view["score_threshold"],
    }
