from pydantic import JsonValue

__all__ = ["CustomRoutingStrategyBase"]


class CustomRoutingStrategyBase:
    async def async_get_available_deployment(
        self, model: str, messages: list[dict[str, str]] | None = None,
        input_value: str | list[JsonValue] | None = None, *,
        specific_deployment: bool | None = False,
        request_kwargs: dict[str, JsonValue] | None = None,
    ) -> None: ...

    def get_available_deployment(
        self, model: str, messages: list[dict[str, str]] | None = None,
        input_value: str | list[JsonValue] | None = None, *,
        specific_deployment: bool | None = False,
        request_kwargs: dict[str, JsonValue] | None = None,
    ) -> None: ...
