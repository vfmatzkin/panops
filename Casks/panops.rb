cask "panops" do
  # Version and sha256 are updated at release time per docs/release-v0.1.md.
  # Run `scripts/package.sh v0.1.0` and use the output sha256.
  version "0.1.0"
  sha256 "REPLACE_WITH_TARBALL_SHA256_FROM_PACKAGE_SH_OUTPUT"  # e.g. "a1b2c3d4..."

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
