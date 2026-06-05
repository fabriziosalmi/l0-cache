# Homebrew formula for l0-cache.
#
# Install (no separate tap repo needed — Homebrew accepts an explicit git URL):
#
#   brew tap fabriziosalmi/l0-cache https://github.com/fabriziosalmi/l0-cache
#   brew install l0-cache
#
# Builds from source; Homebrew provides the Rust toolchain as a build-time dep.
class L0Cache < Formula
  desc "CLI proxy that cuts LLM token use via universal output filtering"
  homepage "https://github.com/fabriziosalmi/l0-cache"
  url "https://github.com/fabriziosalmi/l0-cache/archive/refs/tags/v0.1.8.tar.gz"
  sha256 "REPLACE_WITH_SHA256"
  license "MIT"
  head "https://github.com/fabriziosalmi/l0-cache.git", branch: "master"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
    # The short `t` alias, like the project's own installer creates.
    bin.install_symlink bin/"l0-cache" => "t"
  end

  test do
    assert_match "l0-cache", shell_output("#{bin}/l0-cache --version")
    # End-to-end: filtering a large output truncates with the banner.
    out = shell_output("#{bin}/l0-cache --no-auto --head 5 --tail 5 --threshold 10 seq 1 100")
    assert_match "Showing 5 head + 5 tail of 100 lines", out
  end
end
