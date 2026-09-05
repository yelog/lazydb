class Lazydb < Formula
  desc "A keyboard-first terminal database IDE"
  homepage "https://github.com/yelog/lazydb"
  version "__VERSION__"
  license "MIT"

  on_macos do
    if Hardware::CPU.intel?
      url "https://github.com/yelog/lazydb/releases/download/__TAG__/lazydb___VERSION___x86_64-apple-darwin.tar.xz"
      sha256 "__SHA256_INTEL__"
    else
      url "https://github.com/yelog/lazydb/releases/download/__TAG__/lazydb___VERSION___aarch64-apple-darwin.tar.xz"
      sha256 "__SHA256_ARM__"
    end
  end

  def install
    bin.install "lazydb"
  end

  def caveats
    <<~EOS
      To configure LazyDB for Claude Code, Codex, or OpenCode, run:
        lazydb mcp setup
      from inside your project.
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/lazydb version --json")
  end
end
