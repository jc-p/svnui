class Svnui < Formula
  desc "Subversion with a real TUI: status, diff, commit, conflicts"
  homepage "https://github.com/jc-p/svnui"
  license "MIT"
  version "0.1.0"

  # 预编译二进制，不从源码构建。
  #
  # 为什么不 depends_on "rust" + cargo install：
  # 1. brew 的构建沙箱里 cargo 访问不了 ~/.cargo（依赖下载会失败）
  # 2. 编译时间从 ~10 秒变成 ~3 分钟，还白耗 GitHub runner 资源
  # 3. 用户装个 5MB 的二进制不该先装整个 Rust 工具链
  on_macos do
    on_arm do
      url "https://github.com/jc-p/svnui/releases/download/v0.1.0/svnui-0.1.0-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/jc-p/svnui/releases/download/v0.1.0/svnui-0.1.0-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  def install
    bin.install "svnui"
  end

  test do
    # --version 由 clap 的 #[command(version)] 提供
    assert_match "0.1.0", shell_output("#{bin}/svnui --version")
  end
end
