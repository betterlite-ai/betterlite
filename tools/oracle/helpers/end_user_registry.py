from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy.auth.auth_checks import (  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L1691-L1698
    _column_is_set,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L1691-L1693
    _restricted_end_user_where,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L1696-L1698
)
from pydantic import JsonValue, TypeAdapter


class ColumnIsSetInput(TypedDict):
    kind: Literal["column_is_set"]
    column: str


class RestrictedEndUserWhereInput(TypedDict):
    kind: Literal["restricted_end_user_where"]


Input: TypeAlias = ColumnIsSetInput | RestrictedEndUserWhereInput

COLUMN: Final[TypeAdapter[ColumnIsSetInput]] = TypeAdapter(ColumnIsSetInput)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


def run(input_value: Input) -> JsonValue:
    if input_value["kind"] == "column_is_set":
        call = COLUMN.validate_python(input_value)
        return JSON.validate_python(_column_is_set(call["column"]))
    return JSON.validate_python(_restricted_end_user_where())
