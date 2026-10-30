//! Differential oracle via CLI only. Exhaustive basis columns; exact sparse
//! simulation of exported QASM, phase-sensitive binary64 display comparison.
#[path = "support/exact.rs"]
mod exact;
#[path = "support/sim.rs"]
mod sim;
use bennett_circuits::{lower::lower_program, object::Object, qasm};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}
fn corpus() -> Vec<PathBuf> {
    let workspace = root().parent().unwrap().to_path_buf();
    let mut paths =
        std::fs::read_dir(workspace.join("bennett-isa-spec/reference-interpreter/tests/fixtures"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "bisa"))
            .collect::<Vec<_>>();
    paths.sort();
    let spec = std::fs::read_to_string(workspace.join("bennett-isa-spec/spec/Bennett-ISA-Spec.md"))
        .unwrap();
    let dir = root().join(".tmp/differential");
    std::fs::create_dir_all(&dir).unwrap();
    let mut examples = 0;
    for block in spec
        .split("```")
        .filter(|b| b.trim_start().starts_with("bennett-object "))
    {
        let path = dir.join(format!("example-{examples}.bisa"));
        std::fs::write(&path, block.trim_start()).unwrap();
        paths.push(path);
        examples += 1;
    }
    assert_eq!(examples, 2, "ISA 0.8 spec examples changed; audit corpus");
    paths.push(synthetic_corpus());
    paths
}
// The reference JSON reports finite binary64 numbers. The production JSON
// codec deliberately admits only integers for hashed metadata; this tiny
// independent reader handles the run-action `state` object alone.
fn state(stdout: &str) -> BTreeMap<String, (f64, f64)> {
    let start = stdout.find("\"state\":").expect("reference state");
    let tail = &stdout[start + 8..];
    let open = tail.find('{').unwrap();
    let tail = &tail[open + 1..];
    let tail = &tail[..tail.find('}').unwrap()];
    let mut rest = tail.trim();
    let mut out = BTreeMap::new();
    while !rest.is_empty() {
        rest = rest.strip_prefix('"').unwrap();
        let end = rest.find('"').unwrap();
        let key = rest[..end].to_string();
        rest = rest[end + 1..]
            .trim_start()
            .strip_prefix(':')
            .unwrap()
            .trim_start()
            .strip_prefix('[')
            .unwrap();
        let end = rest.find(']').unwrap();
        let pair = rest[..end]
            .split(',')
            .map(|x| x.trim().parse::<f64>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(pair.len(), 2);
        assert!(out.insert(key, (pair[0], pair[1])).is_none());
        rest = rest[end + 1..].trim_start();
        if let Some(r) = rest.strip_prefix(',') {
            rest = r.trim_start();
        } else {
            assert!(rest.is_empty());
        }
    }
    out
}
#[test]
fn every_action_every_basis_matches_run_action() {
    let binary = root()
        .parent()
        .unwrap()
        .join("bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref");
    assert!(
        binary.is_file(),
        "build the reference binary separately: {}",
        binary.display()
    );
    let (mut actions, mut cases) = (0, 0);
    for path in corpus() {
        let obj = Object::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        obj.check().unwrap();
        for f in obj.functions.iter().filter(|f| f.is_action()) {
            let program = lower_program(&obj, &f.name).unwrap();
            let text = qasm::export(&program).unwrap();
            assert_eq!(qasm::parse(&text).unwrap(), program);
            let flat = qasm::parse_flat(&text).unwrap();
            let placed = bennett_circuits::placement::place_action(&obj, &f.name, &[]).unwrap();
            let width = flat.interface.len();
            assert!(
                width < usize::BITS as usize,
                "exhaustive basis domain exceeds host enumeration capacity"
            );
            let mut action_cases = 0;
            for input in 0..(1usize << width) {
                let bits = (0..width)
                    .map(|b| if input >> b & 1 != 0 { '1' } else { '0' })
                    .collect::<String>();
                let out = Command::new(&binary)
                    .args([
                        "run-action",
                        path.to_str().unwrap(),
                        &f.name,
                        "--basis",
                        &bits,
                    ])
                    .env_remove("DISPLAY")
                    .env_remove("WAYLAND_DISPLAY")
                    .env_remove("DBUS_SESSION_BUS_ADDRESS")
                    .env_remove("BROWSER")
                    .env("TMPDIR", root().join(".tmp"))
                    .output()
                    .unwrap();
                assert!(
                    out.status.success(),
                    "{} @{} {bits}: reference failed: {}",
                    path.display(),
                    f.name,
                    String::from_utf8_lossy(&out.stderr)
                );
                let reference = state(&String::from_utf8(out.stdout).unwrap());
                let ours = std::panic::catch_unwind(|| sim::basis(&flat, &bits)).unwrap_or_else(|failure| {
                    panic!("DIVERGENCE {} @{} input={bits}: exported simulation failed {failure:?}; reference={reference:?}", path.display(), f.name)
                });
                assert_eq!(
                    sim::basis(&placed.flat, &bits),
                    ours,
                    "PLACEMENT DIVERGENCE {} @{} input={bits}",
                    path.display(),
                    f.name
                );
                let display = ours
                    .iter()
                    .map(|(b, a)| (b.clone(), a.value()))
                    .collect::<BTreeMap<_, _>>();
                assert_eq!(
                    display.keys().collect::<Vec<_>>(),
                    reference.keys().collect::<Vec<_>>(),
                    "DIVERGENCE {} @{} input={bits}: exported={display:?} reference={reference:?} (exact support differs)",
                    path.display(),
                    f.name
                );
                for (b, (re, im)) in &display {
                    let (r, i) = reference[b];
                    assert!(
                        (re - r).abs() <= 2e-12 && (im - i).abs() <= 2e-12,
                        "DIVERGENCE {} @{} input={bits}: exported={display:?} reference={reference:?} (§4.4 includes global phase)",
                        path.display(),
                        f.name
                    );
                }
                cases += 1;
                action_cases += 1;
            }
            eprintln!(
                "AGREE {} @{}: {action_cases} basis inputs",
                path.file_name().unwrap().to_string_lossy(),
                f.name
            );
            actions += 1;
        }
    }
    assert!(actions >= 19, "expected fixture plus supplemental actions");
    eprintln!(
        "HARNESS: {actions} actions, {cases} basis comparisons, {cases} agree, 0 diverge; phase-sensitive display tolerance 2e-12; exact support/workspace checks; placed circuits agree on all {cases} tuples"
    );
}

fn synthetic_corpus() -> PathBuf {
    use bennett_circuits::{json::Value as V, object::derived_signature};
    let header = "bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=32\n\nsection symbols\n";
    let functions = concat!(
        "action @gates (t:wire[3] mut) (p:wire in, r:wire in) () () ()\n^entry():\n",
        "x t[0]\nh t[0]\ns t[0]\nsdg t[1]\nt t[1]\ntdg t[0]\n",
        "cx p t[0]\nccx p r t[1]\nmcx (p, r) t[2]\n",
        "cz t[0] t[1]\nswap t[1] t[2]\nrx 0.37 t[0]\nry -0.23 t[1]\nrz 0.81 t[2]\n",
        "qpred p\ncx r t[0]\nh t[1]\nqunpred p\n",
        "perm_action @allocs (t:wire[2] mut) (p:wire[2] in) () () ()\n^entry():\n",
        "d = qalloc any 2\nqxor p d\nqxor d t\nqxor p d\nqfree d\ne = qcopy p\nqexch e t\nqexch e t\nquncopy p e\n",
        "perm_action @memops (t:wire[2] mut) (a:wire in) (m:region[1,2] mut) () ()\n^entry():\n",
        "d = qload m a\nqxor d t\nqunload m a d\nqstore m a t\nqstore m a t\nqexchm m a t\n",
        "perm_action @lexical (t:wire[2] mut) (a:wire in) () () ()\nregs {  } bounds {0}\n^entry():\n",
        "d = qload r a\nqxor d t\nqunload r a d\nqloop 0 {\nqregion r 1 2\n}\n",
        "action @leaf (t:wire mut) () () () ()\n^entry():\n",
        "d = qalloc any 1\ncx t d\ns d\ncx t d\nqfree d\nh t\nry 0.125 t\n",
        "action @parent (t:wire mut) (p:wire in) () () ()\n^entry():\n",
        "ctl p action_call @leaf (t) () () ()\nadj action_call @leaf (t) () () ()\n"
    );
    let initial = format!("{header}{{}}\n\nsection functions\n{functions}");
    let obj = Object::parse(&initial).unwrap();
    let mut symbols = BTreeMap::new();
    for (i, f) in obj.functions.iter().enumerate() {
        symbols.insert(
            i.to_string(),
            V::Object(BTreeMap::from([
                ("interface_hash".into(), V::Str("0".repeat(64))),
                ("kind".into(), V::Str(f.kind.symbol_kind().into())),
                ("name".into(), V::Str(f.name.clone())),
                ("signature".into(), derived_signature(f).unwrap()),
            ])),
        );
    }
    let provisional = format!(
        "{header}{}\n\nsection functions\n{functions}",
        V::Object(symbols.clone()).to_jcs()
    );
    let hashes = Object::parse(&provisional).unwrap().hash().unwrap().symbols;
    for entry in symbols.values_mut() {
        let V::Object(fields) = entry else {
            unreachable!()
        };
        let name = fields["name"].as_str().unwrap();
        let hash = hashes[name].interface_hash.clone();
        fields.insert("interface_hash".into(), V::Str(hash));
    }
    let text = format!(
        "{header}{}\n\nsection functions\n{functions}",
        V::Object(symbols).to_jcs()
    );
    let path = root().join(".tmp/differential/synthetic.bisa");
    let layout = r#"{"A_tab":0,"H":4,"S_cells":0,"globals":{},"heap_base":1,"heap_limit_max":0,"interface_regions":{},"null_size":1,"ptr_width":32}"#;
    std::fs::write(&path, format!("{text}\nsection layout\n{layout}\n")).unwrap();
    path
}
