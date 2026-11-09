<#
.SYNOPSIS
Publishes the version in Cargo.toml as a GitHub Release that installed copies of Shina update to.

.DESCRIPTION
Run from a clean, up-to-date main after merging a PR that bumped Cargo.toml's version.
Builds the release executable in a separate target directory (so a running dev or installed
Shina never locks it), then creates the vX.Y.Z release with shina.exe, the macOS app zips and
SHA256SUMS attached. Pass -Install to also install the new build on this PC.

Installed Mac copies can only update when the release has Shina-macos-aarch64.zip (or
Shina-macos-x86_64.zip for Intel) built on that Mac with tools/package-macos.sh. Pass the zip
builds in -MacZips; without them the release still publishes, but Macs will not update.
#>
param(
    [switch]$Install,
    [string[]]$MacZips = @()
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

$branch = git rev-parse --abbrev-ref HEAD
if ($branch -ne 'main') { throw "Release from main, not $branch." }
if (git status --porcelain --untracked-files=no) { throw 'Commit or stash tracked changes first.' }
git fetch -q origin main
if ((git rev-parse HEAD) -ne (git rev-parse origin/main)) { throw 'Pull or push main first.' }

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches.Groups[1].Value
$tag = "v$version"
# "release not found" on stderr is the expected answer; don't let it stop the script.
$ErrorActionPreference = 'Continue'
gh release view $tag --repo KldsSeeGhosts/shina 2>&1 | Out-Null
$released = $LASTEXITCODE -eq 0
$ErrorActionPreference = 'Stop'
if ($released) { throw "$tag is already released. Bump the version in Cargo.toml first." }

# Each macOS asset is a complete Shina.app zipped by tools/package-macos.sh, named for its
# architecture so the update picks the running build's own.
$zips = @()
foreach ($zip in $MacZips) {
    $item = Get-Item -LiteralPath $zip
    if ($item.Name -notmatch '^Shina-macos-(aarch64|x86_64)\.zip$') {
        throw "Name the Mac asset Shina-macos-aarch64.zip or Shina-macos-x86_64.zip, not $($item.Name)."
    }
    $zips += $item
}
if ($zips.Count -eq 0) { Write-Warning 'No macOS assets: installed Macs will not receive this update.' }

cargo build --release --target-dir target/publish
if ($LASTEXITCODE -ne 0) { throw 'The release build failed.' }
# Every asset is a FileInfo, so .FullName and .Name read the same for the exe and the zips.
$exe = Get-Item -LiteralPath 'target/publish/release/shina.exe' -ErrorAction Stop

# SHA256SUMS covers every asset, so a release without per-asset digests still verifies.
$sums = Join-Path $exe.DirectoryName 'SHA256SUMS'
$lines = foreach ($file in @($exe) + $zips) {
    "{0}  {1}" -f (Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash.ToLowerInvariant(), $file.Name
}
Set-Content -NoNewline -Encoding ascii -Path $sums -Value (($lines -join "`n") + "`n")

$assets = @($exe) + $zips + @(Get-Item -LiteralPath $sums)
$notes = (git log -1 --format=%B) -join "`n"
gh release create $tag @($assets | ForEach-Object { $_.FullName }) --repo KldsSeeGhosts/shina --target (git rev-parse HEAD) --title "Shina $version" --notes $notes
if ($LASTEXITCODE -ne 0) { throw 'Creating the GitHub release failed.' }
Write-Host "Published $tag with $(@($assets).Count) assets."

if ($Install) {
    # -Wait would also wait for the app the installer starts; wait for the installer only.
    (Start-Process -FilePath $exe.FullName -ArgumentList '--install' -PassThru).WaitForExit()
    Write-Host 'Installed and started the new build.'
}
