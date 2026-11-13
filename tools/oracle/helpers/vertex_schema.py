from copy import deepcopy
from typing import Literal, TypeAlias, TypedDict, cast

from litellm.llms.vertex_ai.common_utils import (  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/common_utils.py#L686-L760
    _build_json_schema,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/common_utils.py#L737-L760
    _build_vertex_schema,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/common_utils.py#L686-L735
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict):
    mode: Literal["build_vertex_schema", "build_json_schema"]
    schema: dict[str, Json]
    add_property_ordering: bool
    settings: dict[str, str]


def run(input_value: Input) -> Json:
    parameters = deepcopy(input_value["schema"])
    if input_value["mode"] == "build_json_schema":
        return cast(Json, _build_json_schema(parameters))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/common_utils.py#L737-L760
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/common_utils.py#L686-L735
        Json,
        _build_vertex_schema(
            parameters, add_property_ordering=input_value["add_property_ordering"]
        ),
    )
