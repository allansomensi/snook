//! Migrations the production database has applied must never change: the
//! server refuses to start when one does ("migration N was previously
//! applied but has been modified"), even for a comment. To change the
//! schema, add a new migration file and its line below.

use sha2::{Digest, Sha256};

/// SHA-256 of every migration, as applied in production.
const APPLIED: &[(&str, &str)] = &[
    ("0001_initial.sql", "2a2a412e5d300fe7f26a381bd24d26206cd6e3c983948f8d00b30c5fbf297ae1"),
    ("0002_drop_founder.sql", "0e78d6a89eca791519d09db06131491c798624f3573ee0ed512176f81f5a45a9"),
    ("0003_shop.sql", "f1afdfcd99ed91cfab8232eb8d3ee3dbc1a95832b1501c84ac5ef4c6640b947b"),
    (
        "0004_accounts_replays_stats.sql",
        "80f780c14dc752c522e2f5f49e17763d09cf062bf048e5f30c33845f2b963708",
    ),
];

#[test]
fn applied_migrations_are_unchanged() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    for (name, expected) in APPLIED {
        let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let got: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            &got, expected,
            "{name} was edited: applied migrations must not change (add a new one instead)"
        );
    }
    let on_disk = std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).count();
    assert_eq!(
        on_disk,
        APPLIED.len(),
        "a new migration: add its name and hash to APPLIED once it is released"
    );
}
