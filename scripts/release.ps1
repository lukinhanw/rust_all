# Publishes a GitHub release with a fresh WinRust.exe.
#   release.ps1 v0.2.0 [-Notes "text"]
# Rebuilds everything (build-all.ps1), pushes pending changes, tags, uploads the exe.
# For the auto-updater (src/update) it also uploads manifest.txt and manifest.sig, signed with the
# private key in %USERPROFILE%\.winrust\update-signing.key (tools/winrust-sign). Without that key the release stops.
param([Parameter(Mandatory)][string]$Version, [string]$Notes)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
Set-Location $root
if ($Version -notmatch '^v\d+\.\d+\.\d+$') { throw 'version must look like v0.2.0' }
# the installer shows its own version (CARGO_PKG_VERSION) and compares it with the latest release
$toml = Get-Content "$root\Cargo.toml" -Raw
$toml = [regex]::Replace($toml, '(?m)^version = "[^"]*"', "version = `"$($Version.TrimStart('v'))`"", 1)
[IO.File]::WriteAllText("$root\Cargo.toml", $toml)
& "$PSScriptRoot\build-all.ps1"
cargo build --release -p winrust-sign; if ($LASTEXITCODE) { throw 'winrust-sign build failed' }
# signed before anything is pushed or tagged: a missing key must not leave a half-made release
$dir = Join-Path $env:TEMP "winrust-release-$Version"
if (Test-Path $dir) { Remove-Item $dir -Recurse -Force }
New-Item -ItemType Directory $dir | Out-Null
$exe = Join-Path $dir 'WinRust.exe'
Copy-Item "$root\target\release\plugin-launcher.exe" $exe -Force
$sha = { param($p) (Get-FileHash $p -Algorithm SHA256).Hash.ToLower() }
# manifest names are the payload.txt names; the asset gets dots for spaces (GitHub would rename it that way anyway)
$lines = @('winrust-manifest 1', "version $Version", "$(& $sha $exe) WinRust.exe")
# only WinRust.exe is attached (the plugins ride inside it); the manifest lists it alone, which the auto-updater accepts
$manifest = Join-Path $dir 'manifest.txt'
[IO.File]::WriteAllText($manifest, ($lines -join "`n") + "`n", (New-Object Text.UTF8Encoding $false))
& "$root\target\release\winrust-sign.exe" sign $manifest (Join-Path $dir 'manifest.sig')
if ($LASTEXITCODE) { throw 'signing failed: is the key in %USERPROFILE%\.winrust\update-signing.key?' }
$assets = @(Get-ChildItem $dir | ForEach-Object FullName)
& "$PSScriptRoot\publish.ps1" -Message "Release $Version"
git tag $Version
git push origin $Version
if ($Notes) {
    # through a file: Windows PowerShell mangles double quotes inside a multi-line argument to a native exe
    $nf = Join-Path $env:TEMP "winrust-notes-$Version.md"
    [IO.File]::WriteAllText($nf, $Notes, (New-Object Text.UTF8Encoding $false))
    gh release create $Version @assets --repo Lemxy/WinRust --title "WinRust $Version" --notes-file $nf
    Remove-Item $nf
}
else { gh release create $Version @assets --repo Lemxy/WinRust --title "WinRust $Version" --generate-notes }
if ($LASTEXITCODE) { throw "gh release create failed (the tag $Version is already pushed: rerun only this step)" }
