from typing import Final, Literal, NotRequired, TypedDict

from fastapi import FastAPI, HTTPException
from litellm.proxy._types import LiteLLMRoutes, ProxyException, UserAPIKeyAuth
from litellm.proxy.auth import auth_exception_handler
from litellm.proxy.auth.route_checks import RouteChecks
from litellm.proxy.pass_through_endpoints import pass_through_endpoints
from pydantic import JsonValue
from starlette.requests import Request
from starlette.types import Scope

init_helpers = pass_through_endpoints.InitPassThroughEndpointHelpers

PASSTHROUGH_TARGET: Final = "https://passthrough.invalid"


class KeyFields(TypedDict):
    allowed_routes: NotRequired[list[JsonValue] | None]
    metadata: NotRequired[dict[str, JsonValue] | None]
    team_metadata: NotRequired[dict[str, JsonValue] | None]


class Registration(TypedDict):
    endpoint_id: str
    route_type: Literal["exact", "subpath"]
    path: str
    methods: NotRequired[list[str] | None]
    auth: NotRequired[bool | None]


class Input(TypedDict):
    function: Literal["should_call_route", "passthrough_access", "auth_enforced", "jwt_grant"]
    route: str
    method: NotRequired[str | None]
    key: NotRequired[KeyFields | None]
    registrations: NotRequired[list[Registration]]
    team_allowed_routes: NotRequired[list[str]]


class Denial(TypedDict):
    status: int
    error_type: str
    param: str | None
    code: str


class Answer(TypedDict):
    allowed: bool
    error: Denial | None


def run(input_value: Input) -> Answer:
    register(input_value.get("registrations"))
    key = caller_key(input_value.get("key"))
    method = input_value.get("method")
    called_method = None if method is None else method.upper()
    function = input_value["function"]
    if function == "should_call_route":
        return call_route(input_value["route"], called_method, key)
    if function == "passthrough_access":
        return passthrough_access(input_value["route"], key)
    if function == "auth_enforced":
        return auth_enforced(input_value["route"], called_method)
    return jwt_grant(input_value["route"], input_value.get("team_allowed_routes"))


def call_route(route: str, method: str | None, key: UserAPIKeyAuth) -> Answer:
    try:
        RouteChecks.should_call_route(route=route, valid_token=key, request=request_for(method))
    except HTTPException as raised:
        return refusal(raised)
    return {"allowed": True, "error": None}


def passthrough_access(route: str, key: UserAPIKeyAuth) -> Answer:
    allowed = RouteChecks.check_passthrough_route_access(route=route, user_api_key_dict=key)
    return {"allowed": allowed, "error": None}


def auth_enforced(route: str, method: str | None) -> Answer:
    protected = RouteChecks.is_auth_enforced_pass_through_route(route=route, method=method)
    return {"allowed": protected, "error": None}


def jwt_grant(route: str, settings: list[str] | None) -> Answer:
    team_routes: list[str] = []
    if settings is not None:
        team_routes = settings
    granted = RouteChecks.jwt_team_routes_grant_pass_through(
        route=route, team_allowed_routes=team_routes
    )
    return {"allowed": granted, "error": None}


def refusal(raised: HTTPException) -> Answer:
    exception = auth_exception_handler._as_proxy_exception(raised)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_exception_handler.py#L47-L80
    return {"allowed": False, "error": denial_of(exception)}


def denial_of(exception: ProxyException) -> Denial:
    return {
        "status": int(exception.code),
        "error_type": exception.type,
        "param": exception.param,
        "code": exception.code,
    }


def request_for(method: str | None) -> Request | None:
    if method is None:
        return None
    scope: Scope = {"type": "http", "method": method, "path": "/"}
    return Request(scope)


def caller_key(fields: KeyFields | None) -> UserAPIKeyAuth:
    if fields is None:
        return UserAPIKeyAuth.model_construct()
    values: dict[str, JsonValue] = {}
    routes = fields.get("allowed_routes")
    if routes is not None:
        values["allowed_routes"] = routes
    metadata = fields.get("metadata")
    if metadata is not None:
        values["metadata"] = metadata
    team_metadata = fields.get("team_metadata")
    if team_metadata is not None:
        values["team_metadata"] = team_metadata
    return UserAPIKeyAuth.model_construct(**values)


def register(registrations: list[Registration] | None) -> None:
    if registrations is None:
        return
    app = FastAPI()
    protected_paths: list[str] = []
    for registration in registrations:
        protected = registration.get("auth") is True
        if protected:
            protected_paths.append(registration["path"])
            if registration["route_type"] == "subpath":
                protected_paths.append(registration["path"].rstrip("/") + "/*")
        add_route(app, registration, protected=protected)
    extend_openai_routes(protected_paths)


def add_route(app: FastAPI, registration: Registration, *, protected: bool) -> None:
    methods = registration.get("methods")
    if registration["route_type"] == "exact":
        init_helpers.add_exact_path_route(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/pass_through_endpoints.py#L2957-L2973
            app=app,
            path=registration["path"],
            target=PASSTHROUGH_TARGET,
            custom_headers=None,
            forward_headers=None,
            merge_query_params=None,
            dependencies=None,
            cost_per_request=None,
            endpoint_id=registration["endpoint_id"],
            methods=methods,
            auth=protected,
        )
        return
    init_helpers.add_subpath_route(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/pass_through_endpoints.py#L3040-L3056
        app=app,
        path=registration["path"],
        target=PASSTHROUGH_TARGET,
        custom_headers=None,
        forward_headers=None,
        merge_query_params=None,
        dependencies=None,
        cost_per_request=None,
        endpoint_id=registration["endpoint_id"],
        methods=methods,
        auth=protected,
    )


def extend_openai_routes(paths: list[str]) -> None:
    routes = list(LiteLLMRoutes.openai_routes.value)
    for path in paths:
        if path not in routes:
            routes.append(path)
    LiteLLMRoutes.openai_routes.value[:] = routes
