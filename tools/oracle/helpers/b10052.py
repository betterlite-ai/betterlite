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


def run(input_value: Input) -> list[bool]:
    answers: list[bool] = []
    for row in input_value["rows"]:
        fields: dict[str, str | None] = {
            "user_role": row["user_role"],
            "user_id": row["user_id"],
        }
        auth = AUTH.validate_python(fields)
        answers.append(allowed_route_check_inside_route(auth, row["requested_user_id"]))
    return answers
