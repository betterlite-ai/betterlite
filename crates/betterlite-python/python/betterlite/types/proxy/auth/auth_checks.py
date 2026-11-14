from typing_extensions import override


class UserNotFoundError(ValueError):
    @override
    def __init__(self, user_id: str) -> None:
        super().__init__(
            f"User doesn't exist in db. 'user_id'={user_id}. Create user via `/user/new` call."
        )
