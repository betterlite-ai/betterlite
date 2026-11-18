from .values import RouterCase


class CasePatternRouter:
    def __init__(self, patterns: list[str]) -> None:
        self.patterns = patterns

    def route(self, model: str) -> str | None:
        for pattern in self.patterns:
            if pattern.endswith("*"):
                matched = model.startswith(pattern.removesuffix("*"))
            else:
                matched = model == pattern
            if matched:
                return model
        return None


class CaseRouter:
    def __init__(self, case: RouterCase) -> None:
        self.recognized: set[str] = set(case["recognized"])
        self.team_public_model_names: list[str] = list(case["team_public"])
        self.pattern_router = CasePatternRouter(case["patterns"])
        self.team_pattern_routers: dict[str, CasePatternRouter] = {
            "team": CasePatternRouter(case["team_patterns"])
        }

    # ast-grep-ignore: no-one-line-functions
    def is_recognized_model(self, model: str) -> bool:
        return model in self.recognized
