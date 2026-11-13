from typing import TypedDict

from litellm.models.user import LiteLLM_UserTable
from litellm.repositories.user_repository import UserRepository
from pydantic import JsonValue, TypeAdapter

USER_DUMP = TypeAdapter(dict[str, JsonValue])


class Input(TypedDict):
    records: dict[str, dict[str, JsonValue] | None]


class Repository(UserRepository):
    def convert(self, record: dict[str, JsonValue] | None) -> LiteLLM_UserTable | None:
        return self._to_model(record)


def run(input_value: Input) -> JsonValue:
    output: dict[str, JsonValue] = {}
    for name, record in input_value["records"].items():
        try:
            user = Repository(None).convert(record)
        except ValueError:
            output[name] = None
            continue
        if user is None:
            output[name] = None
            continue
        dumped = USER_DUMP.validate_json(user.model_dump_json())
        dumped["password_present"] = user.password is not None
        output[name] = dumped
    return output
