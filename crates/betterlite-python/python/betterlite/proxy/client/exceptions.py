from ._bridge import redact_string
from ._transport import HTTPError

OrigException = HTTPError | str


def _redacted(orig_exception: OrigException) -> OrigException:
    match orig_exception:
        case HTTPError():
            message = redact_string(str(orig_exception))
            return HTTPError(message, response=orig_exception.response)
        case str():
            return redact_string(orig_exception)


class UnauthorizedError(Exception):
    orig_exception: OrigException

    def __init__(self, orig_exception: OrigException) -> None:
        self.orig_exception = _redacted(orig_exception)
        super().__init__(str(self.orig_exception))


class NotFoundError(Exception):
    orig_exception: OrigException

    def __init__(self, orig_exception: OrigException) -> None:
        self.orig_exception = _redacted(orig_exception)
        super().__init__(str(self.orig_exception))
