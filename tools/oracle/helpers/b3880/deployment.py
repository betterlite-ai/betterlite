from typing import TypeAlias

from litellm.proxy._types import UserAPIKeyAuth
from pydantic import JsonValue

from .logger import LOGGER
from .values import JSON_ADAPTER, OBJECT_ADAPTER, AuthCase, Input

ParamsMetadata: TypeAlias = dict[str, JsonValue | UserAPIKeyAuth]
RequestObject: TypeAlias = dict[str, JsonValue | ParamsMetadata | dict[str, ParamsMetadata]]


def build_auth(case: AuthCase) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        api_key=case.get("api_key"),
        user_email=case.get("user_email"),
        key_alias=case.get("key_alias"),
        team_id=case.get("team_id"),
        team_alias=case.get("team_alias"),
    )


def deployment_request(input_value: Input) -> RequestObject:
    standard: dict[str, JsonValue] = {}
    payload = input_value["payload_metadata"]
    if payload is not None:
        standard["metadata"] = OBJECT_ADAPTER.validate_python(payload)
    metadata: ParamsMetadata = {}
    params = input_value["litellm_params_metadata"]
    if params is not None:
        metadata.update(OBJECT_ADAPTER.validate_python(params))
    auth = input_value["user_api_key_auth"]
    if auth is not None:
        metadata["user_api_key_auth"] = build_auth(auth)
    request: RequestObject = {
        "standard_logging_object": standard,
        "litellm_params": {"metadata": metadata},
    }
    return request


def deployment_answer(input_value: Input) -> JsonValue:
    labels = LOGGER._extract_deployment_failure_label_values(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L2703
        deployment_request(input_value)
    )
    return JSON_ADAPTER.validate_python(labels)
