from typing import Literal

from pydantic import BaseModel, JsonValue

S3PartitionGranularity = Literal["day", "hour"]


class s3BatchLoggingElement(BaseModel):  # noqa: N801  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/integrations/s3_v2.py#L8
    payload: dict[str, JsonValue]
    s3_object_key: str
    s3_object_download_filename: str
    body: str | None = None
    content_type: str = "application/json"
    retrying_since: float | None = None
