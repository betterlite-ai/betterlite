from typing_extensions import override

from betterlite.exceptions._http import Request, Response

LLM_PROVIDER = "a2a_agent"
BASE_STATUS = 500
CONNECTION_STATUS = 503
AGENT_CARD_STATUS = 404
DEFAULT_RESPONSE_URL = "https://litellm.ai"


class A2AError(Exception):
    status_code: int
    message: str
    llm_provider: str
    model: str | None
    litellm_debug_info: str | None
    max_retries: int | None
    num_retries: int | None
    response: Response

    def __init__(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/a2a_protocol/exceptions.py#L19
        self,
        message: str,
        status_code: int = BASE_STATUS,
        llm_provider: str = LLM_PROVIDER,
        model: str | None = None,
        response: Response | None = None,
        litellm_debug_info: str | None = None,
        max_retries: int | None = None,
        num_retries: int | None = None,
    ) -> None:
        self.status_code = status_code
        # the prefix belongs to this constructor alone, so a native message arrives without it
        self.message = f"litellm.A2AError: {message}"
        self.llm_provider = llm_provider
        self.model = model
        self.litellm_debug_info = litellm_debug_info
        self.max_retries = max_retries
        self.num_retries = num_retries
        self.response = (
            response
            if response is not None
            else Response(
                status_code=self.status_code,
                request=Request(method="POST", url=DEFAULT_RESPONSE_URL),
            )
        )
        super().__init__(self.message)

    @override
    def __str__(self) -> str:
        message = self.message
        if self.num_retries:
            message += f" LiteLLM Retried: {self.num_retries} times"
        if self.max_retries:
            message += f", LiteLLM Max Retries: {self.max_retries}"
        return message

    @override
    def __repr__(self) -> str:
        return self.__str__()


class A2AConnectionError(A2AError):
    url: str | None

    def __init__(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/a2a_protocol/exceptions.py#L65
        self,
        message: str,
        url: str | None = None,
        model: str | None = None,
        response: Response | None = None,
        litellm_debug_info: str | None = None,
        max_retries: int | None = None,
        num_retries: int | None = None,
    ) -> None:
        self.url = url
        super().__init__(
            message,
            CONNECTION_STATUS,
            LLM_PROVIDER,
            model,
            response,
            litellm_debug_info,
            max_retries,
            num_retries,
        )


class A2AAgentCardError(A2AError):
    url: str | None

    def __init__(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/a2a_protocol/exceptions.py#L98
        self,
        message: str,
        url: str | None = None,
        model: str | None = None,
        response: Response | None = None,
        litellm_debug_info: str | None = None,
        status_code: int = AGENT_CARD_STATUS,
    ) -> None:
        self.url = url
        super().__init__(
            message=message,
            status_code=status_code,
            llm_provider=LLM_PROVIDER,
            model=model,
            response=response,
            litellm_debug_info=litellm_debug_info,
        )


class A2AAgentCardDiscoveryError(A2AAgentCardError):
    failures: tuple[tuple[str, Exception], ...]

    def __init__(
        self,
        base_url: str,
        failures: tuple[tuple[str, Exception], ...],
        status_code: int,
    ) -> None:
        self.failures = failures
        attempts = ", ".join(f"{path} ({error})" for path, error in failures)
        super().__init__(
            message=f"Failed to fetch agent card from {base_url}. Tried {attempts}",
            url=base_url,
            status_code=status_code,
        )


class A2ALocalhostURLError(A2AConnectionError):
    localhost_url: str
    base_url: str
    original_error: Exception | None

    def __init__(
        self,
        localhost_url: str,
        base_url: str,
        original_error: Exception | None = None,
        model: str | None = None,
    ) -> None:
        self.localhost_url = localhost_url
        self.base_url = base_url
        self.original_error = original_error
        message = (
            f"Agent card contains localhost/internal URL '{localhost_url}'. "
            f"Retrying with base URL '{base_url}'."
        )
        super().__init__(message=message, url=localhost_url, model=model)
