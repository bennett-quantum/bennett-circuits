//! Independent direct row semantics for diagnostic T6 checks. This evaluator
//! does not use the circuit builder. Unsupported rows fail before any verdict.
use crate::{
    Error, Result, bail,
    json::Value,
    object::{Object, Operand},
    oracle::Budget,
};
use std::collections::BTreeMap;
fn mask(w: u32) -> u128 {
    if w == 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}
fn signed(x: u128, w: u32) -> i128 {
    if w == 128 {
        x as i128
    } else {
        ((x << (128 - w)) as i128) >> (128 - w)
    }
}
fn to_int(bits: &[bool]) -> u128 {
    bits.iter()
        .enumerate()
        .fold(0, |x, (i, b)| x | (u128::from(*b) << i))
}
fn bits(x: u128, n: usize) -> Vec<bool> {
    (0..n).map(|i| x >> i & 1 != 0).collect()
}
fn direct(
    o: &Object,
    name: &str,
    budget: Budget,
    input: &[Vec<bool>],
    memory: &BTreeMap<String, Vec<bool>>,
) -> Result<(u128, u128, u64)> {
    let f = o
        .function(name)
        .ok_or_else(|| Error("step-check: missing oracle".into()))?;
    let mut ws = f
        .regs
        .iter()
        .map(|r| {
            let w = r.ty.wire_width().unwrap();
            if w > 128 {
                bail!("unimplemented: §14.1 direct diagnostic reg widths above 128 (milestone 2)")
            };
            Ok((r.name.clone(), (0u128, w)))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    for (p, bs) in f.groups[0].iter().zip(input) {
        if bs.len() > 128 {
            bail!("unimplemented: §14.1 direct diagnostic input widths above 128 (milestone 2)")
        };
        ws.insert(p.name.clone(), (to_int(bs), bs.len() as u32));
    }
    let mut bytes = BTreeMap::<u128, u8>::new();
    let mut ranges = vec![];
    let mut bases = BTreeMap::new();
    if let Some(layout) = &o.layout {
        if let Some(globals) = layout.get("globals").and_then(Value::as_object) {
            for (n, g) in globals {
                let base = g.get("base").and_then(Value::as_u64).unwrap() as u128;
                let data = o
                    .data_entry(g.get("image").and_then(Value::as_str).unwrap())
                    .unwrap();
                let size = g.get("size").and_then(Value::as_u64).unwrap() as u128;
                ranges.push((base, size));
                bases.insert(n.clone(), base);
                for (i, b) in data.bytes.iter().enumerate() {
                    bytes.insert(base + i as u128, *b);
                }
            }
        }
        for p in &f.groups[3] {
            let crate::object::Type::Region(a, w) = p.ty else {
                unreachable!()
            };
            if w != 8 {
                bail!("unimplemented: §14.1 direct diagnostic non-byte region cells (milestone 2)")
            };
            let base = layout
                .get("interface_regions")
                .and_then(|v| v.get(name))
                .and_then(|v| v.get(&p.name))
                .and_then(|v| v.get("base"))
                .and_then(Value::as_u64)
                .ok_or_else(|| Error("step-check: interface base".into()))?
                as u128;
            let n = 1usize
                .checked_shl(a)
                .ok_or_else(|| Error("step-check: region host limit".into()))?;
            ranges.push((base, n as u128));
            bases.insert(p.name.clone(), base);
            for i in 0..n {
                let v = memory
                    .get(&p.name)
                    .map(|bs| to_int(&bs[i * 8..i * 8 + 8]) as u8)
                    .unwrap_or(0);
                bytes.insert(base + i as u128, v);
            }
        }
    }
    let rows = f.blocks.iter().flat_map(|b| &b.instrs).collect::<Vec<_>>();
    let mut labels = BTreeMap::new();
    let mut cursor = 0;
    for b in &f.blocks {
        labels.insert(b.name.clone(), cursor);
        cursor += b.instrs.len();
    }
    let mut pc = 0usize;
    for t in 1..=budget.steps {
        let i = rows
            .get(pc)
            .ok_or_else(|| Error("step-check: pc outside body".into()))?;
        pc += 1;
        let a = &i.operands;
        let op = i.opcode.strip_prefix("lift.").unwrap_or(&i.opcode);
        let get = |v: &Operand| -> Result<(u128, u32)> {
            match v {
                Operand::Name(n) => ws
                    .get(n)
                    .copied()
                    .ok_or_else(|| Error(format!("step-check: unknown {n}"))),
                Operand::Int(n) => Ok((
                    n.parse()
                        .map_err(|_| Error("step-check: literal host limit".into()))?,
                    o.header.ptr_width,
                )),
                _ => bail!("step-check: integer operand"),
            }
        };
        let value = if let Some(w) = op.strip_prefix("const.i") {
            let w = w.parse::<u32>().unwrap();
            Some((get(&a[0])?.0 & mask(w), w))
        } else if let Some(w) = op
            .strip_prefix("zext.i")
            .or_else(|| op.strip_prefix("trunc.i"))
            .or_else(|| op.strip_prefix("sext.i"))
        {
            let w = w.parse::<u32>().unwrap();
            let (x, old) = get(&a[0])?;
            Some((
                (if op.starts_with("sext") {
                    signed(x, old) as u128
                } else {
                    x
                }) & mask(w),
                w,
            ))
        } else if let Some(pred) = op.strip_prefix("icmp.") {
            let (x, w) = get(&a[0])?;
            let y = get(&a[1])?.0;
            Some((
                u128::from(match pred {
                    "eq" => x == y,
                    "ne" => x != y,
                    "ult" => x < y,
                    "ule" => x <= y,
                    "slt" => signed(x, w) < signed(y, w),
                    "sle" => signed(x, w) <= signed(y, w),
                    _ => bail!("step-check: predicate"),
                }),
                1,
            ))
        } else if [
            "add", "sub", "mul", "and", "or", "xor", "shl", "lshr", "ashr", "udiv", "urem", "sdiv",
            "srem",
        ]
        .contains(&op)
        {
            let (x, w) = get(&a[0])?;
            let y = get(&a[1])?.0;
            if ["udiv", "urem", "sdiv", "srem"].contains(&op) && y == 0 {
                return Ok((0, 5, t));
            }
            if ["sdiv", "srem"].contains(&op) && x == 1u128 << (w - 1) && y == mask(w) {
                return Ok((0, 6, t));
            }
            let z = match op {
                "add" => x.wrapping_add(y),
                "sub" => x.wrapping_sub(y),
                "mul" => x.wrapping_mul(y),
                "and" => x & y,
                "or" => x | y,
                "xor" => x ^ y,
                "shl" => {
                    if y >= w as u128 {
                        0
                    } else {
                        x << (y as u32)
                    }
                }
                "lshr" => {
                    if y >= w as u128 {
                        0
                    } else {
                        x >> (y as u32)
                    }
                }
                "ashr" => {
                    if y >= w as u128 {
                        0
                    } else {
                        (signed(x, w) >> (y as u32)) as u128
                    }
                }
                "udiv" => x / y,
                "urem" => x % y,
                "sdiv" => (signed(x, w) / signed(y, w)) as u128,
                "srem" => (signed(x, w) % signed(y, w)) as u128,
                _ => unreachable!(),
            };
            Some((z & mask(w), w))
        } else {
            match op {
                "addr" => {
                    let Operand::Sym(n) = &a[0] else {
                        bail!("step-check: address symbol")
                    };
                    Some((
                        *bases
                            .get(n)
                            .ok_or_else(|| Error("step-check: address missing".into()))?,
                        o.header.ptr_width,
                    ))
                }
                s if s.starts_with("load.i") => {
                    let w = s[6..].parse::<u32>().unwrap();
                    let p = get(&a[0])?.0;
                    let n = w / 8;
                    if !ranges.iter().any(|(b, s)| {
                        p >= *b && p.checked_add(n as u128).is_some_and(|end| end <= b + s)
                    }) {
                        return Ok((0, 7, t));
                    }
                    let mut x = 0;
                    for j in 0..n {
                        x |= u128::from(*bytes.get(&(p + j as u128)).unwrap_or(&0)) << (8 * j);
                    }
                    Some((x, w))
                }
                "boundscheck" => {
                    let p = get(&a[0])?.0;
                    let n = get(&a[1])?.0;
                    if !ranges
                        .iter()
                        .any(|(b, s)| p >= *b && p.checked_add(n).is_some_and(|end| end <= b + s))
                    {
                        return Ok((0, 7, t));
                    }
                    None
                }
                "select" => Some(get(&a[if get(&a[0])?.0 != 0 { 1 } else { 2 }])?),
                "qjump" | "qbranch" => {
                    let j = if op == "qjump" {
                        0
                    } else if get(&a[0])?.0 != 0 {
                        1
                    } else {
                        2
                    };
                    let Operand::Label(n, _) = &a[j] else {
                        bail!("step-check: branch label")
                    };
                    pc = labels[n];
                    None
                }
                "retval" => return Ok((get(&a[0])?.0, 0, t)),
                "fault" => return Ok((0, get(&a[0])?.0, t)),
                "faultif" => {
                    if get(&a[0])?.0 != 0 {
                        return Ok((0, get(&a[1])?.0, t));
                    }
                    None
                }
                _ => bail!("unimplemented: §14.1 independent diagnostic row {op} (milestone 2)"),
            }
        };
        if let Some(v) = value {
            ws.insert(i.lhs[0].clone(), v);
        }
    }
    Ok((0, 1, budget.steps))
}
/// Enumerate the free input/memory domain and compare the independent row
/// evaluator to the literal circuit, including exact restoration checks.
pub fn check(
    o: &Object,
    name: &str,
    budget: Budget,
    fixed: &BTreeMap<String, Vec<bool>>,
) -> Result<Value> {
    let f = o
        .function(name)
        .ok_or_else(|| Error("step-check: missing oracle".into()))?;
    if !f.groups[4].is_empty() {
        bail!("unimplemented: §14.1 step-check classical arguments (milestone 2)")
    }
    let p = crate::oracle::specialise(o, name, budget, &[])?;
    let flat = crate::flatten::flatten(&p)?;
    let c = p.top_circuit()?;
    let input_width = f.groups[0]
        .iter()
        .map(|p| p.ty.wire_width().unwrap() as usize)
        .sum::<usize>();
    let mem_width = c
        .mem
        .iter()
        .filter(|m| !fixed.contains_key(&m.name))
        .map(|m| m.width() as usize)
        .sum::<usize>();
    let total = input_width + mem_width;
    if total > 16 {
        bail!("unimplemented: §17.13 sampled step-check above 16 free bits (milestone 2)")
    }
    if fixed.keys().any(|n| !c.mem.iter().any(|m| m.name == *n)) {
        bail!("step-check: unknown fixed region")
    }
    for x in 0..1usize << total {
        let mut column = 0;
        let mut inputs = vec![];
        for p in &f.groups[0] {
            let w = p.ty.wire_width().unwrap() as usize;
            inputs.push(bits((x >> column) as u128, w));
            column += w;
        }
        let mut memory = fixed.clone();
        for m in &c.mem {
            let w = m.width() as usize;
            if let Some(bs) = fixed.get(&m.name) {
                if bs.len() != w {
                    bail!("step-check: fixed memory width")
                }
            } else {
                memory.insert(m.name.clone(), bits((x >> column) as u128, w));
                column += w;
            }
        }
        let expected = direct(o, name, budget, &inputs, &memory)?;
        let n = f.groups[1][0].ty.wire_width().unwrap() as usize;
        let fw = f.groups[2][0].ty.wire_width().unwrap() as usize;
        if n > 128 || fw > 128 {
            bail!("unimplemented: §14.1 direct diagnostic outcome widths above 128 (milestone 2)")
        }
        let mut initial = vec![false; n + fw];
        for bs in &inputs {
            initial.extend(bs);
        }
        for m in &c.mem {
            initial.extend(&memory[&m.name]);
        }
        let result = crate::proof::basis(&flat, &initial)?;
        if result[n + fw..] != initial[n + fw..]
            || (to_int(&result[..n]), to_int(&result[n..n + fw])) != (expected.0, expected.1)
        {
            bail!(
                "step-check: direct/circuit mismatch at domain element {x}: expected {expected:?}, actual {result:?} (§14.1; T6)"
            )
        }
    }
    Ok(Value::Object(BTreeMap::from([
        ("status".into(), Value::Str("ok".into())),
        ("cases".into(), Value::Int((1usize << total).to_string())),
        (
            "input_and_memory_bits".into(),
            Value::Int(total.to_string()),
        ),
        ("mode".into(), Value::Str("exhaustive".into())),
        (
            "fixed_regions".into(),
            Value::Array(fixed.keys().cloned().map(Value::Str).collect()),
        ),
    ])))
}
