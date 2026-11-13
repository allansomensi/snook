<#
.SYNOPSIS
  Packs the game as an MSIX for the Microsoft Store (see docs/MSSTORE.md).

.DESCRIPTION
  Takes a snookered.exe built for the Store
      $env:RUSTFLAGS = "-C target-feature=+crt-static"
      cargo build --release --locked -p snookered --features store
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
    [string] $Exe = "target/release/snookered.exe",
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
Copy-Item $Exe (Join-Path $stage "snookered.exe")
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

# The Windows SDK tools (makepri, makeappx) from its newest version.
function SdkTool([string] $name) {
    $tool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\$name" -ErrorAction SilentlyContinue |
        Sort-Object FullName | Select-Object -Last 1
    if (-not $tool) { throw "$name not found: install the Windows SDK." }
    $tool.FullName
}

# resources.pri: the index of the pictures' scales and target sizes. Without
# it Windows can't find the "unplated" small icons and draws the emblem on a
# plate in the accent colour (blue corners in the taskbar and Start menu).
$makepri = SdkTool "makepri.exe"
$priconfig = Join-Path $Out "priconfig.xml"
& $makepri createconfig /cf $priconfig /dq en-US /pv 10.0.0 /o | Out-Host
if ($LASTEXITCODE -ne 0) { throw "makepri createconfig failed ($LASTEXITCODE)." }
& $makepri new /pr $stage /cf $priconfig /mn (Join-Path $stage "AppxManifest.xml") /of (Join-Path $stage "resources.pri") /o | Out-Host
if ($LASTEXITCODE -ne 0) { throw "makepri new failed ($LASTEXITCODE)." }

if ($Register) {
    Add-AppxPackage -Register (Join-Path $stage "AppxManifest.xml")
    Write-Host "Registered ${DisplayName} ${msixVersion}: it is in the Start menu."
    exit 0
}

$makeappx = SdkTool "makeappx.exe"
$package = Join-Path $Out "snookered-$msixVersion-x64.msix"
& $makeappx pack /o /h SHA256 /d $stage /p $package
if ($LASTEXITCODE -ne 0) { throw "makeappx failed ($LASTEXITCODE)." }
Write-Host "Packed $package"
if ($env:GITHUB_OUTPUT) { "msix=$package" | Out-File -Append -Encoding utf8 $env:GITHUB_OUTPUT }
