from collections.abc import Callable, Mapping, MutableMapping
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from botocore.credentials import Credentials
from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
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
from pydantic import BaseModel, ConfigDict, JsonValue

Json: TypeAlias = JsonValue
Params: TypeAlias = dict[str, Json]


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid")
    operation: str
    params: Params | None = None


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
    try:
        _ = pop_aws_auth_params(remaining)
    except ValueError:
        return cast(Json, {"remaining": remaining, "valid": False})
    return cast(Json, {"remaining": remaining, "valid": True})


def cache_key(params: Params) -> Json:
    instance = BaseAWSLLM()
    left = instance.get_cache_key(cast(Mapping[str, str | bool | None], params["left"]))
    right = instance.get_cache_key(cast(Mapping[str, str | bool | None], params["right"]))
    return {"length": len(left), "equal": left == right}


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


def auth_error(params: Params) -> Json:
    status = cast(int, params["status"])
    error = AwsAuthError(status, str(params["message"]))
    request = {"method": error.request.method, "url": str(error.request.url)}
    with patch("os.path.exists", return_value=False):
        verification = BaseAWSLLM()._get_ssl_verify(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            cast(bool | str | None, params.get("ssl_verify"))
        )
    return cast(
        Json,
        {
            "error": {
                "status": error.status_code,
                "message": error.message,
                "request": request,
                "response": {"status_code": error.response.status_code, "request": request},
            },
            "ssl_verify": verification,
        },
    )


class FixtureSession:
    def __init__(self, credentials: Credentials) -> None:
        self.credentials = credentials

    def get_credentials(self) -> Credentials:
        return self.credentials


def local(params: Params) -> Json:

    settings = cast(Params, params["settings"])
    fixture = cast(list[str | None], params["fixture"])
    instance = BaseAWSLLM()
    supplied = Credentials(fixture[0], fixture[1], fixture[2])
    with patch("boto3.Session", return_value=FixtureSession(supplied)) as session:
        profile = cast(str | None, settings.get("aws_profile_name"))
        access = cast(str | None, settings.get("aws_access_key_id"))
        secret = cast(str | None, settings.get("aws_secret_access_key"))
        token = cast(str | None, settings.get("aws_session_token"))
        region = cast(str | None, settings.get("aws_region_name"))
        if profile is not None:
            credentials, ttl = instance._auth_with_aws_profile(profile)  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            session.assert_called_once_with(profile_name=profile)
        elif access is not None and secret is not None and token is not None:
            credentials, ttl = instance._auth_with_aws_session_token(access, secret, token)  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
        elif access is not None and secret is not None and region is not None:
            credentials, ttl = instance._auth_with_access_key_and_secret_key(access, secret, region)  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            session.assert_called_once_with(
                aws_access_key_id=access, aws_secret_access_key=secret, region_name=region
            )
        else:
            credentials, ttl = instance._auth_with_env_vars()  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            session.assert_called_once_with()
    return cast(
        Json,
        {
            "credentials": [credentials.access_key, credentials.secret_key, credentials.token],
            "ttl": ttl,
        },
    )


def cache(params: Params) -> Json:

    instance = BaseAWSLLM()
    instance.iam_cache = DualCache(in_memory_cache=InMemoryCache(max_size_in_memory=1000))
    arguments = cast(Mapping[str, str | bool | None], params["parameters"])
    fetches = 0
    results: list[Json] = []
    for step in cast(list[Params], params["steps"]):

        def fetch(step: Params = step) -> tuple[Credentials, int | None]:
            nonlocal fetches
            fetches += 1
            if step.get("fail"):
                raise ValueError("fixture failure")
            access = cast(str | None, step["access"])
            credentials = Credentials(access, "fixture") if access is not None else None
            return cast(Credentials, credentials), cast(int | None, step["ttl"])

        with patch("time.time", return_value=step["now"]):
            try:
                value = cast(
                    Credentials | None,
                    instance._get_or_set_cached_credentials(arguments, fetch),  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
                )
                results.append(
                    {"access": cast(str, value.access_key) if value is not None else None}
                )
            except ValueError:
                results.append({"error": True})
    return {"fetches": fetches, "results": results}


HANDLERS: dict[str, Callable[[Params], Json]] = {
    "static_key_pair": static_pair,
    "canonical_tags": tags,
    "assume_role": assume_role,
    "web_policy": policy,
    "bearer": bearer,
    "pop_auth": pop_auth,
    "cache_key": cache_key,
    "auth_selector": selector,
    "local": local,
    "cache": cache,
    "auth_error": auth_error,
}


def run(argument: Json) -> Json:
    supplied = Input.model_validate(argument)
    handler = HANDLERS.get(supplied.operation)
    if handler is None:
        raise ValueError(f"unknown operation: {supplied.operation}")
    _ = litellm
    return handler({} if supplied.params is None else supplied.params)
