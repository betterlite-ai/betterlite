import os
from typing import Final, TypedDict

import litellm
from litellm.llms.vertex_ai.vertex_llm_base import VertexBase

ENVIRONMENT: Final[tuple[str, ...]] = (
    "VERTEXAI_PROJECT",
    "VERTEXAI_LOCATION",
    "VERTEX_LOCATION",
    "VERTEXAI_CREDENTIALS",
)


class Params(TypedDict, total=False):
    vertex_project: str
    vertex_ai_project: str
    vertex_location: str
    vertex_ai_location: str
    vertex_credentials: str
    vertex_ai_credentials: str


class Settings(TypedDict, total=False):
    vertex_project: str
    vertex_location: str


class Scenario(TypedDict, total=False):
    params: Params
    settings: Settings
    environment: dict[str, str]


class Input(TypedDict):
    scenarios: list[Scenario]


class Projection(TypedDict):
    project: str | None
    location: str | None
    credentials: str | None
    explicit_location: str | None


class Output(TypedDict):
    scenarios: list[Projection]


EMPTY_PARAMS: Final[Params] = Params()


def apply_environment(environment: dict[str, str]) -> None:
    for name in ENVIRONMENT:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def apply_settings(settings: Settings | None) -> None:
    litellm.vertex_project = None if settings is None else settings.get("vertex_project")
    litellm.vertex_location = None if settings is None else settings.get("vertex_location")


def projection(params: Params) -> Projection:
    fields = dict(params)
    project = VertexBase.safe_get_vertex_ai_project(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vertex_llm_base.py#L1197-L1205
        fields
    )
    credentials = VertexBase.safe_get_vertex_ai_credentials(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vertex_llm_base.py#L1211-L1216
        fields
    )
    return Projection(
        project=project,
        location=VertexBase.safe_get_vertex_ai_location(fields),
        credentials=credentials,
        explicit_location=VertexBase.explicit_vertex_ai_location(params),
    )


def run(data: Input) -> Output:
    projections: list[Projection] = []
    for fields in data["scenarios"]:
        apply_settings(fields.get("settings"))
        environment = fields.get("environment")
        apply_environment({} if environment is None else environment)
        params = fields.get("params")
        projections.append(projection(EMPTY_PARAMS if params is None else params))
    return Output(scenarios=projections)
