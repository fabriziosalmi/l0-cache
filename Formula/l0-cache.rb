# Homebrew formula for l0-cache.
#
# Install (no separate tap repo needed — Homebrew accepts an explicit git URL):
#
#   brew tap fabriziosalmi/l0-cache https://github.com/fabriziosalmi/l0-cache
#   brew install l0-cache
#
# Builds from source; Homebrew provides the Rust toolchain as a build-time dep.
# Besides the `l0-cache` binary (and the `t` alias), this also installs the
# integration helpers: `l0-cache-claude-hook`, `l0-cache-agent-hook`,
# `l0-cache-agent-rules` (the repo's *.sh scripts, runnable without cloning).
class L0Cache < Formula
  desc "CLI proxy that cuts LLM token use via universal output filtering"
  homepage "https://github.com/fabriziosalmi/l0-cache"
  url "https://github.com/fabriziosalmi/l0-cache/archive/refs/tags/v0.1.13.tar.gz"
  sha256 "e1d948d744bca023a3f153ab708844e92912e34614b7755fc858dfca5864bc7c"
  license "MIT"
  head "https://github.com/fabriziosalmi/l0-cache.git", branch: "master"

  depends_on "rust" => :build
  # Runtime: the bundled hook managers use jq to edit the agent's settings.json.
  depends_on "jq"

  def install
    system "cargo", "install", *std_cargo_args
    # The short `t` alias, like the project's own installer creates.
    bin.install_symlink bin/"l0-cache" => "t"

    # Standalone integration setup scripts — they depend only on l0-cache + jq
    # on PATH (no sibling files), so Homebrew users get the transparent-hook
    # tooling without cloning the repo. Exposed under namespaced commands.
    libexec.install "claude-hook.sh", "agent-hook.sh", "agent-rules.sh"
    bin.install_symlink libexec/"claude-hook.sh" => "l0-cache-claude-hook"
    bin.install_symlink libexec/"agent-hook.sh"  => "l0-cache-agent-hook"
    bin.install_symlink libexec/"agent-rules.sh" => "l0-cache-agent-rules"
  end

  test do
    assert_match "l0-cache", shell_output("#{bin}/l0-cache --version")
    # End-to-end: filtering a large output truncates with the banner.
    out = shell_output("#{bin}/l0-cache --no-auto --head 5 --tail 5 --threshold 10 seq 1 100")
    assert_match "Showing 5 head + 5 tail of 100 lines", out
    # The bundled Claude Code hook manager is installed and self-describes
    # (side-effect-free: `help` neither needs jq nor edits any settings).
    assert_match "Claude Code", shell_output("#{bin}/l0-cache-claude-hook help")
  end
end
