from typing import Literal, TypeAlias, TypedDict, cast

from litellm.caching.dual_cache import DualCache
from litellm.proxy._types import LiteLLM_ObjectPermissionTable, UserAPIKeyAuth
from litellm.proxy.auth.auth_checks import (
    _copy_user_api_key_auth_for_cache,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3970-L3978
)
from litellm.proxy.common_utils.cache_pydantic_utils import CacheCodec
from litellm.proxy.common_utils.user_api_key_cache import (
    END_USER_RESTRICTED_REGISTRY_OVERFLOW_SENTINEL as END_USER_RESTRICTED_OVERFLOW,
)
from litellm.proxy.common_utils.user_api_key_cache import (
    MODEL_ACCESS_GROUP_REGISTRY_OVERFLOW_SENTINEL as GROUP_REGISTRY_OVERFLOW,
)
from litellm.proxy.common_utils.user_api_key_cache import (
    NO_TEAM_MEMBERSHIP_SENTINEL,
    UserApiKeyCache,
    end_user_cache_key,
    end_user_restricted_registry_cache_key,
    get_management_object_ttl,
    is_user_key_cache_key,
    model_access_group_cache_key,
    model_access_group_registry_cache_key,
    model_access_group_spend_counter_key,
    object_permission_cache_key,
    project_cache_key,
    project_spend_counter_key,
    tag_cache_key,
    tag_registry_cache_key,
    team_membership_auth_cache_key,
    team_membership_reservation_cache_key,
    user_object_permission_id_cache_key,
)
from litellm.proxy.common_utils.user_api_key_cache import (
    TAG_REGISTRY_OVERFLOW_SENTINEL as TAG_REGISTRY_OVERFLOW,
)
from litellm.proxy.common_utils.user_api_key_cache import (
    USER_NO_MCP_PERMISSION_SENTINEL as USER_NO_MCP_PERMISSION,
)
from pydantic import ValidationError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

class KeyIds(TypedDict):
    user_id: str
    object_permission_id: str
    tag: str
    access_group: str
    project_id: str
    end_user_id: str
    team_id: str
    member_user_id: str


class Input(TypedDict):
    scenario: Literal["codec", "management_ttl", "partition", "keys", "copy"]
    payloads: list[Json]
    ttls: list[float | None]
    fallback: float
    keys: list[str]
    ids: KeyIds
    auth: dict[str, Json]


def codec_projection(payload: Json) -> Json:
    untyped = cast(Json, CacheCodec.serialize(payload))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/cache_pydantic_utils.py#L40
    typed: Json = None
    error: str | None = None
    try:
        typed = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/cache_pydantic_utils.py#L40
            Json, CacheCodec.serialize(payload, model_type=LiteLLM_ObjectPermissionTable)
        )
    except ValidationError:
        error = "validation"
    decoded = CacheCodec.deserialize(payload, model_type=LiteLLM_ObjectPermissionTable)
    decoded_payload: Json = None
    if decoded is not None:
        decoded_payload = cast(Json, decoded.model_dump(mode="json"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/object_permission.py#L11-L26
    return {"typed": typed, "error": error, "untyped": untyped, "decoded": decoded_payload}


def codec_report(payloads: list[Json]) -> Json:
    projections: list[Json] = [codec_projection(payload) for payload in payloads]
    return {"projections": projections}


def management_ttl_report(ttls: list[float | None]) -> Json:
    reports: list[Json] = []
    for ttl in ttls:
        cache = DualCache()
        if ttl is not None:
            cache.update_cache_ttl(default_in_memory_ttl=ttl, default_redis_ttl=None)
        reports.append(float(get_management_object_ttl(cache)))
    return {"ttls": reports}


def partition_report(keys: list[str]) -> Json:
    cache = UserApiKeyCache()
    selections: list[Json] = [
        {
            "key": key,
            "key_partition": is_user_key_cache_key(key),
            "store": cache.in_memory_cache_for(key) is cache.key_object_cache.in_memory_cache,
        }
        for key in keys
    ]
    return {"selections": selections}


def keys_report(ids: KeyIds) -> Json:
    return {
        "user_object_permission_id": user_object_permission_id_cache_key(ids["user_id"]),
        "object_permission_id": object_permission_cache_key(ids["object_permission_id"]),
        "tag": tag_cache_key(ids["tag"]),
        "tag_registry": tag_registry_cache_key(),
        "tag_registry_overflow": TAG_REGISTRY_OVERFLOW,
        "model_access_group": model_access_group_cache_key(ids["access_group"]),
        "model_access_group_registry": model_access_group_registry_cache_key(),
        "model_access_group_registry_overflow": GROUP_REGISTRY_OVERFLOW,
        "model_access_group_spend": model_access_group_spend_counter_key(ids["access_group"]),
        "project_id": project_cache_key(ids["project_id"]),
        "spend_project": project_spend_counter_key(ids["project_id"]),
        "end_user_id": end_user_cache_key(ids["end_user_id"]),
        "end_user_restricted_registry": end_user_restricted_registry_cache_key(),
        "end_user_restricted_registry_overflow": END_USER_RESTRICTED_OVERFLOW,
        "team_membership_auth": team_membership_auth_cache_key(
            ids["team_id"], ids["member_user_id"]
        ),
        "team_membership_reservation": team_membership_reservation_cache_key(
            ids["member_user_id"], ids["team_id"]
        ),
        "user_no_mcp_permission": USER_NO_MCP_PERMISSION,
        "no_team_membership": NO_TEAM_MEMBERSHIP_SENTINEL,
    }


def copy_report(fields: dict[str, Json]) -> Json:
    original = UserAPIKeyAuth.model_validate(fields)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356
    copied = _copy_user_api_key_auth_for_cache(original)
    dump_equal: Json = copied.model_dump() == original.model_dump()  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356
    return {"dump_equal": dump_equal}


def run(input_value: Input) -> Json:
    scenario = input_value["scenario"]
    if scenario == "codec":
        return codec_report(input_value["payloads"])
    if scenario == "management_ttl":
        return management_ttl_report(input_value["ttls"])
    if scenario == "partition":
        return partition_report(input_value["keys"])
    if scenario == "keys":
        return keys_report(input_value["ids"])
    return copy_report(input_value["auth"])
