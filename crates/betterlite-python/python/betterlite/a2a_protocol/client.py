from collections.abc import AsyncGenerator

from betterlite.a2a_protocol.main import (
    A2AClientType,
    aget_agent_card,
    asend_message,
    asend_message_streaming,
    create_a2a_client,
)
from betterlite.a2a_protocol.types import (
    LiteLLMSendMessageResponse,
    SendMessageRequest,
    SendStreamingMessageRequest,
    SendStreamingMessageResponse,
)
from betterlite.types.utils.base import ResponseObject


class A2AClient:
    base_url: str
    timeout: float
    extra_headers: dict[str, str] | None
    _a2a_client: A2AClientType | None

    def __init__(
        self,
        base_url: str,
        timeout: float = 60.0,
        extra_headers: dict[str, str] | None = None,
    ) -> None:
        self.base_url = base_url
        self.timeout = timeout
        self.extra_headers = extra_headers
        self._a2a_client = None

    async def _get_client(self) -> A2AClientType:
        client = self._a2a_client
        if client is None:
            client = await create_a2a_client(
                base_url=self.base_url,
                timeout=self.timeout,
                extra_headers=self.extra_headers,
            )
            self._a2a_client = client
        return client

    async def get_agent_card(self) -> ResponseObject:
        return await aget_agent_card(
            base_url=self.base_url,
            timeout=self.timeout,
            extra_headers=self.extra_headers,
        )

    async def send_message(self, request: SendMessageRequest) -> LiteLLMSendMessageResponse:
        client = await self._get_client()
        return await asend_message(a2a_client=client, request=request)

    async def send_message_streaming(
        self, request: SendStreamingMessageRequest
    ) -> AsyncGenerator[SendStreamingMessageResponse, None]:
        client = await self._get_client()
        async for chunk in asend_message_streaming(a2a_client=client, request=request):
            yield chunk
