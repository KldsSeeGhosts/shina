<#
.SYNOPSIS
Publishes the version in Cargo.toml as a GitHub Release that installed copies of Shina update to.

.DESCRIPTION
Run from a clean, up-to-date main after merging a PR that bumped Cargo.toml's version.
Builds the release executable in a separate target directory (so a running dev or installed
Shina never locks it), then creates the vX.Y.Z release with shina.exe attached.
Pass -Install to also install the new build on this PC.
#>
param([switch]$Install)
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

cargo build --release --target-dir target/publish
if ($LASTEXITCODE -ne 0) { throw 'The release build failed.' }
$exe = Resolve-Path target/publish/release/shina.exe

$notes = (git log -1 --format=%B) -join "`n"
gh release create $tag $exe --repo KldsSeeGhosts/shina --target (git rev-parse HEAD) --title "Shina $version" --notes $notes
if ($LASTEXITCODE -ne 0) { throw 'Creating the GitHub release failed.' }
Write-Host "Published $tag."

if ($Install) {
    & $exe --install
    Write-Host 'Installed and started the new build.'
}
