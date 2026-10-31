from typing import Literal, TypedDict

from litellm.models.end_user import LiteLLM_EndUserTable
from litellm.models.user import LiteLLM_UserTable, SCIMPlaceholder
from litellm.models.verification_token import (
    LiteLLM_DeletedVerificationToken,
    LiteLLM_VerificationToken,
)

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict):
    operation: Literal["key", "deleted_key", "user", "placeholder", "end_user"]
    row: dict[str, Json]


def run(input_value: Input) -> Json:
    operation = input_value["operation"]
    row = input_value["row"]
    if operation == "key":
        return LiteLLM_VerificationToken.model_validate(row).model_dump(mode="json")
    if operation == "deleted_key":
        return LiteLLM_DeletedVerificationToken.model_validate(row).model_dump(mode="json")
    if operation == "user":
        user = LiteLLM_UserTable.model_validate(row)
        included_dump = user.model_dump(mode="json", include={"password"})
        if "password" in included_dump:
            raise ValueError("password must stay excluded even when explicitly included")
        return {**user.model_dump(mode="json"), "password_present": user.password is not None}
    if operation == "placeholder":
        return SCIMPlaceholder.model_validate(row).model_dump(mode="json")
    if operation == "end_user":
        return LiteLLM_EndUserTable.model_validate(row).model_dump(mode="json")
    raise ValueError("unsupported identity row operation")
