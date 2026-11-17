# Homebrew formula for the `lite` CLI

[`lite.rb`](./lite.rb) is the Homebrew formula that installs betterlite's thin client, the `lite` command with `lite login`, `lite claude` and `lite models list`. It lives in this repository so it is versioned with the code it builds, and it is the source of truth: every release copies it into the tap repository `betterlite-ai/homebrew-betterlite` as `Formula/lite.rb`.

The formula builds the Rust crate `crates/betterlite-cli` from the release tag archive with one `cargo install`, so it needs the `rust` build dependency and no other toolchain.

Once the tap is published, end users install with

```shell
brew install betterlite-ai/betterlite/lite
```

which gives them the `lite` command without the proxy server. The server is the separate `betterlite` binary, installed from the container image or with cargo.

## Why a tap and not homebrew-core

The formula builds from a git tag, and betterlite has a moving release cadence: a formula in homebrew-core needs its own review for the first version and for every update after it, which does not fit a CLI that ships with each release. Keeping the formula in the project's own tap lets the formula update land with the tag it points at.

## Release runbook

1. Tag a release `vX.Y.Z` whose workspace version in `Cargo.toml` is `X.Y.Z`, and push the tag.
2. Compute the checksum of the tag archive with `curl -fsSL https://github.com/betterlite-ai/betterlite/archive/refs/tags/vX.Y.Z.tar.gz | sha256sum`.
3. Replace `REPLACE_AT_RELEASE` in the url with `X.Y.Z`, and in `sha256` with the checksum.
4. Copy `lite.rb` to `Formula/lite.rb` in the tap repo, then run `brew install --build-from-source ./Formula/lite.rb`, `brew test lite` and `brew audit --strict --new lite`.
5. Commit and push the tap repo.

Steps 4 and 5 are the only proof of a real Homebrew build and need a published tag, so they are not part of CI.
