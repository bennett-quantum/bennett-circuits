//! `bennett-circuits` CLI. Data commands print one JSON object to stdout
//! and exit 0; diagnostics go to stderr with exit status 1 (mirroring the
//! reference interpreter's CLI conventions).

use std::collections::BTreeMap;
use std::process::ExitCode;

use bennett_circuits::json::Value;
use bennett_circuits::object::{CheckReport, Object};
use bennett_circuits::Result;

fn report_json(report: &CheckReport) -> Value {
    let symbols: BTreeMap<String, Value> = report
        .symbols
        .iter()
        .map(|(name, h)| {
            let entry: BTreeMap<String, Value> = [
                (
                    "closure_hash".to_string(),
                    h.closure_hash.clone().map_or(Value::Null, Value::Str),
                ),
                (
                    "interface_hash".to_string(),
                    Value::Str(h.interface_hash.clone()),
                ),
            ]
            .into_iter()
            .collect();
            (name.clone(), Value::Object(entry))
        })
        .collect();
    let top: BTreeMap<String, Value> = [
        ("object_hash".to_string(), Value::Str(report.object_hash.clone())),
        ("status".to_string(), Value::Str(report.status.clone())),
        ("symbols".to_string(), Value::Object(symbols)),
        (
            "unresolved_imports".to_string(),
            Value::Array(
                report
                    .unresolved_imports
                    .iter()
                    .cloned()
                    .map(Value::Str)
                    .collect(),
            ),
        ),
    ]
    .into_iter()
    .collect();
    Value::Object(top)
}

fn load(path: &str) -> Result<Object> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| bennett_circuits::Error(format!("cannot read {path}: {e}")))?;
    Object::parse(&text)
}

fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("check") if args.len() == 2 => {
            let report = load(&args[1])?.check()?;
            println!("{}", report_json(&report).to_jcs());
            Ok(())
        }
        Some("export-qasm") if args.len() == 3 => {
            let obj = load(&args[1])?;
            obj.check()?;
            let p = bennett_circuits::lower::lower_program(&obj, &args[2])?;
            print!("{}", bennett_circuits::qasm::export(&p)?);
            Ok(())
        }
        Some("hash") if args.len() == 2 => {
            let report = load(&args[1])?.hash()?;
            println!("{}", report_json(&report).to_jcs());
            Ok(())
        }
        _ => Err(bennett_circuits::Error(
            "usage: bennett-circuits check <object> | hash <object> | export-qasm <object> <action>"
                .into(),
        )),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bennett-circuits: {e}");
            ExitCode::FAILURE
        }
    }
}
