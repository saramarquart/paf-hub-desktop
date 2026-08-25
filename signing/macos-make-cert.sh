#!/usr/bin/env bash
# Create a self-signed code-signing identity for the Planet A Foods desktop app
# on macOS, and add it to the login keychain.
#
# IMPORTANT — read SIGNING.md. A self-signed identity makes the app *signed* but
# NOT *notarized*. Trusting this cert removes the "unidentified developer"
# identity complaint, but a DOWNLOADED .dmg is still quarantine-flagged, so
# Gatekeeper still prompts unless you also strip the quarantine attribute
# (see install-macos.sh). Real zero-prompt requires a $99 Apple Developer
# account for notarization — there is no free way around that.
#
# Usage:
#   ./macos-make-cert.sh                 # creates identity "Planet A Foods (self-signed)"
#   IDENTITY_NAME="My Name" ./macos-make-cert.sh
set -euo pipefail

IDENTITY_NAME="${IDENTITY_NAME:-Planet A Foods (self-signed)}"
WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

echo "Creating self-signed code-signing identity: $IDENTITY_NAME"

# OpenSSL config: code-signing EKU (1.3.6.1.5.5.7.3.3).
cat > "$WORKDIR/codesign.cnf" <<EOF
[ req ]
distinguished_name = dn
x509_extensions    = v3
prompt             = no
[ dn ]
CN = $IDENTITY_NAME
O  = Planet A Foods
C  = DE
[ v3 ]
basicConstraints       = critical,CA:false
keyUsage               = critical,digitalSignature
extendedKeyUsage       = critical,codeSigning
EOF

openssl req -x509 -newkey rsa:3072 -nodes \
  -keyout "$WORKDIR/key.pem" \
  -out    "$WORKDIR/cert.pem" \
  -days 1825 \
  -config "$WORKDIR/codesign.cnf" >/dev/null 2>&1

# Bundle key + cert into a .p12 for import.
openssl pkcs12 -export \
  -inkey "$WORKDIR/key.pem" \
  -in    "$WORKDIR/cert.pem" \
  -out   "$WORKDIR/identity.p12" \
  -passout pass:paf >/dev/null 2>&1

# Import into the login keychain and trust it for code signing.
security import "$WORKDIR/identity.p12" -k ~/Library/Keychains/login.keychain-db \
  -P paf -T /usr/bin/codesign >/dev/null

# Mark the cert as trusted for code signing (may prompt for your login password).
security add-trusted-cert -d -r trustAsRoot \
  -p codeSign \
  -k ~/Library/Keychains/login.keychain-db \
  "$WORKDIR/cert.pem" || true

echo ""
echo "Done. Verify the identity is available:"
echo "  security find-identity -v -p codesigning"
echo ""
echo "Then set it in src-tauri/tauri.conf.json under bundle.macOS.signingIdentity:"
echo "  \"signingIdentity\": \"$IDENTITY_NAME\""
echo ""
echo "NOTE: this only signs; downloaded apps are still quarantined. Users still"
echo "run install-macos.sh (quarantine strip). See SIGNING.md."
