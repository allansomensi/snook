//! Where this copy of the game was distributed, and what that changes.
//!
//! The same code makes every build. The Microsoft Store build (feature
//! `store`, see `docs/MSSTORE.md`) sells nothing for real money through the
//! game's own payment pages: the Store's policies ask games to sell digital
//! items through the Store, so until that is wired up, coins there come
//! from playing (rated games and the daily bonus) and VIP is not offered.

/// This is the Microsoft Store build.
pub const STORE: bool = cfg!(feature = "store");

/// Coin packs and VIP may be bought through the game's payment pages.
pub const fn real_money_purchases() -> bool {
    !STORE
}

#[cfg(test)]
mod tests {
    use snookered_proto::{LAN_DISCOVERY_PORT, LAN_GAME_PORT};

    /// The Store package's manifest stays in step with the game: the
    /// executable's name, the local network ports it opens in the firewall,
    /// the pictures it names and the values the packing script fills in.
    #[test]
    fn store_manifest_matches_the_game() {
        let m = include_str!("../msix/AppxManifest.xml");
        assert!(m.contains(&format!("Executable=\"{}.exe\"", env!("CARGO_PKG_NAME"))));
        for port in [LAN_GAME_PORT, LAN_DISCOVERY_PORT] {
            assert!(
                m.contains(&format!("LocalPortMin=\"{port}\" LocalPortMax=\"{port}\"")),
                "port {port} is not opened in the manifest's firewall rules"
            );
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("msix");
        let files: Vec<String> = std::fs::read_dir(dir.join("Assets"))
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .collect();
        for asset in m.split("Assets\\").skip(1).filter_map(|s| s.split('"').next()) {
            let asset = asset.split('<').next().unwrap_or(asset);
            // The file itself, or versions of it with qualifiers
            // (`Square44x44Logo.scale-200.png`), which resources.pri indexes.
            let (stem, ext) = asset.rsplit_once('.').unwrap();
            let found = files.iter().any(|f| {
                f == asset
                    || (f.starts_with(&format!("{stem}.")) && f.ends_with(&format!(".{ext}")))
            });
            assert!(found, "missing msix/Assets/{asset}");
        }
        // The small icon in every size the shell asks for, without the
        // accent-coloured plate behind it.
        for size in crate::icon::tests::MSIX_TARGET_SIZES {
            for form in ["", "_altform-unplated", "_altform-lightunplated"] {
                let name = format!("Square44x44Logo.targetsize-{size}{form}.png");
                assert!(files.contains(&name), "missing msix/Assets/{name}");
            }
        }
        for key in
            ["IDENTITY_NAME", "PUBLISHER", "PUBLISHER_DISPLAY_NAME", "DISPLAY_NAME", "VERSION"]
        {
            assert!(m.contains(&format!("{{{{{key}}}}}")), "{key} is filled in by make-msix.ps1");
        }
    }
}
