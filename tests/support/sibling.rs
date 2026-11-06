//! Rust adapter to this repo's single sibling-pin resolver.
use std::{path::Path, path::PathBuf, process::Command, sync::OnceLock};

pub fn isa() -> &'static Path {
    static ISA: OnceLock<PathBuf> = OnceLock::new();
    ISA.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let output = Command::new("python3")
            .arg("-I")
            .arg(root.join("scripts/pinned_sibling.py"))
            .arg(root)
            .arg("bennett-isa-spec")
            .output()
            .expect("run scripts/pinned_sibling.py");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        PathBuf::from(
            String::from_utf8(output.stdout)
                .expect("sibling path UTF-8")
                .trim(),
        )
    })
}
