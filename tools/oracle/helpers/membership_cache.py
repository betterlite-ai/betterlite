from typing import Final, Literal, TypeAlias, TypedDict

from litellm.models.team_membership import LiteLLM_TeamMembership
from litellm.proxy.auth.auth_checks import (
    _TeamMembershipCacheMiss,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L402-L406
    _membership_from_cached_payload,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2328-L2336
)
from pydantic import JsonValue, TypeAdapter


class PendingPayload(TypedDict):
    kind: Literal["pending"]


class TypedPayload(TypedDict):
    kind: Literal["typed"]
    membership: dict[str, JsonValue]


class StoredPayload(TypedDict):
    kind: Literal["stored"]
    payload: JsonValue


CachedPayload: TypeAlias = PendingPayload | TypedPayload | StoredPayload


class Input(TypedDict):
    payloads: list[CachedPayload]


DUMPED: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


def classify(payload: CachedPayload) -> JsonValue:
    if payload["kind"] == "pending":
        result = _membership_from_cached_payload(None)
    elif payload["kind"] == "typed":
        typed = LiteLLM_TeamMembership.model_validate(payload["membership"])
        result = _membership_from_cached_payload(typed)
    else:
        result = _membership_from_cached_payload(payload["payload"])
    match result:
        case None:
            return {"kind": "absent"}
        case _TeamMembershipCacheMiss():
            return {"kind": "miss"}
        case LiteLLM_TeamMembership() as membership:
            dumped = DUMPED.validate_json(membership.model_dump_json())
            return {"kind": "membership", "membership": dumped}


def run(input_value: Input) -> JsonValue:
    results: list[JsonValue] = [classify(payload) for payload in input_value["payloads"]]
    return {"results": results}
