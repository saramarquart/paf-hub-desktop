<#
.SYNOPSIS
  Create a self-signed code-signing certificate for the Planet A Foods desktop app,
  and export it as a password-protected .pfx (for CI signing) and a .cer (for IT to
  trust on the team's machines).

.DESCRIPTION
  Run this ONCE on a Windows machine (PowerShell as your normal user is fine).
  It produces two files in the current folder:

    paf-codesign.pfx  -> PRIVATE key + cert. Add to GitHub as secrets so CI can sign
                         the .exe/.msi. NEVER commit this file.
    paf-codesign.cer  -> PUBLIC cert only. IT imports this into Trusted Root +
                         Trusted Publishers on each machine so Windows trusts the
                         signature and skips the SmartScreen prompt.

  See ../SIGNING.md for the full workflow (secrets + trusting the cert).

.PARAMETER Password
  Password used to protect the exported .pfx. You will also store this as the
  WINDOWS_CERT_PASSWORD GitHub secret. Choose a strong value.

.PARAMETER Years
  Validity in years (default 5). When it expires, re-run this and re-distribute.
#>
param(
  [Parameter(Mandatory = $true)]
  [string]$Password,
  [int]$Years = 5
)

$ErrorActionPreference = "Stop"

$subject = "CN=Planet A Foods, O=Planet A Foods, C=DE"
$notAfter = (Get-Date).AddYears($Years)

Write-Host "Creating self-signed code-signing certificate: $subject"

# EnhancedKeyUsage 1.3.6.1.5.5.7.3.3 = Code Signing.
$cert = New-SelfSignedCertificate `
  -Type CodeSigningCert `
  -Subject $subject `
  -KeyUsage DigitalSignature `
  -KeyAlgorithm RSA `
  -KeyLength 3072 `
  -HashAlgorithm SHA256 `
  -CertStoreLocation "Cert:\CurrentUser\My" `
  -NotAfter $notAfter `
  -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3")

Write-Host "Created cert with thumbprint: $($cert.Thumbprint)"

$pfxPath = Join-Path (Get-Location) "paf-codesign.pfx"
$cerPath = Join-Path (Get-Location) "paf-codesign.cer"
$securePwd = ConvertTo-SecureString -String $Password -Force -AsPlainText

Export-PfxCertificate -Cert $cert -FilePath $pfxPath -Password $securePwd | Out-Null
Export-Certificate   -Cert $cert -FilePath $cerPath | Out-Null

Write-Host ""
Write-Host "Wrote:"
Write-Host "  $pfxPath   (PRIVATE - GitHub secrets, do not commit)"
Write-Host "  $cerPath   (PUBLIC  - give to IT to trust on machines)"
Write-Host ""
Write-Host "Next: base64-encode the .pfx for the GitHub secret:" -ForegroundColor Cyan
Write-Host '  [Convert]::ToBase64String([IO.File]::ReadAllBytes("paf-codesign.pfx")) | Set-Clipboard'
Write-Host "  -> paste as secret WINDOWS_CERT_BASE64"
Write-Host "  -> store your password as secret WINDOWS_CERT_PASSWORD"
