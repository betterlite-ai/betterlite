from typing import cast

from litellm.exceptions import BudgetExceededError
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.auth import user_api_key_auth
from litellm.proxy.common_utils import http_parsing_utils
from pydantic import JsonValue
from starlette.requests import Request
from starlette.types import Scope

from .doubles import ScriptedKeyLimiter, ScriptedTeamLimiter, ScriptedUserLimiter
from .wire import OBJECT, Call


def auth_of(call: Call) -> UserAPIKeyAuth:
    token = call.get("token")
    if token is None:
        raise ValueError("the call names the token fields")
    return UserAPIKeyAuth.model_validate(token)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356


def denial_record(denied: BudgetExceededError) -> JsonValue:
    return {
        "exception": type(denied).__name__,
        "status": denied.status_code,
        "current_cost": denied.current_cost,
        "max_budget": denied.max_budget,
        "entity_type": denied.entity_type,
        "entity_id": denied.entity_id,
    }


def parsed_model_of(request: Request) -> JsonValue:
    parsed = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L303-L309
        "dict[str, JsonValue] | None",
        http_parsing_utils._safe_get_request_parsed_body(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L303
            request=request
        ),
    )
    return None if parsed is None else parsed.get("model")


def json_model_of(request: Request) -> JsonValue:
    try:
        cached = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L484
            "dict[str, JsonValue]",
            request._json,  # type: ignore[attr-defined]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L484
        )
    except AttributeError:
        return None
    return cached.get("model")


def body_model_of(request: Request) -> JsonValue:
    try:
        body = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L485
            "bytes",
            request._body,  # type: ignore[attr-defined]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L485
        )
    except AttributeError:
        return None
    return OBJECT.validate_json(body).get("model")


async def key_fallback_call(call: Call) -> JsonValue:
    fixed = auth_of(call)
    model_name = call.get("model_name")
    request_data = call.get("request_data")
    if model_name is None or request_data is None:
        raise ValueError("a key fallback call names the model and the request data")
    limiter = ScriptedKeyLimiter(failing=call.get("failing"), fallback=call.get("fallback"))
    scope: Scope = {"type": "http", "headers": [], "query_string": b""}
    path_params = call.get("path_params")
    if path_params is not None:
        scope["path_params"] = path_params
    request = Request(scope)
    error: JsonValue = None
    try:
        await user_api_key_auth._check_key_model_budget_with_fallback(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L417-L488
            valid_token=fixed,
            model_max_budget_limiter=limiter,
            model_name=model_name,
            request_data=request_data,
            request=request,
        )
    except BudgetExceededError as denied:
        error = denial_record(denied)
    return {
        "calls": list(limiter.calls),
        "request_model": request_data.get("model"),
        "parsed_model": parsed_model_of(request),
        "json_model": json_model_of(request),
        "body_model": body_model_of(request),
        "path_model": None if path_params is None else path_params.get("model"),
        "error": error,
    }


async def user_budget_call(call: Call) -> JsonValue:
    fixed = auth_of(call)
    models = call.get("models")
    if models is None:
        raise ValueError("a user budget call names the models")
    failing_models = call.get("failing_models")
    limiter = ScriptedUserLimiter([] if failing_models is None else failing_models)
    error: JsonValue = None
    try:
        await user_api_key_auth._check_user_model_budget(fixed, limiter, models)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L377-L395
    except BudgetExceededError as denied:
        error = denial_record(denied)
    return {"calls": list(limiter.calls), "error": error}


async def team_budget_call(call: Call) -> JsonValue:
    fixed = auth_of(call)
    models = call.get("models")
    if models is None:
        raise ValueError("a team budget call names the models")
    failing_models = call.get("failing_models")
    limiter = ScriptedTeamLimiter([] if failing_models is None else failing_models)
    error: JsonValue = None
    try:
        await user_api_key_auth._check_team_model_budget(fixed, limiter, models)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L398-L414
    except BudgetExceededError as denied:
        error = denial_record(denied)
    return {"calls": list(limiter.calls), "error": error}
