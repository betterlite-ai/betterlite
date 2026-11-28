import json
from typing import Final, TypedDict

from litellm.proxy.management_endpoints.internal_user_endpoints import _user_search_where  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/internal_user_endpoints.py#L2202-L2215
from pydantic import JsonValue, TypeAdapter

PREDICATE: Final = TypeAdapter(dict[str, JsonValue])


class Input(TypedDict):
    search: str | None


def run(input_value: Input) -> dict[str, JsonValue]:
    predicate = _user_search_where(input_value["search"])
    return PREDICATE.validate_json(json.dumps(predicate, default=dict))
