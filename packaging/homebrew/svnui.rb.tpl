class Svnui < Formula
  desc "Subversion with a real TUI: status, diff, commit, conflicts"
  homepage "https://github.com/@@OWNER@@/svnui"

  url "https://github.com/@@OWNER@@/svnui/releases/download/@@VERSION@@/svnui-@@VERSION@@-x86_64-apple-darwin.tar.gz"
  sha256 "@@SHA_X86@@"

  license "MIT"

  depends_on :macos

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/@@OWNER@@/svnui/releases/download/@@VERSION@@/svnui-@@VERSION@@-aarch64-apple-darwin.tar.gz"
      sha256 "@@SHA_ARM@@"
    end
  end

  def install
    bin.install "svnui"
  end

  test do
    assert_match "@@VERSION@@", shell_output("#{bin}/svnui --version")
  end
end
