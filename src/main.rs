//! `bennett-circuits` CLI. Data commands print one JSON object to stdout
//! and exit 0; diagnostics go to stderr with exit status 1 (mirroring the
//! reference interpreter's CLI conventions).

use std::collections::BTreeMap;
use std::process::ExitCode;

use bennett_circuits::Result;
use bennett_circuits::json::Value;
use bennett_circuits::object::{CheckReport, Object};

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
        (
            "object_hash".to_string(),
            Value::Str(report.object_hash.clone()),
        ),
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

fn parse_budget(
    text: &str,
    obj: &Object,
    defaults: Option<bennett_circuits::oracle::Budget>,
) -> Result<bennett_circuits::oracle::Budget> {
    use bennett_circuits::{Error, oracle::Budget};
    let values = text
        .split(',')
        .map(|s| {
            s.parse::<u64>()
                .map_err(|_| Error(format!("export-qasm: invalid budget component {s:?}")))
        })
        .collect::<Result<Vec<_>>>()?;
    let default = if let Some(default) = defaults {
        default
    } else if let Some(layout) = &obj.layout {
        let heap = layout
            .get("heap_limit_max")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error("export-qasm: missing layout heap limit".into()))?;
        let sc = layout
            .get("S_cells")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error("export-qasm: missing stack exponent".into()))?;
        let stack = 1u64
            .checked_shl(
                sc.try_into()
                    .map_err(|_| Error("host limit: stack exponent".into()))?,
            )
            .ok_or_else(|| Error("host limit: stack size".into()))?;
        Budget {
            steps: 0,
            heap,
            stack,
            depth: 64,
        }
    } else {
        Budget {
            steps: 0,
            heap: 0,
            stack: 0,
            depth: 0,
        }
    };
    match values.as_slice() {
        [steps] => Ok(Budget {
            steps: *steps,
            ..default
        }),
        [steps, heap, stack, depth] => Ok(Budget {
            steps: *steps,
            heap: *heap,
            stack: *stack,
            depth: *depth,
        }),
        _ => Err(Error(
            "export-qasm: budget requires B or B,heap,stack,depth".into(),
        )),
    }
}

fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("check") if args.len() == 2 => {
            let report = load(&args[1])?.check()?;
            println!("{}", report_json(&report).to_jcs());
            Ok(())
        }
        Some("export-qasm") if args.len() >= 3 => {
            let obj = load(&args[1])?;
            obj.check()?;
            let mut budget=None;let mut arguments=Vec::new();let mut options=args[3..].iter();
            while let Some(option)=options.next(){let value=options.next().ok_or_else(||bennett_circuits::Error(format!("export-qasm: {option} requires a value")))?;match option.as_str(){
                "--budget"=>{if budget.is_some(){return Err(bennett_circuits::Error("export-qasm: repeated --budget".into()))}budget=Some(parse_budget(value,&obj,None)?);},
                "--arg"=>{use bennett_circuits::oracle::Argument;let argument=if let Some(text)=value.strip_prefix("token:"){Argument::Token(parse_budget(text,&obj,budget)?)}else if let Some(text)=value.strip_prefix("f64:"){let v=text.parse::<f64>().map_err(|_|bennett_circuits::Error("export-qasm: invalid f64 argument".into()))?;if !v.is_finite(){return Err(bennett_circuits::Error("export-qasm: non-finite f64 argument (§3.1)".into()))}Argument::Float(v)}else if let Some(text)=value.strip_prefix("0x"){Argument::Integer(u128::from_str_radix(text,16).map_err(|_|bennett_circuits::Error("export-qasm: hexadecimal classical argument exceeds 128 bits".into()))?.to_string())}else{Argument::Integer(value.clone())};arguments.push(argument);},
                _=>return Err(bennett_circuits::Error(format!("export-qasm: unknown option {option}")))
            }}
            let name=args[2].strip_prefix('@').unwrap_or(&args[2]);
            let function=obj.function(name).ok_or_else(||bennett_circuits::Error(format!("export-qasm: unknown region @{name}")))?;
            let p=if function.kind==bennett_circuits::object::FnKind::Oracle {
                let budget=budget.ok_or_else(||bennett_circuits::Error("export-qasm: oracle entry requires --budget B[,heap,stack,depth]".into()))?;
                bennett_circuits::oracle::specialise(&obj,name,budget,&arguments)?
            }else {if budget.is_some(){return Err(bennett_circuits::Error("export-qasm: --budget requires an oracle entry; pass action tokens with --arg token:B,heap,stack,depth".into()))}bennett_circuits::lower::lower_program_with_args(&obj,name,&arguments)?};
            print!("{}", bennett_circuits::qasm::export(&p)?);
            Ok(())
        }
        Some("hash") if args.len() == 2 => {
            let report = load(&args[1])?.hash()?;
            println!("{}", report_json(&report).to_jcs());
            Ok(())
        }
        _ => Err(bennett_circuits::Error(
            "usage: bennett-circuits check <object> | hash <object> | export-qasm <object> <region> [--budget B[,heap,stack,depth]] [--arg value]..."
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
