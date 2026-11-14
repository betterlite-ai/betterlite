from pydantic import BaseModel

from betterlite.types.integrations.arize import Protocol


class ArizePhoenixConfig(BaseModel):
    otlp_auth_headers: str | None = None
    protocol: Protocol
    endpoint: str
    project_name: str | None = None
