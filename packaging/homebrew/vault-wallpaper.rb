# Draft cask for a personal tap (github.com/william-ricchiuti/homebrew-tap, not created yet).
# Copy to Casks/vault-wallpaper.rb in the tap. Steps in docs/release-checklist.md.
#
# Before publishing:
# - `sha256`: run `shasum -a 256 dist/Vault-Wallpaper-<version>.dmg` on the notarized DMG.
# - The download URL must be public. While the vault-wallpaper repo is private, its release
#   assets need a GitHub token, so `brew install` fails for everyone else.
cask "vault-wallpaper" do
  version "1.0.0-beta.1"
  sha256 "REPLACE_WITH_SHA256_OF_THE_NOTARIZED_DMG"

  url "https://github.com/william-ricchiuti/vault-wallpaper/releases/download/v#{version}/Vault-Wallpaper-#{version}.dmg"
  name "Vault Wallpaper"
  desc "Obsidian vault graph as a live desktop wallpaper"
  homepage "https://github.com/william-ricchiuti/vault-wallpaper"

  # Betas are GitHub pre-releases, which :github_latest skips; :github_releases sees them.
  livecheck do
    url :url
    strategy :github_releases
  end

  depends_on macos: ">= :ventura"

  app "Vault Wallpaper.app"

  uninstall quit: "com.williamricchiuti.vaultwallpaper"

  zap trash: [
    "~/Library/Application Support/Vault Wallpaper",
    "~/Library/Caches/com.williamricchiuti.vaultwallpaper",
    "~/Library/Preferences/com.williamricchiuti.vaultwallpaper.plist",
    "~/Library/WebKit/com.williamricchiuti.vaultwallpaper",
  ]
end
