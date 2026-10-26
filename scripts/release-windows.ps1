# Builds the Windows installers on this computer, signs them with the Certum code signing
# certificate in SimplySign Desktop, and adds them to the tag's draft release that CI made
# for the other systems, with their entries in latest.json.
#
# Needs: SimplySign Desktop signed in, the GitHub CLI signed in, the updater key at
# ~/.tauri/detour-updater.key (or TAURI_SIGNING_PRIVATE_KEY). Its password comes from
# TAURI_SIGNING_PRIVATE_KEY_PASSWORD or is asked for.
#
#   powershell -ExecutionPolicy Bypass -File scripts\release-windows.ps1

param(
  # Build and sign, but don't touch the release.
  [switch]$NoUpload
)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$version = (Get-Content src-tauri\tauri.conf.json -Raw | ConvertFrom-Json).version
$tag = "v$version"
$repo = "INotSleep/Detour"

function Step($text) { Write-Host "`n== $text" -ForegroundColor Cyan }

# ---------- the certificate ----------

Step "Certificate"
$cert = Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert |
  Where-Object { $_.HasPrivateKey -and $_.Issuer -like "*Certum*" -and $_.NotAfter -gt (Get-Date) } |
  Sort-Object NotAfter -Descending | Select-Object -First 1
if (-not $cert) { throw "No Certum code signing certificate: start SimplySign Desktop and sign in." }
Write-Host "$($cert.Subject), until $($cert.NotAfter.ToString('yyyy-MM-dd'))"

$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
  Sort-Object FullName -Descending | Select-Object -First 1
if (-not $signtool) { throw "signtool.exe not found: install the Windows SDK." }

# ---------- the updater key ----------

if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
  $key = Join-Path $HOME ".tauri\detour-updater.key"
  if (-not (Test-Path $key)) { throw "No updater key at $key." }
  $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $key -Raw
}
if ($null -eq $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) {
  $secure = Read-Host "Updater key password (Enter if it has none)" -AsSecureString
  $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = [Runtime.InteropServices.Marshal]::PtrToStringBSTR([Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure))
}

# ---------- build and sign ----------

Step "Building $tag"
$config = Join-Path $env:TEMP "detour-sign.json"
$sign = @{ bundle = @{ windows = @{ certificateThumbprint = $cert.Thumbprint; digestAlgorithm = "sha256"; timestampUrl = "http://time.certum.pl"; tsp = $true } } }
[IO.File]::WriteAllText($config, ($sign | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))
npm run tauri build -- --bundles nsis,msi --config $config
if ($LASTEXITCODE -ne 0) { throw "The build failed." }

$bundle = Join-Path $root "target\release\bundle"
$files = @{
  nsis = Join-Path $bundle "nsis\Detour_${version}_x64-setup.exe"
  msi = Join-Path $bundle "msi\Detour_${version}_x64_en-US.msi"
}

Step "Checking signatures"
foreach ($file in @((Join-Path $root "target\release\detour.exe"), $files.nsis, $files.msi)) {
  & $signtool.FullName verify /pa /q $file
  if ($LASTEXITCODE -ne 0) { throw "Not signed: $file" }
  if (-not (Test-Path "$file.sig") -and $file -ne (Join-Path $root "target\release\detour.exe")) { throw "No update signature: $file.sig" }
  Write-Host "signed  $(Split-Path $file -Leaf)"
}

if ($NoUpload) { exit 0 }

# ---------- the release ----------

Step "Waiting for CI to finish $tag"
while ($true) {
  $runs = gh run list -R $repo -w CI -b $tag --json status,conclusion | ConvertFrom-Json
  if ($runs.Count -and ($runs | Where-Object { $_.status -ne "completed" }).Count -eq 0) {
    if (($runs | Where-Object { $_.conclusion -ne "success" }).Count) { throw "CI for $tag did not succeed." }
    break
  }
  Start-Sleep -Seconds 30
}

Step "Uploading"
$assets = @($files.nsis, "$($files.nsis).sig", $files.msi, "$($files.msi).sig")
gh release upload $tag @assets -R $repo --clobber
if ($LASTEXITCODE -ne 0) { throw "Upload failed." }

# latest.json: the other systems come from CI, Windows is added here (the MSI is the default, as tauri-action has it).
$work = Join-Path $env:TEMP "detour-release"
New-Item -ItemType Directory -Force $work | Out-Null
$latestPath = Join-Path $work "latest.json"
gh release download $tag -R $repo -p latest.json -D $work --clobber
if ($LASTEXITCODE -ne 0) { throw "No latest.json in the $tag release." }
$latest = Get-Content $latestPath -Raw | ConvertFrom-Json
$url = "https://github.com/$repo/releases/download/$tag"
$entry = { param($file) [pscustomobject]@{ signature = (Get-Content "$file.sig" -Raw).Trim(); url = "$url/$(Split-Path $file -Leaf)" } }
$latest.platforms | Add-Member -Force NoteProperty "windows-x86_64" (& $entry $files.msi)
$latest.platforms | Add-Member -Force NoteProperty "windows-x86_64-msi" (& $entry $files.msi)
$latest.platforms | Add-Member -Force NoteProperty "windows-x86_64-nsis" (& $entry $files.nsis)
[IO.File]::WriteAllText($latestPath, ($latest | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))
gh release upload $tag $latestPath -R $repo --clobber
if ($LASTEXITCODE -ne 0) { throw "latest.json upload failed." }

Write-Host "`nDone: the signed Windows installers are in the $tag draft." -ForegroundColor Green
