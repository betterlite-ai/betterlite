from typing_extensions import override


class RetryText:
    message: str
    num_retries: int | None
    max_retries: int | None

    @override
    def __str__(self) -> str:
        text = self.message
        if self.num_retries:
            text += f" LiteLLM Retried: {self.num_retries} times"
        if self.max_retries:
            text += f", LiteLLM Max Retries: {self.max_retries}"
        return text

    @override
    def __repr__(self) -> str:
        return str(self)
