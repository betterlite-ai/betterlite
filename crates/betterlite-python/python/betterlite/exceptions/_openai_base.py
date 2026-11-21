from pydantic import JsonValue

from betterlite.exceptions._http import Request, Response


class OpenAIError(Exception):
    pass


class APIError(OpenAIError):
    message: str
    request: Request | None
    body: JsonValue | bytes | None
    code: JsonValue = None
    param: JsonValue = None
    type: JsonValue

    def __init__(
        self, message: str, request: Request, *, body: JsonValue | bytes | None
    ) -> None:
        super().__init__(message)
        self.request = request
        self.message = message
        self.body = body
        match body:
            case dict() as fields:
                self.code = fields.get("code")
                self.param = fields.get("param")
                self.type = fields.get("type")
            case list() | str() | bool() | float() | int() | bytes() | None:
                self.code = None
                self.param = None
                self.type = None


class APIResponseValidationError(APIError):
    response: Response | None
    status_code: int

    def __init__(
        self, response: Response, body: JsonValue | bytes | None, *, message: str | None = None
    ) -> None:
        super().__init__(
            message if message is not None else "Data returned by API invalid for expected schema.",
            response.request,
            body=body,
        )
        self.response = response
        self.status_code = response.status_code


class APIStatusError(APIError):
    response: Response | None
    status_code: int
    request_id: str | None

    def __init__(
        self, message: str, *, response: Response, body: JsonValue | bytes | None
    ) -> None:
        super().__init__(message, response.request, body=body)
        self.response = response
        self.status_code = response.status_code
        self.request_id = response.headers.get("x-request-id")


class APIConnectionError(APIError):
    def __init__(self, *, message: str = "Connection error.", request: Request) -> None:
        super().__init__(message, request, body=None)


class APITimeoutError(APIConnectionError):
    def __init__(self, request: Request) -> None:
        super().__init__(message="Request timed out.", request=request)


class BadRequestError(APIStatusError):
    status_code: int = 400


class AuthenticationError(APIStatusError):
    status_code: int = 401


class PermissionDeniedError(APIStatusError):
    status_code: int = 403


class NotFoundError(APIStatusError):
    status_code: int = 404


class ConflictError(APIStatusError):
    status_code: int = 409


class UnprocessableEntityError(APIStatusError):
    status_code: int = 422


class RateLimitError(APIStatusError):
    status_code: int = 429


class InternalServerError(APIStatusError):
    pass
