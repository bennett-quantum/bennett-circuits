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

fn word(text: &str, width: usize) -> Result<Vec<bool>> {
    use bennett_circuits::Error;
    if let Some(hex) = text.strip_prefix("0x") {
        let mut out = Vec::new();
        for c in hex.chars().rev() {
            let x = c
                .to_digit(16)
                .ok_or_else(|| Error("basis CLI: invalid hex integer".into()))?;
            out.extend((0..4).map(|i| x >> i & 1 != 0));
        }
        if out.iter().skip(width).any(|b| *b) {
            return Err(Error("basis CLI: integer out of range".into()));
        }
        out.resize(width, false);
        out.truncate(width);
        return Ok(out);
    }
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error(format!(
            "basis CLI: invalid unsigned integer {text:?}"
        )));
    }
    let mut digits = text.bytes().map(|b| b - b'0').collect::<Vec<_>>();
    let mut out = Vec::new();
    for _ in 0..width {
        let mut carry = 0;
        for d in &mut digits {
            let x = carry * 10 + *d;
            *d = x / 2;
            carry = x % 2;
        }
        out.push(carry != 0);
    }
    if digits.iter().any(|d| *d != 0) {
        return Err(Error("basis CLI: integer out of range".into()));
    }
    Ok(out)
}
fn decimal(bits: &[bool]) -> String {
    let mut digits = vec![0u8];
    for bit in bits.iter().rev() {
        let mut carry = u8::from(*bit);
        for d in &mut digits {
            let x = *d * 2 + carry;
            *d = x % 10;
            carry = x / 10;
        }
        if carry != 0 {
            digits.push(carry);
        }
    }
    digits.iter().rev().map(|d| (b'0' + d) as char).collect()
}
fn uint(bits: &[bool]) -> Value {
    let text = decimal(bits);
    if text
        .parse::<u64>()
        .is_ok_and(|n| n <= 9_007_199_254_740_991)
    {
        Value::Int(text)
    } else {
        Value::Str(text)
    }
}
fn hex_bits(text: &str) -> Result<Vec<bool>> {
    use bennett_circuits::Error;
    if !text.len().is_multiple_of(2) {
        return Err(Error("basis CLI: region hex needs whole bytes".into()));
    }
    let mut out = Vec::new();
    for pair in text.as_bytes().chunks(2) {
        let s = std::str::from_utf8(pair).map_err(|_| Error("basis CLI: invalid hex".into()))?;
        let v = u8::from_str_radix(s, 16).map_err(|_| Error("basis CLI: invalid hex".into()))?;
        out.extend((0..8).map(|i| v >> i & 1 != 0));
    }
    Ok(out)
}
#[derive(Default)]
struct BasisOptions {
    budget: Option<bennett_circuits::oracle::Budget>,
    args: Vec<bennett_circuits::oracle::Argument>,
    input: Option<String>,
    muts: Option<String>,
    ins: Option<String>,
    basis: Option<String>,
    memory: BTreeMap<String, Vec<bool>>,
    adjoint: bool,
}
fn basis_options(obj: &Object, args: &[String]) -> Result<BasisOptions> {
    use bennett_circuits::{Error, oracle::Argument};
    let mut out = BasisOptions::default();
    let mut options = args.iter();
    while let Some(option) = options.next() {
        if option == "--adjoint" {
            if out.adjoint {
                return Err(Error("basis CLI: repeated --adjoint".into()));
            }
            out.adjoint = true;
            continue;
        }
        let value = options
            .next()
            .ok_or_else(|| Error(format!("basis CLI: {option} requires a value")))?;
        let replace = |slot: &mut Option<String>| -> Result<()> {
            if slot.replace(value.clone()).is_some() {
                return Err(Error(format!("basis CLI: repeated {option}")));
            }
            Ok(())
        };
        match option.as_str() {
            "--input" => replace(&mut out.input)?,
            "--mut" => replace(&mut out.muts)?,
            "--in" => replace(&mut out.ins)?,
            "--basis" => replace(&mut out.basis)?,
            "--budget" => {
                if out.budget.is_some() {
                    return Err(Error("basis CLI: repeated budget".into()));
                }
                out.budget = Some(parse_budget(value, obj, None)?);
            }
            "--arg" => {
                out.args
                    .push(if let Some(t) = value.strip_prefix("token:") {
                        Argument::Token(parse_budget(t, obj, out.budget)?)
                    } else if let Some(t) = value.strip_prefix("f64:") {
                        let v = t
                            .parse::<f64>()
                            .map_err(|_| Error("basis CLI: invalid float".into()))?;
                        if !v.is_finite() {
                            return Err(Error("basis CLI: non-finite float (§3.1)".into()));
                        }
                        Argument::Float(v)
                    } else {
                        Argument::Integer(value.clone())
                    });
            }
            "--mem" => {
                let (n, hex) = value
                    .split_once('=')
                    .ok_or_else(|| Error("basis CLI: --mem requires name=hex".into()))?;
                if out.memory.insert(n.into(), hex_bits(hex)?).is_some() {
                    return Err(Error("basis CLI: repeated memory region".into()));
                }
            }
            _ => return Err(Error(format!("basis CLI: unknown option {option}"))),
        }
    }
    Ok(out)
}
fn values(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split(',').collect()
    }
}
fn run_basis(args: &[String]) -> Result<()> {
    use bennett_circuits::{Error, flatten::flatten, lower::lower_program_with_args, proof};
    let obj = load(&args[1])?;
    obj.check()?;
    let name = args[2].strip_prefix('@').unwrap_or(&args[2]);
    let options = basis_options(&obj, &args[3..])?;
    let f = obj
        .function(name)
        .ok_or_else(|| Error(format!("basis CLI: unknown @{name}")))?;
    if args[0] == "run-oracle" {
        if f.kind != bennett_circuits::object::FnKind::Oracle {
            return Err(Error("run-oracle: entry is not oracle".into()));
        }
        if options.muts.is_some()
            || options.ins.is_some()
            || options.basis.is_some()
            || options.adjoint
        {
            return Err(Error("run-oracle: action input options are invalid".into()));
        }
        let input = options
            .input
            .ok_or_else(|| Error("run-oracle: --input is required".into()))?;
        let input = values(&input);
        if input.len() != f.groups[0].len() {
            return Err(Error("run-oracle: input arity".into()));
        }
        let inputs = input
            .iter()
            .zip(&f.groups[0])
            .map(|(s, p)| word(s, p.ty.wire_width().unwrap() as usize))
            .collect::<Result<Vec<_>>>()?;
        let budget = options
            .budget
            .unwrap_or(parse_budget("1000000", &obj, None)?);
        let (payload, code, steps) =
            proof::oracle_basis(&obj, name, budget, &options.args, &inputs, &options.memory)?;
        println!(
            "{}",
            Value::Object(BTreeMap::from([
                ("code".into(), uint(&code)),
                ("payload".into(), uint(&payload)),
                (
                    "steps".into(),
                    if steps <= 9_007_199_254_740_991 {
                        Value::Int(steps.to_string())
                    } else {
                        Value::Str(steps.to_string())
                    }
                )
            ]))
            .to_jcs()
        );
    } else {
        if !f.is_action() {
            return Err(Error("run-action: entry is not action".into()));
        }
        if options.input.is_some() || options.budget.is_some() {
            return Err(Error(
                "run-action: oracle input/budget options are invalid".into(),
            ));
        }
        let mut program = lower_program_with_args(&obj, name, &options.args)?;
        if options.adjoint {
            let top = program
                .circuits
                .iter()
                .position(|c| c.name == program.top)
                .unwrap();
            program.circuits[top] = program.circuits[top].adjoint();
        }
        let c = program.top_circuit()?;
        let flat = flatten(&program)?;
        let input = if let Some(bits) = options.basis {
            if options.muts.is_some() || options.ins.is_some() || !options.memory.is_empty() {
                return Err(Error(
                    "run-action: mixed basis and tuple input forms".into(),
                ));
            }
            if bits.len() != flat.interface.len() || !bits.bytes().all(|b| b == b'0' || b == b'1') {
                return Err(Error("run-action: invalid basis bit string".into()));
            }
            bits.bytes().map(|b| b == b'1').collect()
        } else {
            let muts = options
                .muts
                .ok_or_else(|| Error("run-action: --basis or --mut required".into()))?;
            let muts = values(&muts);
            if muts.len() != c.mut_bundles.len() {
                return Err(Error("run-action: mut arity".into()));
            }
            let mut bits = Vec::new();
            for (s, p) in muts.iter().zip(&c.mut_bundles) {
                bits.extend(word(s, p.width as usize)?);
            }
            let ins = options.ins.as_deref().map(values);
            if ins
                .as_ref()
                .is_some_and(|is| is.len() != c.in_bundles.len())
            {
                return Err(Error("run-action: in arity".into()));
            }
            for (j, p) in c.in_bundles.iter().enumerate() {
                bits.extend(if let Some(ins) = &ins {
                    word(ins[j], p.width as usize)?
                } else {
                    vec![false; p.width as usize]
                });
            }
            for m in &c.mem {
                if let Some(data) = options.memory.get(&m.name) {
                    if data.len() != m.width() as usize {
                        return Err(Error("run-action: memory width mismatch".into()));
                    }
                    bits.extend(data);
                } else {
                    bits.resize(bits.len() + m.width() as usize, false);
                }
            }
            if options
                .memory
                .keys()
                .any(|n| !c.mem.iter().any(|m| m.name == *n))
            {
                return Err(Error("run-action: unknown memory region".into()));
            }
            bits
        };
        let output = proof::basis(&flat, &input)?;
        let mut cursor = 0;
        let mut bundles = |bs: &[bennett_circuits::ir::Bundle]| {
            bs.iter()
                .map(|b| {
                    let value = uint(&output[cursor..cursor + b.width as usize]);
                    cursor += b.width as usize;
                    value
                })
                .collect::<Vec<_>>()
        };
        let muts = bundles(&c.mut_bundles);
        let ins = bundles(&c.in_bundles);
        let mut memory = BTreeMap::new();
        for m in &c.mem {
            let cells = (0..m.num_cells())
                .map(|_| {
                    let value = uint(&output[cursor..cursor + m.cell_width as usize]);
                    cursor += m.cell_width as usize;
                    value
                })
                .collect();
            memory.insert(m.name.clone(), Value::Array(cells));
        }
        let bits = output
            .iter()
            .map(|b| if *b { '1' } else { '0' })
            .collect::<String>();
        let state = Value::Object(BTreeMap::from([(
            bits,
            Value::Array(vec![Value::Int("1".into()), Value::Int("0".into())]),
        )]));
        println!(
            "{}",
            Value::Object(BTreeMap::from([
                ("action".into(), Value::Str(name.into())),
                ("adjoint".into(), Value::Bool(options.adjoint)),
                ("wires".into(), Value::Int(output.len().to_string())),
                ("state".into(), state),
                ("mut".into(), Value::Array(muts)),
                ("in".into(), Value::Array(ins)),
                ("mem".into(), Value::Object(memory))
            ]))
            .to_jcs()
        );
    }
    Ok(())
}

fn run(args: &[String]) -> Result<()> {
    if matches!(args.first().map(String::as_str), Some("run-cc" | "reverse")) && args.len() >= 3 {
        let o = load(&args[1])?;
        o.check()?;
        let name = args[2].trim_start_matches('@');
        if o.function(name).is_some_and(|f| {
            matches!(
                f.kind,
                bennett_circuits::object::FnKind::Cc | bennett_circuits::object::FnKind::R
            )
        }) {
            println!(
                "{}",
                bennett_circuits::classical::run(&o, name, &args[3..], args[0] == "reverse")?
                    .to_jcs()
            );
            return Ok(());
        }
    }
    match args.first().map(String::as_str) {
        Some("run-oracle"|"run-action") if args.len()>=3 => run_basis(args),
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
