import argparse
import os
import shutil
import subprocess
import tempfile
from pathlib import Path


def git(root: Path, *arguments: str) -> bytes:
    return subprocess.check_output(["git", "-C", str(root), *arguments], stderr=subprocess.PIPE)


def active_checkout(root: Path) -> int | None:
    root = root.resolve()
    for entry in Path("/proc").glob("[0-9]*/cwd"):
        if entry.parent.name == str(os.getpid()):
            continue
        try:
            cwd = entry.resolve(strict=True)
            if cwd == root or root in cwd.parents:
                return int(entry.parent.name)
        except (OSError, RuntimeError):
            continue
    return None


def configure(root: Path, source: Path, coordinator: Path) -> None:
    remote = git(source, "remote", "get-url", "origin").decode().strip()
    git(root, "remote", "set-url", "origin", remote)
    git(root, "config", "betterlite.coordinator", str(coordinator))
    git(root, "config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")
    for key in ("user.name", "user.email", "commit.gpgsign", "core.hooksPath"):
        setting = subprocess.run(["git", "-C", str(source), "config", "--get", key], capture_output=True, check=False)
        if setting.returncode == 0:
            git(root, "config", key, setting.stdout.decode().strip())
    for name in ("pre-commit", "pre-push"):
        hook = coordinator / "hooks" / name
        if hook.is_file():
            shutil.copy2(hook, root / ".git" / "hooks" / name)


def isolate(source: Path, target: Path, branch: str) -> str:
    source, target = source.resolve(), target.resolve()
    coordinator = Path(git(source, "rev-parse", "--path-format=absolute", "--git-common-dir").decode().strip())
    dotgit = target / ".git"
    if dotgit.is_symlink():
        raise RuntimeError("refusing a symlinked .git")
    if dotgit.is_dir():
        if git(target, "branch", "--show-current").decode().strip() != branch:
            raise RuntimeError("existing checkout is on a different task branch")
        configure(target, source, coordinator)
        return "already isolated"
    if not dotgit.exists():
        subprocess.run(["git", "clone", "--quiet", "--local", "--no-checkout", str(source), str(target)], check=True)
        retained = subprocess.run(
            ["git", "-C", str(source), "show-ref", "--verify", "--quiet", f"refs/heads/{branch}"], check=False
        )
        base = f"origin/{branch}" if retained.returncode == 0 else "origin/main"
        git(target, "switch", "--no-track", "-c", branch, base)
        configure(target, source, coordinator)
        return "created isolated checkout"
    pid = active_checkout(target)
    if pid is not None:
        raise RuntimeError(f"checkout is still used by process {pid}; stop it before migration")
    old_git = Path(git(target, "rev-parse", "--absolute-git-dir").decode().strip())
    if old_git.parent != coordinator / "worktrees":
        raise RuntimeError("linked checkout does not belong to this coordinator")
    operations = ("MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "rebase-merge", "rebase-apply", "BISECT_LOG")
    if any((old_git / name).exists() for name in operations) or any(old_git.glob("*.lock")):
        raise RuntimeError("checkout has an unfinished Git operation; finish or recover it before migration")
    head = git(target, "rev-parse", "HEAD").decode().strip()
    actual_branch = git(target, "symbolic-ref", "--short", "HEAD").decode().strip()
    if actual_branch != branch:
        raise RuntimeError("linked checkout is on a different task branch")
    before = git(target, "status", "--porcelain=v1", "-z", "--untracked-files=all")
    backup = coordinator / "betterlite" / "isolation" / target.name
    backup.mkdir(parents=True, exist_ok=True)
    if (backup / "gitfile").exists():
        raise RuntimeError("a previous migration backup exists; inspect it before retrying")
    with tempfile.TemporaryDirectory(prefix=".git-isolation-", dir=target.parent) as scratch:
        clone = Path(scratch) / "checkout"
        subprocess.run(["git", "clone", "--quiet", "--local", "--no-checkout", str(source), str(clone)], check=True)
        git(clone, "update-ref", f"refs/heads/{actual_branch}", head)
        git(clone, "symbolic-ref", "HEAD", f"refs/heads/{actual_branch}")
        configure(clone, source, coordinator)
        shutil.copy2(old_git / "index", clone / ".git" / "index")
        dotgit.rename(backup / "gitfile")
        try:
            (clone / ".git").rename(dotgit)
            after = git(target, "status", "--porcelain=v1", "-z", "--untracked-files=all")
            if before != after or git(target, "rev-parse", "HEAD").decode().strip() != head:
                raise RuntimeError("migration changed HEAD or index/worktree status")
            old_git.rename(backup / "linked-metadata")
        except BaseException:
            if dotgit.is_dir():
                dotgit.rename(clone / ".git")
            (backup / "gitfile").rename(dotgit)
            raise
    return "isolated existing checkout; HEAD, index and working files preserved"


class Arguments(argparse.Namespace):
    source: Path
    target: Path
    branch: str


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="Give task checkouts private Git metadata without changing their working files."
    )
    parser.add_argument("source", type=Path)
    parser.add_argument("target", type=Path)
    parser.add_argument("branch")
    args = parser.parse_args(namespace=Arguments())
    print(isolate(args.source, args.target, args.branch))
