class Lite < Formula
  desc "Thin client for the betterlite proxy: lite login, lite claude/codex/opencode"
  homepage "https://github.com/betterlite-ai/betterlite"
  url "https://github.com/betterlite-ai/betterlite/archive/refs/tags/vREPLACE_AT_RELEASE.tar.gz"
  sha256 "REPLACE_AT_RELEASE"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/betterlite-cli")
  end

  test do
    assert_match "login", shell_output("#{bin}/lite --help")
  end
end
