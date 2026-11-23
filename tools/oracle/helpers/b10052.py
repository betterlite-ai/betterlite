from typing import Final, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.auth.auth_checks import allowed_route_check_inside_route
from pydantic import TypeAdapter

AUTH: Final[TypeAdapter[UserAPIKeyAuth]] = TypeAdapter(UserAPIKeyAuth)


class Row(TypedDict):
    user_role: str | None
    user_id: str | None
    requested_user_id: str | None


class Input(TypedDict):
    rows: list[Row]


def answer(row: Row) -> bool:
    auth = AUTH.validate_python({"user_role": row["user_role"], "user_id": row["user_id"]})
    return allowed_route_check_inside_route(auth, row["requested_user_id"])


def run(input_value: Input) -> list[bool]:
    return [answer(row) for row in input_value["rows"]]
