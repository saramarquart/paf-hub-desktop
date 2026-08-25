#!/usr/bin/env bash
# One-time macOS install helper for the Planet A Foods desktop app.
#
# Why this exists: the app is self-signed, not Apple-notarized. macOS puts a
# `com.apple.quarantine` flag on anything downloaded from the internet, and
# Gatekeeper blocks quarantined apps that aren't notarized. Stripping that flag
# is the real "no-popup" gate on macOS without a $99 Apple Developer account.
#
# Usage — after dragging "Planet A Foods" into /Applications:
#   ./install-macos.sh
# or the one-liner directly:
#   sudo xattr -dr com.apple.quarantine "/Applications/Planet A Foods.app"
set -euo pipefail

APP="/Applications/Planet A Foods.app"

if [ ! -d "$APP" ]; then
  echo "Couldn't find $APP"
  echo "Open the .dmg and drag 'Planet A Foods' into Applications first, then re-run."
  exit 1
fi

echo "Removing quarantine flag from $APP (may ask for your password)..."
sudo xattr -dr com.apple.quarantine "$APP"

echo "Done. You can now open Planet A Foods from Applications without a prompt."
