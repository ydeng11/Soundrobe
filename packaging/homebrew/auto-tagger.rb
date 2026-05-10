class AutoTagger < Formula
  include Language::Python::Virtualenv

  desc "Intelligent audio file tagging CLI tool"
  homepage "https://github.com/auto-tagger/auto-tagger"
  url "https://files.pythonhosted.org/packages/source/a/auto-tagger/auto_tagger-0.1.0.tar.gz"
  sha256 "UPDATE_AFTER_PYPI_RELEASE"
  license "MIT"

  depends_on "python@3.12"
  depends_on "ffmpeg"

  def install
    virtualenv_install_with_resources
  end

  test do
    assert_match "0.1.0", shell_output("#{bin}/auto-tag --version")
  end
end
