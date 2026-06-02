class Sush < Formula
  desc "Lightweight TUI SSH + SFTP manager"
  homepage "https://github.com/0xlic/sush.sh"
  version "1.1.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/0xlic/sush.sh/releases/download/v1.1.0/sush-aarch64-apple-darwin.tar.xz"
      sha256 "9596977996724c4d9ea787b265bb25e6a307d41f44e154b8552c16b714a79177"
    else
      url "https://github.com/0xlic/sush.sh/releases/download/v1.1.0/sush-x86_64-apple-darwin.tar.xz"
      sha256 "d07161b02e3dfe79af94838cf75010de619fb28b63894633c4234183df51a363"
    end
  end

  def install
    bin.install "sush"
  end

  test do
    assert_path_exists bin/"sush"
    assert_predicate bin/"sush", :executable?
  end
end
