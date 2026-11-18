from litellm.constants import (
    BEDROCK_REALTIME_COMMITTED_FAILURE_SCOPE_KEY as FAILURE_KEY,
)
from litellm.constants import (
    BEDROCK_REALTIME_PENDING_SESSION_UPDATE_SCOPE_KEY as PENDING_KEY,
)
from litellm.constants import (
    BEDROCK_REALTIME_SESSION_COMMITTED_SCOPE_KEY as COMMITTED_KEY,
)
from pydantic import JsonValue

from .models import JsonObject, Scope


class ProviderError(Exception): ...


class AccessDeniedException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ConflictException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class InternalServerException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ModelErrorException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ModelNotReadyException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ModelStreamErrorException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ModelTimeoutException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ResourceNotFoundException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ServiceQuotaExceededException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ServiceUnavailableException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ThrottlingException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class ValidationException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


class UnknownThrottlingException(ProviderError): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L45-L60


PROVIDER_ERRORS = (
    AccessDeniedException, ConflictException, InternalServerException, ModelErrorException,
    ModelNotReadyException, ModelStreamErrorException, ModelTimeoutException,
    ResourceNotFoundException, ServiceQuotaExceededException, ServiceUnavailableException,
    ThrottlingException, ValidationException, UnknownThrottlingException,
)


class FakeClient:
    def __init__(
        self,
        scope: dict[str, JsonValue],
        frames: list[str | None],
        send_error: str | None,
    ) -> None:
        self.scope = scope
        self.frames = frames
        self.send_error = send_error
        self.receive_calls = 0
        self.sent: list[str] = []

    async def receive_text(self) -> str:
        self.receive_calls += 1
        frame = self.frames.pop(0) if self.frames else None
        if frame is None:
            raise RuntimeError("the client sent no more frames")
        return frame

    async def send_text(self, data: str) -> None:
        self.sent.append(data)
        if self.send_error is not None:
            raise RuntimeError(self.send_error)

    async def close(self, _code: int = 1000, _reason: str | None = None) -> None: ...


class FakeStream:
    def __init__(self, error: str | None) -> None:
        self.error = error
        self.awaited = 0

    async def await_output(self) -> tuple[None, None]:
        self.awaited += 1
        if self.error is not None:
            raise RuntimeError(self.error)
        return (None, None)

    @property
    def input_stream(self) -> None:
        return None


def scope_of(scope: Scope) -> dict[str, JsonValue]:
    return {
        PENDING_KEY: scope["pending_session_update"],
        COMMITTED_KEY: scope["session_committed"],
        FAILURE_KEY: scope["committed_failure"],
    }


def record_of(scope: dict[str, JsonValue]) -> JsonObject:
    return {
        "pending_session_update": scope.get(PENDING_KEY),
        "session_committed": scope.get(COMMITTED_KEY),
        "committed_failure": scope.get(FAILURE_KEY),
    }


def provider_error(class_name: str, message: str) -> ProviderError:
    error_class = next(error for error in PROVIDER_ERRORS if error.__name__ == class_name)
    return error_class(message)
