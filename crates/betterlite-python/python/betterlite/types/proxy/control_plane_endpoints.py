from pydantic import BaseModel, field_validator


class WorkerRegistryEntry(BaseModel):
    worker_id: str
    name: str
    url: str

    @field_validator("url")
    @classmethod
    def url_must_be_http(cls, value: str) -> str:
        if not value.startswith(("http://", "https://")):
            raise ValueError("Worker URL must start with http:// or https://")
        return value
