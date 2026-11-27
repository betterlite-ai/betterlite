from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.proxy._types import JWTRoutingOverride, LiteLLM_JWTAuth
from litellm.proxy.auth.handle_jwt import JWTHandler
from litellm.proxy.auth.user_api_key_auth import (
    _matches_routing_override,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L555-L565
    _routing_selector_matches_claim,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L514-L552
    _should_route_jwt_to_oauth2_override,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L568-L582
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from pydantic import JsonValue


class Override(TypedDict):
    iss: str | list[str]
    client_id: NotRequired[str | list[str] | None]
    scope: NotRequired[str | list[str] | None]
    aud: NotRequired[str | list[str] | None]
    path: NotRequired[Literal["oauth2"]]


class SelectorCall(TypedDict):
    operation: Literal["selector"]
    selector: str | list[str] | None
    claim: NotRequired[str | list[str] | None]
    split_space_delimited: bool


class OverrideCall(TypedDict):
    operation: Literal["override"]
    override: Override
    claims: dict[str, JsonValue]


class RouteCall(TypedDict):
    operation: Literal["route"]
    token: str
    routing_overrides: list[Override] | None


Input: TypeAlias = SelectorCall | OverrideCall | RouteCall


def handler_for(overrides: list[Override] | None) -> JWTHandler:
    rules = (
        None
        if overrides is None
        else [JWTRoutingOverride(**override) for override in overrides]
    )
    handler = JWTHandler()
    handler.update_environment(
        prisma_client=None,
        user_api_key_cache=UserApiKeyCache(),
        litellm_jwtauth=LiteLLM_JWTAuth(routing_overrides=rules),
        leeway=0,
    )
    return handler


def run(input_value: Input) -> dict[str, bool]:
    match input_value["operation"]:
        case "selector":
            return {
                "matched": _routing_selector_matches_claim(
                    input_value["selector"],
                    input_value.get("claim"),
                    split_space_delimited=input_value["split_space_delimited"],
                )
            }
        case "override":
            return {
                "matched": _matches_routing_override(
                    token_claims=input_value["claims"],
                    override=JWTRoutingOverride(**input_value["override"]),
                )
            }
        case "route":
            return {
                "matched": _should_route_jwt_to_oauth2_override(
                    token=input_value["token"],
                    jwt_handler=handler_for(input_value["routing_overrides"]),
                )
            }
