from enum import StrEnum
from typing import Literal, TypeAlias

from pydantic import BaseModel

Protocol: TypeAlias = Literal["otlp_grpc", "otlp_http"]


class LangfuseOtelConfig(BaseModel):
    otlp_auth_headers: str | None = None
    protocol: Protocol = "otlp_http"


class LangfuseSpanAttributes(StrEnum):
    LANGFUSE_ENVIRONMENT = "langfuse.environment"
    VERSION = "langfuse.version"
    RELEASE = "langfuse.release"
    GENERATION_NAME = "langfuse.generation.name"
    GENERATION_ID = "langfuse.generation.id"
    PARENT_OBSERVATION_ID = "langfuse.generation.parent_observation_id"
    MASK_INPUT = "langfuse.generation.mask_input"
    MASK_OUTPUT = "langfuse.generation.mask_output"
    OBSERVATION_INPUT = "langfuse.observation.input"
    OBSERVATION_OUTPUT = "langfuse.observation.output"
    TRACE_USER_ID = "user.id"
    SESSION_ID = "session.id"
    TAGS = "langfuse.trace.tags"
    TRACE_NAME = "langfuse.trace.name"
    TRACE_ID = "langfuse.trace.id"
    TRACE_METADATA = "langfuse.trace.metadata"
    EXISTING_TRACE_ID = "langfuse.trace.existing_id"
    UPDATE_TRACE_KEYS = "langfuse.trace.update_keys"
    DEBUG_LANGFUSE = "langfuse.debug"
