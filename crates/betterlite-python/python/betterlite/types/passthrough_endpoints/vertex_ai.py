from typing import ClassVar

from pydantic import ConfigDict

from betterlite.types.utils.base import ResponseObject

VERTEX_CREDENTIALS_TYPES = str | dict[str, str]


class VertexPassThroughCredentials(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    vertex_project: str | None = None
    vertex_location: str | None = None
    vertex_credentials: VERTEX_CREDENTIALS_TYPES | None = None
