from collections.abc import Callable, Mapping, MutableMapping
from typing import TypeAlias, cast

import litellm
from litellm.llms.bedrock.base_aws_llm import (
    AwsAuthError,
    BaseAWSLLM,
    _assume_role_params,  # pyright: ignore[reportPrivateUsage]
    _canonical_aws_session_tags,  # pyright: ignore[reportPrivateUsage]
    bedrock_bearer_token,
    build_web_identity_session_policy,
    pop_aws_auth_params,
)  # pyright: ignore[reportPrivateUsage]
from litellm.llms.bedrock.common_utils import s3_static_key_pair
from pydantic import BaseModel, ConfigDict

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid")
    operation: str
    params: Params


def static_pair(params: Params) -> Json:
    return cast(Json, {"value": s3_static_key_pair(params)})


def tags(params: Params) -> Json:
    result = _canonical_aws_session_tags(params.get("tags"))  # pyright: ignore[reportPrivateUsage]
    return cast(Json, {"value": result})


def assume_role(params: Params) -> Json:
    session_tags = _canonical_aws_session_tags(params.get("tags"))  # pyright: ignore[reportPrivateUsage]
    result = _assume_role_params(  # pyright: ignore[reportPrivateUsage]
        str(params.get("role_arn", "")),
        str(params.get("session_name", "")),
        cast(str | None, params.get("external_id")),
        session_tags,
    )
    return cast(Json, {"value": result})


def policy(_params: Params) -> Json:
    return cast(Json, {"value": build_web_identity_session_policy()})


def bearer(params: Params) -> Json:
    result = bedrock_bearer_token(cast(str | None, params.get("api_key")))
    return cast(Json, {"value": result})


def pop_auth(params: Params) -> Json:
    remaining: MutableMapping[str, object] = cast(MutableMapping[str, object], dict(params))
    removed = pop_aws_auth_params(remaining)
    return cast(Json, {"remaining": remaining, "removed": removed.model_dump()})


def cache_key(params: Params) -> Json:
    key = BaseAWSLLM().get_cache_key(cast(Mapping[str, str | bool | None], params))
    return cast(Json, {"value": key})


def selector(params: Params) -> Json:
    instance = BaseAWSLLM()
    result = {
        "web_identity": instance._is_auth_with_web_identity_token(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            cast(str | None, params.get("aws_web_identity_token")),
            cast(str | None, params.get("aws_role_name")),
            cast(str | None, params.get("aws_session_name")),
        ),
        "role": instance._is_auth_with_aws_role(cast(str | None, params.get("aws_role_name"))),  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
        "profile": instance._is_auth_with_aws_profile(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            cast(str | None, params.get("aws_profile_name")),
        ),
        "session_token": instance._is_auth_with_aws_session_token_tuple(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            cast(str | None, params.get("aws_access_key_id")),
            cast(str | None, params.get("aws_secret_access_key")),
            cast(str | None, params.get("aws_session_token")),
        ),
        "static": instance._is_auth_with_access_key_and_secret_key(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            cast(str | None, params.get("aws_access_key_id")),
            cast(str | None, params.get("aws_secret_access_key")),
            cast(str | None, params.get("aws_region_name")),
        ),
    }
    return cast(Json, result)


def session_auth(params: Params) -> Json:
    credentials, _ttl = BaseAWSLLM()._auth_with_aws_session_token(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
        str(params.get("aws_access_key_id", "")),
        str(params.get("aws_secret_access_key", "")),
        str(params.get("aws_session_token", "")),
    )
    return cast(Json, {
        "access_key_id": credentials.access_key,
        "secret_access_key": credentials.secret_key,
        "token": credentials.token,
    })


def auth_error(params: Params) -> Json:
    status_value = params.get("status", 500)
    message = params.get("message", "")
    status = status_value if isinstance(status_value, int) else 500
    error = AwsAuthError(status, str(message))
    return cast(Json, {
        "status": error.status_code,
        "message": error.message,
        "request": {"method": error.request.method, "url": str(error.request.url)},
        "response": {"status": error.response.status_code},
    })


HANDLERS: dict[str, Callable[[Params], Json]] = {
    "static_key_pair": static_pair,
    "canonical_tags": tags,
    "assume_role": assume_role,
    "web_policy": policy,
    "bearer": bearer,
    "pop_auth": pop_auth,
    "cache_key": cache_key,
    "auth_selector": selector,
    "session_auth": session_auth,
    "auth_error": auth_error,
}


def run(argument: Json) -> Json:
    supplied = Input.model_validate(argument)
    handler = HANDLERS.get(supplied.operation)
    if handler is None:
        raise ValueError(f"unknown operation: {supplied.operation}")
    _ = litellm
    return handler(supplied.params)
