from typing import TypedDict

from litellm.repositories import config_repository
from litellm.repositories.config_repository import ConfigParam
from pydantic import JsonValue, TypeAdapter


class Input(TypedDict):
    raw: str


def run(input_value: Input) -> JsonValue:
    decoded = config_repository._decoded_json(input_value["raw"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/repositories/config_repository.py#L15-L17
    param = ConfigParam("param_name", decoded)
    return TypeAdapter(JsonValue).validate_python(param.param_value)
