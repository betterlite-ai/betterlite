from typing import Final, Literal, TypedDict

from litellm.models.team import Member
from litellm.proxy.management_endpoints.team_endpoints import _members_audit_value  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L3156
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)


class MemberValue(TypedDict):
    user_id: str | None
    user_email: str | None
    role: Literal["admin", "user"]


class Input(TypedDict):
    team_alias: str | None
    members: list[MemberValue]


def run(input_value: Input) -> JsonValue:
    members = [
        Member(user_id=member["user_id"], user_email=member["user_email"], role=member["role"])
        for member in input_value["members"]
    ]
    text = _members_audit_value(input_value["team_alias"], members)  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L3156
    return JSON.validate_json(text)
