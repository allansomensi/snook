<#
.SYNOPSIS
  Packs the game as an MSIX for the Microsoft Store (see docs/MSSTORE.md).

.DESCRIPTION
  Takes a snook.exe built for the Store
      $env:RUSTFLAGS = "-C target-feature=+crt-static"
      cargo build --release --locked -p snook --features store
  fills in crates/client/msix/AppxManifest.xml and packs it with the
  Assets folder using makeappx (Windows SDK). The package is not signed:
  the Store signs it. To try it on this computer without packing, use
  -Register (needs Developer Mode on).

  The identity values come from Partner Center (Product management →
  Product identity): Package/Identity/Name, Package/Identity/Publisher and
  Package/Properties/PublisherDisplayName.

.EXAMPLE
  ./tools/msix/make-msix.ps1 -IdentityName 12345Me.MyPool -Publisher "CN=ABCD-1234" `
    -PublisherDisplayName "Me" -DisplayName "My Pool"
#>
param(
    [Parameter(Mandatory)] [string] $IdentityName,
    [Parameter(Mandatory)] [string] $Publisher,
    [Parameter(Mandatory)] [string] $PublisherDisplayName,
    [Parameter(Mandatory)] [string] $DisplayName,
    [string] $Description = "3D eight-ball pool: play online, with friends or against the computer.",
    # Defaults to the workspace version (0.1.1 becomes 0.1.1.0).
    [string] $Version,
    [string] $Exe = "target/release/snook.exe",
    [string] $Out = "target/msix",
    # Register the unpacked package on this computer instead of packing it.
    [switch] $Register
)
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "../..")
Set-Location $root

# The version: the Store wants four numbers, the last one 0.
if (-not $Version) {
    $line = Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    $Version = $line.Matches[0].Groups[1].Value
}
if ($Version -notmatch '^(\d+)\.(\d+)\.(\d+)$') {
    throw "Only final versions go to the Store (x.y.z, no beta): got '$Version'."
}
$msixVersion = "$Version.0"

if (-not (Test-Path $Exe)) { throw "No game at ${Exe}: build it first (see the description)." }

$stage = Join-Path $Out "package"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item $Exe (Join-Path $stage "snook.exe")
Copy-Item -Recurse "crates/client/msix/Assets" (Join-Path $stage "Assets")

function Escape([string] $s) { [System.Security.SecurityElement]::Escape($s) }
$manifest = Get-Content -Raw "crates/client/msix/AppxManifest.xml"
$values = @{
    "IDENTITY_NAME" = $IdentityName
    "PUBLISHER" = $Publisher
    "PUBLISHER_DISPLAY_NAME" = $PublisherDisplayName
    "DISPLAY_NAME" = $DisplayName
    "DESCRIPTION" = $Description
    "VERSION" = $msixVersion
}
foreach ($k in $values.Keys) { $manifest = $manifest.Replace("{{$k}}", (Escape $values[$k])) }
if ($manifest -match '\{\{[A-Z_]+\}\}') { throw "A value was left out of the manifest: $($Matches[0])" }
# Well-formed XML, or stop here (makeappx's errors are less clear).
[xml] $manifest | Out-Null
Set-Content -Path (Join-Path $stage "AppxManifest.xml") -Value $manifest -Encoding UTF8

if ($Register) {
    Add-AppxPackage -Register (Join-Path $stage "AppxManifest.xml")
    Write-Host "Registered ${DisplayName} ${msixVersion}: it is in the Start menu."
    exit 0
}

# makeappx from the newest Windows SDK.
$makeappx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\makeappx.exe" -ErrorAction SilentlyContinue |
    Sort-Object FullName | Select-Object -Last 1
if (-not $makeappx) { throw "makeappx.exe not found: install the Windows SDK." }
$package = Join-Path $Out "snook-$msixVersion-x64.msix"
& $makeappx.FullName pack /o /h SHA256 /d $stage /p $package
if ($LASTEXITCODE -ne 0) { throw "makeappx failed ($LASTEXITCODE)." }
Write-Host "Packed $package"
if ($env:GITHUB_OUTPUT) { "msix=$package" | Out-File -Append -Encoding utf8 $env:GITHUB_OUTPUT }
