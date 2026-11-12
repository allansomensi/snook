//! On Windows: the emblem as the executable's icon, and the version details
//! Explorer and the Task Manager show ("Snook" instead of "snook.exe").

use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=assets/icon/snook.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    // major.minor.patch, then the pre-release number (0.1.1-beta.2 is
    // 0.1.1.2, as in the .msi), so each build has its own file version: the
    // installer doesn't replace a snook.exe whose version is the same.
    let (core, pre) = version.split_once('-').unwrap_or((&version, ""));
    let mut parts = core.split('.').map(|p| p.parse::<u16>().unwrap_or(0));
    let mut next = || parts.next().unwrap_or(0);
    let pre = pre.rsplit('.').next().and_then(|n| n.parse::<u16>().ok()).unwrap_or(0);
    let numeric = format!("{},{},{},{pre}", next(), next(), next());
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest.join("assets/icon/snook.ico").display().to_string().replace('\\', "/");
    // Written out with the values in place: no macros to quote for rc.exe or windres.
    let rc = format!(
        r#"#include <winver.h>

1 ICON "{icon}"

VS_VERSION_INFO VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
FILESUBTYPE VFT2_UNKNOWN
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "FileDescription", "Snook"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "snook"
            VALUE "LegalCopyright", "MIT License"
            VALUE "OriginalFilename", "snook.exe"
            VALUE "ProductName", "Snook"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    );
    let path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("snook.rc");
    std::fs::write(&path, rc).unwrap();
    embed_resource::compile(&path, embed_resource::NONE).manifest_optional().unwrap();
}
