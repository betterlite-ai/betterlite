import asyncio
from types import ModuleType
from typing import Final, Self

from litellm.llms.bedrock.realtime.handler import ClosableBedrockRuntimeClient
from pydantic import JsonValue
from typing_extensions import override

from .models import JsonAdapter, Run


class StreamOpenError(Exception): ...


# LiteLLM maps a provider exception by its class name, so the fake carries the SDK's name
class ModelStreamErrorException(Exception): ...  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L63-L67


class ClientStreamError(Exception): ...


class ClientGoneError(Exception): ...


class ClientSendError(Exception): ...


class CredentialLookupError(Exception): ...


class Recording:
    def __init__(self) -> None:
        self.endpoint: str | None = None
        self.region: str | None = None
        self.access_key_id: str | None = None
        self.secret_access_key: str | None = None
        self.session_token: str | None = None
        self.model_ids: list[str] = []
        self.resolver_static = False
        self.transport_injected = False


class Script:
    def __init__(self, run: Run) -> None:
        self.chunks = list(run["chunks"])
        self.block = run["block"]
        self.receive_failure = run["receive_failure"]
        self.send_failure = run["send_failure"]
        self.open_error = run["open_error"]
        self.receives = 0
        self.sent: list[JsonValue] = []
        self.input_closed = False


class FakePart:
    def __init__(self, bytes_: bytes | None) -> None:
        self.bytes_ = bytes_


class FakeChunk:
    def __init__(self, value: FakePart) -> None:
        self.value = value


class FakeOperation:
    def __init__(self, model_id: str) -> None:
        self.model_id = model_id


class FakeInput:
    async def send(self, event: FakeChunk) -> None:
        script = SCRIPT[-1]
        index = len(script.sent)
        payload = event.value.bytes_
        if payload is not None:
            script.sent.append(JsonAdapter.validate_json(payload))
        if script.send_failure == index:
            raise ClientStreamError("the input send failed")

    async def close(self) -> None:
        script = SCRIPT[-1]
        script.input_closed = True


class FakeOutput:
    async def receive(self) -> FakeChunk | None:
        script = SCRIPT[-1]
        index = script.receives
        script.receives += 1
        if script.receive_failure == index:
            raise ModelStreamErrorException("ModelStreamErrorException")
        if not script.chunks:
            if script.block == "stream":
                await asyncio.Future[None]()
            return None
        return FakeChunk(FakePart(script.chunks.pop(0).encode("utf-8")))


class FakeStream:
    @property
    def input_stream(self) -> FakeInput:
        return FakeInput()

    async def await_output(self) -> tuple[None, FakeOutput]:
        output = FakeOutput()
        return (None, output)


class FakeRuntimeConfig:
    @classmethod
    async def resolve(cls, endpoint_uri: str, region: str, **rest: JsonValue) -> Self:
        RECORDING.endpoint = endpoint_uri
        RECORDING.region = region
        RECORDING.resolver_static = "aws_credentials_identity_resolver" in rest
        RECORDING.transport_injected = "transport" in rest
        return cls()


class FakeRuntimeClient(ClosableBedrockRuntimeClient):
    def __init__(self, config: FakeRuntimeConfig) -> None:
        self.config = config
        self.closed = False
        CLIENTS.append(self)

    async def invoke_model_with_bidirectional_stream(  # ast-grep-ignore: no-long-names
        self, operation: FakeOperation
    ) -> FakeStream:
        RECORDING.model_ids.append(operation.model_id)
        if SCRIPT[-1].open_error:
            raise StreamOpenError("the invoke failed")
        return FakeStream()

    @override
    async def close(self) -> None:
        self.closed = True


class FakeIdentity:
    def __init__(
        self, access_key_id: str, secret_access_key: str, session_token: str | None
    ) -> None:
        RECORDING.access_key_id = access_key_id
        RECORDING.secret_access_key = secret_access_key
        RECORDING.session_token = session_token


class FakeResolver:
    def __init__(self, identity: FakeIdentity) -> None:
        self.identity = identity


class FakeTransport:
    pass


class FakeSdkRuntime(ModuleType):
    def __init__(self) -> None:
        super().__init__("aws_sdk_bedrock_runtime")


class FakeSdkClient(ModuleType):
    def __init__(self) -> None:
        super().__init__("aws_sdk_bedrock_runtime.client")
        self.AsyncBedrockRuntimeClient = FakeRuntimeClient


class FakeSdkConfig(ModuleType):
    def __init__(self) -> None:
        super().__init__("aws_sdk_bedrock_runtime.config")
        self.AsyncBedrockRuntimeConfig = FakeRuntimeConfig


class FakeSdkModels(ModuleType):
    def __init__(self) -> None:
        super().__init__("aws_sdk_bedrock_runtime.models")
        self.BidirectionalInputPayloadPart = FakePart
        self.InvokeModelWithBidirectionalStreamInputChunk = FakeChunk
        self.InvokeModelWithBidirectionalStreamOperationInput = FakeOperation


class FakeSmithyCore(ModuleType):
    def __init__(self) -> None:
        super().__init__("smithy_aws_core")


class FakeSmithyCoreIdentity(ModuleType):
    def __init__(self) -> None:
        super().__init__("smithy_aws_core.identity")
        self.AWSCredentialsIdentity = FakeIdentity
        self.StaticCredentialsResolver = FakeResolver


class FakeSmithyHttp(ModuleType):
    def __init__(self) -> None:
        super().__init__("smithy_http")


class FakeSmithyHttpCrt(ModuleType):
    def __init__(self) -> None:
        super().__init__("smithy_http.aio.crt")
        self.AWSCRTHTTPClient = FakeTransport


class FakeBoto3Session:
    region_name: str | None = None


class FakeBoto3(ModuleType):
    def __init__(self) -> None:
        super().__init__("boto3")
        self.Session = FakeBoto3Session


SDK_MODULES: Final[dict[str, ModuleType]] = {
    "aws_sdk_bedrock_runtime": FakeSdkRuntime(),
    "aws_sdk_bedrock_runtime.client": FakeSdkClient(),
    "aws_sdk_bedrock_runtime.config": FakeSdkConfig(),
    "aws_sdk_bedrock_runtime.models": FakeSdkModels(),
    "boto3": FakeBoto3(),
    "smithy_aws_core": FakeSmithyCore(),
    "smithy_aws_core.identity": FakeSmithyCoreIdentity(),
    "smithy_http": FakeSmithyHttp(),
    "smithy_http.aio.crt": FakeSmithyHttpCrt(),
}

RECORDING: Final = Recording()
SCRIPT: Final[list[Script]] = []
CLIENTS: Final[list[FakeRuntimeClient]] = []
