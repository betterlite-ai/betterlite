import asyncio
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy._types import LiteLLM_VerificationToken, UserAPIKeyAuth
from litellm.proxy.management_endpoints import key_management_endpoints
from pydantic import JsonValue, TypeAdapter

Role: TypeAlias = Literal[
    "proxy_admin",
    "proxy_admin_viewer",
    "org_admin",
    "internal_user",
    "internal_user_viewer",
    "team",
    "customer",
]
CALLER: Final[TypeAdapter[UserAPIKeyAuth]] = TypeAdapter(UserAPIKeyAuth)


class CallerInput(TypedDict):
    user_role: Role | None
    api_key: str | None
    user_id: str | None


class KeyRowInput(TypedDict):
    user_id: str | None
    team_id: str | None


class Call(TypedDict):
    caller: CallerInput
    key: str | None
    key_row: KeyRowInput


class Input(TypedDict):
    calls: list[Call]


def caller_record(input_value: CallerInput) -> UserAPIKeyAuth:
    fields: dict[str, JsonValue] = {
        "user_role": input_value["user_role"],
        "api_key": input_value["api_key"],
        "user_id": input_value["user_id"],
    }
    return CALLER.validate_python(fields)


def run(input_value: Input) -> list[bool]:
    answers: list[bool] = []
    for call in input_value["calls"]:
        caller = caller_record(call["caller"])
        key_row = LiteLLM_VerificationToken(
            user_id=call["key_row"]["user_id"],
            team_id=call["key_row"]["team_id"],
        )
        answers.append(
            asyncio.run(
                key_management_endpoints._can_user_query_key_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L7606-L7627
                    user_api_key_dict=caller,
                    key=call["key"],
                    key_info=key_row,
                )
            )
        )
    return answers
