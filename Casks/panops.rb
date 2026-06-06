cask "panops" do
  version "0.1.0"
  sha256 "REPLACE_WITH_TARBALL_SHA256"

  url "https://github.com/vfmatzkin/panops/releases/download/v#{version}/Panops-#{version}.tar.gz"
  name "Panops"
  desc "Local-first macOS recorder with screenshot-anchored meeting notes"
  homepage "https://github.com/vfmatzkin/panops"

  depends_on macos: ">= :sequoia"
  app "Panops.app"

  caveats <<~EOS
    Panops is ad-hoc signed (not Apple-notarized). On first launch macOS
    Gatekeeper will block it. Clear the quarantine flag once:

      xattr -dr com.apple.quarantine "#{appdir}/Panops.app"

    or right-click the app → Open, then confirm in System Settings → Privacy & Security.
  EOS
end