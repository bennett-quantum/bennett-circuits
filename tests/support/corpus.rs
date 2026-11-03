//! Versioned selection over the shared conformance corpus (bennett-circuits-dzo, -l2t).
//!
//! The corpus in `../bennett-isa-spec/conformance/cases` holds cases for every published ISA
//! version (0.8 and 0.9 side by side). A case's version is the `isa=` its object header
//! declares. This consumer implements exactly the `isa` of `SPEC_VERSIONS.toml`:
//! - `Implemented`: the case runs with its full published expectations.
//! - `Foreign(v)`: the expectations are for a `v` consumer and are NOT asserted. The case still
//!   runs, and this consumer must refuse it at load (exit 1, empty stdout, `unsupported isa=v`,
//!   a cited section), which is what the 0.8 contract requires of an incompatible version
//!   (preamble Versioning; §1; §9.1). The 0.8 version-rejection case F-version (isa=0.7) is
//!   checked by exactly this assertion. Every foreign case is printed with its version.
//! - `Unversioned`: no object, or a first line that is not a header; nothing declares a version,
//!   so the case runs with its full expectations.
//!
//! Selection is never by name list. The corpus' own citation label (`spec` starting `ISA<v>`)
//! must agree with the header, or classification panics.
use bennett_circuits::json::Value;

#[derive(Debug, PartialEq, Eq)]
pub enum Declared {
    Implemented,
    Foreign(String),
    Unversioned,
}

/// The `isa` pin of SPEC_VERSIONS.toml, which must equal the version the loader implements.
pub fn implemented_isa() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/SPEC_VERSIONS.toml");
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let pins: Vec<&str> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("isa = "))
        .collect();
    assert_eq!(
        pins.len(),
        1,
        "{path}: expected exactly one `isa = \"..\"` line"
    );
    let v = pins[0]
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or_else(|| panic!("{path}: isa pin is not a quoted string: {}", pins[0]));
    assert_eq!(
        v,
        bennett_circuits::object::ISA_VERSION,
        "SPEC_VERSIONS.toml isa pin disagrees with the loader's ISA_VERSION"
    );
    v.to_string()
}

/// Classify a case by the `isa=` its object header declares (`object_text` is the object file,
/// `None` when the case has no object).
pub fn declared(case: &Value, object_text: Option<&str>, implemented: &str) -> Declared {
    let id = case.get("id").and_then(Value::as_str).unwrap_or("<no id>");
    let label = case
        .get("spec")
        .and_then(Value::as_str)
        .and_then(|s| s.strip_prefix("ISA"))
        .map(|s| {
            s.trim_start()
                .split(|c: char| c.is_whitespace() || c == '§')
                .next()
                .unwrap()
                .to_string()
        });
    let header = object_text
        .and_then(|t| t.lines().next())
        .and_then(|l| l.strip_prefix("bennett-object "))
        .and_then(|l| l.split(' ').next())
        .and_then(|t| t.strip_prefix("isa="));
    match (header, label) {
        (Some(h), Some(l)) => assert_eq!(
            h, l,
            "{id}: corpus label ISA{l} disagrees with object header isa={h}"
        ),
        (None, Some(l)) => {
            panic!("{id}: labelled ISA{l} but no object header declares a version; classify it")
        }
        _ => {}
    }
    match header {
        None => Declared::Unversioned,
        Some(h) if h == implemented => Declared::Implemented,
        Some(h) => Declared::Foreign(h.to_string()),
    }
}

/// The 0.8 obligation for a foreign case: refusal at load, naming the declared version.
pub fn assert_refused(id: &str, version: &str, exit: Option<i32>, stdout: &str, stderr: &str) {
    let needle = format!("unsupported isa={version}");
    assert!(
        exit == Some(1) && stdout.is_empty() && stderr.contains(&needle) && stderr.contains('§'),
        "{id}: declares isa={version}, which this consumer does not implement, so it must be \
         refused at load (exit 1, empty stdout, `{needle}`, a cited §); got exit={exit:?} \
         stdout={stdout:?} stderr={stderr:?}"
    );
}

/// Per-suite counts, printed always; a suite with nothing run at an implemented or
/// unversioned level fails.
pub struct Tally {
    suite: String,
    selected: Vec<String>,
    unversioned: Vec<String>,
    refused: Vec<String>,
}

impl Tally {
    pub fn new(suite: &str) -> Self {
        Tally {
            suite: suite.into(),
            selected: vec![],
            unversioned: vec![],
            refused: vec![],
        }
    }

    pub fn record(&mut self, id: &str, d: &Declared) {
        match d {
            Declared::Implemented => self.selected.push(id.into()),
            Declared::Unversioned => self.unversioned.push(id.into()),
            Declared::Foreign(v) => self.refused.push(format!("{id} (isa={v})")),
        }
    }

    /// Returns (selected, unversioned, refused-by-version).
    pub fn finish(self) -> (usize, usize, usize) {
        eprintln!(
            "CORPUS suite {}: {} run at the implemented isa, {} unversioned run, {} refused by \
             declared version (expectations deselected: isa not in SPEC_VERSIONS.toml): [{}]",
            self.suite,
            self.selected.len(),
            self.unversioned.len(),
            self.refused.len(),
            self.refused.join(", ")
        );
        assert!(
            !self.selected.is_empty() || !self.unversioned.is_empty(),
            "suite {}: no case ran with its expectations ({} refused by version)",
            self.suite,
            self.refused.len()
        );
        (
            self.selected.len(),
            self.unversioned.len(),
            self.refused.len(),
        )
    }
}
