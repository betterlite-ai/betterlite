from typing import Literal, TypeAlias

from pydantic import BaseModel

Protocol: TypeAlias = Literal["otlp_grpc", "otlp_http"]


class ArizeConfig(BaseModel):
    space_id: str | None = None
    space_key: str | None = None
    api_key: str | None = None
    protocol: Protocol
    endpoint: str
    project_name: str | None = None
