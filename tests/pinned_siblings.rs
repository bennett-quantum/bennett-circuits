//! Exercise pin failures and the shell reader during the default cargo test run.
#[test]
fn implemented_isa_matches_spec_versions() {
    assert_eq!(bennett_circuits::object::ISA_VERSION, "0.8", "ISA pin");
    let expected = format!("isa = \"{}\"", bennett_circuits::object::ISA_VERSION);
    let pins: Vec<_> = include_str!("../SPEC_VERSIONS.toml")
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("isa = "))
        .collect();
    assert_eq!(pins, [expected.as_str()], "SPEC_VERSIONS vs loader ISA");
}

#[test]
fn resolver_and_conformance_script_obey_pins() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = std::process::Command::new("python3")
        .arg("-I")
        .arg(root.join("scripts/test_pinned_sibling.py"))
        .output()
        .expect("run pin resolver regressions");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn quality_ratchet_detects_growth_in_each_dimension() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = std::process::Command::new("python3")
        .args([
            "-I",
            root.join("scripts/quality_metrics.py").to_str().unwrap(),
            "--self-test",
        ])
        .output()
        .expect("run quality metric regressions");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
