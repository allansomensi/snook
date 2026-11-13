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

# A tool from the newest Windows SDK.
function Find-SdkTool([string] $name) {
    Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\$name" -ErrorAction SilentlyContinue |
        Sort-Object FullName | Select-Object -Last 1
}

# The resource index. The icons come in several sizes and "unplated" forms
# (Square44x44Logo.targetsize-32_altform-unplated.png…); without
# resources.pri Windows can't find them and draws the icon on a plate of
# the accent colour.
$makepri = Find-SdkTool "makepri.exe"
if (-not $makepri) { throw "makepri.exe not found: install the Windows SDK." }
$priconfig = Join-Path $Out "priconfig.xml"
& $makepri.FullName createconfig /cf $priconfig /dq en-US /pv 10.0.0 /o
if ($LASTEXITCODE -ne 0) { throw "makepri createconfig failed ($LASTEXITCODE)." }
& $makepri.FullName new /pr $stage /cf $priconfig /mn (Join-Path $stage "AppxManifest.xml") `
    /of (Join-Path $stage "resources.pri") /o
if ($LASTEXITCODE -ne 0) { throw "makepri new failed ($LASTEXITCODE)." }

if ($Register) {
    Add-AppxPackage -Register (Join-Path $stage "AppxManifest.xml")
    Write-Host "Registered ${DisplayName} ${msixVersion}: it is in the Start menu."
    exit 0
}

$makeappx = Find-SdkTool "makeappx.exe"
if (-not $makeappx) { throw "makeappx.exe not found: install the Windows SDK." }
$package = Join-Path $Out "snookered-$msixVersion-x64.msix"
& $makeappx.FullName pack /o /h SHA256 /d $stage /p $package
if ($LASTEXITCODE -ne 0) { throw "makeappx failed ($LASTEXITCODE)." }
Write-Host "Packed $package"
if ($env:GITHUB_OUTPUT) { "msix=$package" | Out-File -Append -Encoding utf8 $env:GITHUB_OUTPUT }
