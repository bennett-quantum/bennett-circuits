//! ISA 0.8 producer validation. Independent of stored identity and execution.
use crate::{
    Error, Result, bail,
    flatten::{Flat, FlatGate},
    ir::*,
    object::*,
};
use std::collections::{BTreeMap, BTreeSet};
fn err(s: impl Into<String>) -> Error {
    Error(s.into())
}
fn walk<'a>(is: &'a [Instr], out: &mut Vec<&'a Instr>) {
    for i in is {
        out.push(i);
        for child in &i.children {
            walk(child, out)
        }
    }
}
fn all(f: &Function) -> Vec<&Instr> {
    let mut out = Vec::new();
    for b in &f.blocks {
        walk(&b.instrs, &mut out)
    }
    out
}
fn width(t: &Type, pw: u32) -> Option<u32> {
    match t {
        Type::Int(w) | Type::Reg(w) | Type::Wire(w) => Some(*w),
        Type::Ptr => Some(pw),
        _ => None,
    }
}
fn opname(o: &Operand) -> Result<&str> {
    match o {
        Operand::Name(n) => Ok(n),
        _ => bail!("checker: expected location, got {o:?} (§3.3)"),
    }
}
fn int(o: &Operand) -> Result<u64> {
    match o {
        Operand::Int(n) => n
            .parse()
            .map_err(|_| err("checker: host limit: literal count exceeds u64")),
        _ => bail!("checker: expected count literal (§3.3)"),
    }
}
/// Imported functions have no body to inspect, but their complete operand
/// contract is published in §9.1. Checking an unlinked object must use it.
fn callee_contract(o: &Object, n: &str) -> Result<Function> {
    if let Some(f) = o.function(n) {
        return Ok(f.clone());
    }
    let s = &o
        .imports
        .iter()
        .find(|i| i.name == n)
        .ok_or_else(|| err(format!("checker: unknown callee @{n} (§9.1)")))?
        .signature;
    let kind = match s.get("kind").and_then(|v| v.as_str()) {
        Some("cc") => FnKind::Cc,
        Some("r") => FnKind::R,
        Some("lproc") => FnKind::Lproc,
        Some("oracle") => FnKind::Oracle,
        Some("action") => FnKind::Action,
        Some("perm_action") => FnKind::PermAction,
        _ => bail!("checker: data is not callable (§9.1)"),
    };
    let mut groups: [Vec<Param>; 5] = Default::default();
    let mut add = |group: usize, key: &str, wire: bool| -> Result<()> {
        for (i, v) in s
            .get(key)
            .and_then(|v| v.as_array())
            .ok_or_else(|| err("checker: import signature list (§9.1)"))?
            .iter()
            .enumerate()
        {
            let ty = if wire {
                Type::Reg(
                    v.as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| err("host limit: import width"))?,
                )
            } else {
                crate::object::parse_type(
                    v.as_str()
                        .ok_or_else(|| err("checker: classical type (§9.1)"))?,
                )?
            };
            groups[group].push(Param {
                name: format!("arg{i}"),
                ty,
                role: None,
            });
        }
        Ok(())
    };
    match kind {
        FnKind::Cc | FnKind::R => {
            add(0, "params", false)?;
            add(1, "results", false)?;
        }
        FnKind::Lproc => {
            add(0, "params", true)?;
            let w = s
                .get("result")
                .and_then(|v| v.as_u64())
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| err("host limit: imported result width"))?;
            groups[1].push(Param {
                name: "result".into(),
                ty: Type::Reg(w),
                role: None,
            });
        }
        FnKind::Oracle | FnKind::Action | FnKind::PermAction => {
            let oracle = kind == FnKind::Oracle;
            if oracle {
                add(0, "in", true)?;
                add(4, "args", false)?;
            } else {
                add(0, "mut", true)?;
                add(1, "in", true)?;
                add(3, "args", false)?;
            }
            // Quantum interfaces use wires; lifted call sites separately bind
            // those published widths to locations (§3.3).
            for ps in &mut groups[..if oracle { 1 } else { 2 }] {
                for p in ps {
                    if let Type::Reg(w) = p.ty {
                        p.ty = Type::Wire(w);
                    }
                }
            }
            if oracle {
                for (group, key) in [(1, "tgt"), (2, "fault")] {
                    let w = s
                        .get(key)
                        .and_then(|v| v.as_u64())
                        .and_then(|w| u32::try_from(w).ok())
                        .ok_or_else(|| err("host limit: import wire width"))?;
                    groups[group].push(Param {
                        name: key.into(),
                        ty: Type::Wire(w),
                        role: None,
                    });
                }
            }
            let mem_group = if oracle { 3 } else { 2 };
            for (i, mem) in s
                .get("mem")
                .and_then(|v| v.as_array())
                .ok_or_else(|| err("checker: import mem (§9.1)"))?
                .iter()
                .enumerate()
            {
                let dim = |k| {
                    mem.get(k)
                        .and_then(|v| v.as_u64())
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| err("host limit: imported memory geometry"))
                };
                groups[mem_group].push(Param {
                    name: format!("mem{i}"),
                    ty: Type::Region(dim("A")?, dim("W")?),
                    role: mem.get("role").and_then(|v| v.as_str()).map(str::to_owned),
                });
            }
        }
    }
    Ok(Function {
        kind,
        name: n.into(),
        groups,
        regs: vec![],
        bounds: vec![],
        blocks: vec![],
        canonical_bytes: String::new(),
    })
}
fn schema(f: &Function, o: &Object) -> Result<()> {
    let mut names = BTreeMap::new();
    for p in f.groups.iter().flatten() {
        if names.insert(p.name.clone(), p.ty.clone()).is_some() {
            bail!("checker: @{} duplicate name {} (§3.3)", f.name, p.name)
        }
    }
    for p in &f.regs {
        if names.insert(p.name.clone(), p.ty.clone()).is_some() {
            bail!("checker: duplicate location {} (§3.3)", p.name)
        }
    }
    let lifted = matches!(f.kind, FnKind::Oracle | FnKind::Lproc);
    if !lifted {
        return Ok(());
    }
    let mut writes = BTreeSet::new();
    for r in &f.regs {
        writes.insert(r.name.clone());
    }
    if f.kind == FnKind::Lproc {
        writes.extend(
            f.groups[0]
                .iter()
                .chain(&f.groups[1])
                .map(|p| p.name.clone()),
        );
    }
    let get = |a: &Operand| -> Result<u32> {
        let n = opname(a)?;
        let t = names
            .get(n)
            .ok_or_else(|| err(format!("checker: @{} unknown read {n} (§3.7)", f.name)))?;
        width(t, o.header.ptr_width)
            .ok_or_else(|| err(format!("checker: {n} is not a bit vector (§3.3)")))
    };
    let labels = f
        .blocks
        .iter()
        .map(|b| b.name.clone())
        .collect::<BTreeSet<_>>();
    if labels.len() != f.blocks.len() {
        bail!("checker: duplicate label (§3.6)")
    }
    let fault_width = if f.kind == FnKind::Oracle {
        f.groups[2][0].ty.wire_width().unwrap()
    } else {
        // shared lprocs must fit every containing oracle; literal check below
        o.functions
            .iter()
            .filter(|f| f.kind == FnKind::Oracle)
            .map(|f| f.groups[2][0].ty.wire_width().unwrap())
            .min()
            .unwrap_or(4)
    };
    if fault_width < 4 {
        bail!("checker: fault width F must be >=4 (§5.4)")
    }
    for b in &f.blocks {
        if !b.params.is_empty() {
            bail!("checker: lifted block parameters are not locations (§3.7)")
        }
        if b.instrs.is_empty() {
            bail!("checker: empty block without terminator (§3.6)")
        }
        for (j, i) in b.instrs.iter().enumerate() {
            let op = i.opcode.strip_prefix("lift.").unwrap_or(&i.opcode);
            let a = &i.operands;
            if f.kind == FnKind::Oracle {
                for operand in a {
                    let ns = match operand {
                        Operand::Name(n) => vec![n.as_str()],
                        Operand::List(ns) => ns.iter().map(String::as_str).collect(),
                        _ => Vec::new(),
                    };
                    if ns
                        .iter()
                        .any(|n| f.groups[1].iter().chain(&f.groups[2]).any(|p| p.name == *n))
                    {
                        bail!(
                            "checker: oracle target/fault is not a readable pseudo-location (§3.7)"
                        )
                    }
                }
            }
            let term = matches!(
                op,
                "qjump" | "qbranch" | "qjumpr" | "ret" | "retval" | "fault"
            );
            if term != (j + 1 == b.instrs.len()) {
                bail!(
                    "checker: @{} block ^{} terminator position (§3.6)",
                    f.name,
                    b.name
                )
            }
            for n in &i.lhs {
                if !writes.contains(n) {
                    bail!("checker: @{0} writes non-location {n} (§3.7)", f.name)
                }
            }
            let dest = if let Some(n) = i.lhs.first() {
                width(&names[n], o.header.ptr_width).unwrap()
            } else {
                0
            };
            let (argc, results) = match op {
                "mov" | "label" | "addr" | "alloc" | "call" | "select" => (
                    match op {
                        "call" => 2,
                        "select" => 3,
                        _ => 1,
                    },
                    1,
                ),
                "parmov" => (1, i.lhs.len()),
                "qjump" | "qjumpr" | "fault" | "ret" | "retval" | "free" => (1, 0),
                "qbranch" => (3, 0),
                "boundscheck" | "faultif" => (2, 0),
                "oracle_call" | "oracle_uncall" => (7, 0),
                "perm_call" => (5, 0),
                _ if op.starts_with("const.i") => (1, 1),
                _ if op.starts_with("load.i") => (1, 1),
                _ if op.starts_with("store.i") => (2, 0),
                _ if op.starts_with("zext.i")
                    || op.starts_with("sext.i")
                    || op.starts_with("trunc.i") =>
                {
                    (1, 1)
                }
                _ if op.starts_with("icmp.") => (2, 1),
                "add" | "sub" | "mul" | "and" | "or" | "xor" | "shl" | "lshr" | "ashr" | "udiv"
                | "urem" | "sdiv" | "srem" => (2, 1),
                _ => bail!(
                    "checker: {} illegal in lifted body (§4.8; §4.2 exact-permutation only)",
                    i.opcode
                ),
            };
            if a.len() != argc || i.lhs.len() != results || !i.children.is_empty() {
                bail!("checker: {} arity/result schema (§3.3)", i.opcode)
            }
            for a in a {
                if let Operand::Label(n, None) = a
                    && !labels.contains(n)
                {
                    bail!("checker: label ^{n} is outside function (§3.6)")
                }
            }
            let equal = |x: u32, y: u32| -> Result<()> {
                if x != y {
                    bail!("checker: {} width mismatch {x} != {y} (§3.3)", i.opcode)
                }
                Ok(())
            };
            match op {
                "add" | "sub" | "mul" | "and" | "or" | "xor" | "shl" | "lshr" | "ashr" | "udiv"
                | "urem" | "sdiv" | "srem" => {
                    equal(get(&a[0])?, get(&a[1])?)?;
                    equal(dest, get(&a[0])?)?;
                }
                "mov" => equal(dest, get(&a[0])?)?,
                "select" => {
                    equal(get(&a[0])?, 1)?;
                    equal(get(&a[1])?, get(&a[2])?)?;
                    equal(dest, get(&a[1])?)?;
                }
                "parmov" => {
                    let Operand::List(src) = &a[0] else {
                        bail!("checker: parmov list (§3.3)")
                    };
                    if src.len() != i.lhs.len()
                        || i.lhs.iter().collect::<BTreeSet<_>>().len() != i.lhs.len()
                    {
                        bail!("checker: parmov destinations not distinct or arity (§3.3)")
                    }
                    for (n, s) in i.lhs.iter().zip(src) {
                        equal(
                            width(&names[n], o.header.ptr_width).unwrap(),
                            get(&Operand::Name(s.clone()))?,
                        )?;
                    }
                }
                "qbranch" | "faultif" => {
                    equal(get(&a[0])?, 1)?;
                }
                "qjumpr" => {
                    let n = f.blocks.iter().map(|b| b.instrs.len()).sum::<usize>();
                    let w = get(&a[0])?;
                    if w < usize::BITS && 1usize << w <= n {
                        bail!("checker: qjumpr index width (§3.3)")
                    }
                }
                "ret" => {
                    if f.kind != FnKind::Lproc {
                        bail!("checker: lift.ret outside lproc (§4.2)")
                    }
                    equal(get(&a[0])?, f.groups[1][0].ty.wire_width().unwrap())?;
                }
                "retval" => {
                    if f.kind != FnKind::Oracle {
                        bail!("checker: lift.retval in lproc (§4.2)")
                    }
                    equal(get(&a[0])?, f.groups[1][0].ty.wire_width().unwrap())?;
                }
                "addr" => {
                    equal(dest, o.header.ptr_width)?;
                    let Operand::Sym(n) = &a[0] else {
                        bail!("checker: lift.addr symbol (§4.2)")
                    };
                    let global = o
                        .layout
                        .as_ref()
                        .and_then(|v| v.get("globals"))
                        .and_then(|v| v.get(n))
                        .is_some();
                    if !global
                        && !(f.kind == FnKind::Oracle && f.groups[3].iter().any(|p| p.name == *n))
                    {
                        bail!("checker: lift.addr @{n} not visible (§4.2)")
                    }
                }
                "alloc" | "boundscheck" | "free" => {
                    if op == "alloc" {
                        equal(dest, o.header.ptr_width)?
                    } else {
                        equal(get(&a[0])?, o.header.ptr_width)?
                    }
                    if op != "free" {
                        let n = if op == "alloc" { &a[0] } else { &a[1] };
                        if !matches!(n, Operand::Int(_)) {
                            get(n)?;
                        } else {
                            let value = int(n)?;
                            if o.header.ptr_width < 64 && value >= 1u64 << o.header.ptr_width {
                                bail!("checker: allocation length range (§3.3)")
                            }
                        }
                    }
                }
                "call" => {
                    let Operand::Sym(n) = &a[0] else {
                        bail!("checker: lift.call symbol (§3.3)")
                    };
                    let cf = callee_contract(o, n)?;
                    if cf.kind != FnKind::Lproc {
                        bail!("checker: lift.call non-lproc (§3.7)")
                    }
                    let Operand::List(actuals) = &a[1] else {
                        bail!("checker: call list (§3.3)")
                    };
                    if actuals.len() != cf.groups[0].len() {
                        bail!("checker: call arity (§3.3)")
                    }
                    for (n, p) in actuals.iter().zip(&cf.groups[0]) {
                        equal(get(&Operand::Name(n.clone()))?, p.ty.wire_width().unwrap())?;
                    }
                    equal(dest, cf.groups[1][0].ty.wire_width().unwrap())?;
                }
                "oracle_call" | "oracle_uncall" | "perm_call" => {
                    lifted_call(o, f, i, &names)?;
                }
                _ if op.starts_with("icmp.") => {
                    if !["eq", "ne", "ult", "ule", "slt", "sle"].contains(&&op[5..]) {
                        bail!("checker: icmp predicate (§3.3)")
                    }
                    equal(dest, 1)?;
                    equal(get(&a[0])?, get(&a[1])?)?;
                }
                _ if op.starts_with("const.i") => {
                    let suffix = op[7..]
                        .parse::<u32>()
                        .map_err(|_| err("checker: const suffix (§3.3)"))?;
                    equal(dest, suffix)?;
                    if !matches!(&a[0], Operand::Int(_)) {
                        bail!("checker: const literal (§3.3)")
                    }
                }
                _ if op.starts_with("zext.i")
                    || op.starts_with("sext.i")
                    || op.starts_with("trunc.i") =>
                {
                    let w = get(&a[0])?;
                    let narrow = op.starts_with("trunc");
                    if (narrow && dest >= w) || (!narrow && dest <= w) {
                        bail!("checker: cast must strictly widen/narrow (§3.3)")
                    }
                }
                _ if op.starts_with("load.i") || op.starts_with("store.i") => {
                    let n = op
                        .split_once(".i")
                        .unwrap()
                        .1
                        .parse::<u32>()
                        .map_err(|_| err("checker: memory width (§3.3)"))?;
                    if n == 0 || n > 128 || n % 8 != 0 {
                        bail!("checker: byte load/store width (§3.3)")
                    }
                    equal(get(&a[0])?, o.header.ptr_width)?;
                    if op.starts_with("load") {
                        equal(dest, n)?
                    } else {
                        equal(get(&a[1])?, n)?
                    }
                }
                _ => {}
            }
            if matches!(op, "fault" | "faultif") {
                let c = a.last().unwrap();
                let Operand::Int(c) = c else {
                    bail!("checker: fault code literal (§3.3)")
                };
                if c.starts_with('-') || decimal_bits(c) > fault_width {
                    bail!("checker: fault code must fit F (§3.3)")
                }
            }
            if o.layout.is_none()
                && (matches!(
                    op,
                    "addr" | "boundscheck" | "alloc" | "free" | "call" | "ret"
                ) || op.starts_with("load.")
                    || op.starts_with("store."))
            {
                bail!("checker: memory row in memoryless object (§7.2)")
            }
        }
    }
    matched_cleanup(o, f)?;
    Ok(())
}
fn decimal_bits(s: &str) -> u32 {
    let mut ds = s.bytes().map(|b| b - b'0').collect::<Vec<_>>();
    let mut n = 0;
    while ds.iter().any(|d| *d != 0) {
        let mut carry = 0;
        for d in &mut ds {
            let a = carry * 10 + *d;
            *d = a / 2;
            carry = a % 2;
        }
        n += 1;
    }
    n
}
fn lifted_call(o: &Object, f: &Function, i: &Instr, names: &BTreeMap<String, Type>) -> Result<()> {
    let a = &i.operands;
    let Operand::Sym(n) = &a[0] else {
        bail!("checker: call symbol (§3.3)")
    };
    let sig = o
        .symbol(n)
        .map(|s| &s.signature)
        .or_else(|| {
            o.imports
                .iter()
                .find(|s| s.name == *n)
                .map(|s| &s.signature)
        })
        .ok_or_else(|| err("checker: unresolved call (§3.7)"))?;
    let oracle = i.opcode != "lift.perm_call";
    let location = |n: &str| {
        f.regs.iter().any(|p| p.name == n)
            || (f.kind == FnKind::Lproc
                && f.groups[0].iter().chain(&f.groups[1]).any(|p| p.name == n))
    };
    if oracle && matches!(f.kind, FnKind::Oracle | FnKind::Lproc) {
        for index in [3, 4] {
            if !location(opname(&a[index])?) {
                bail!("checker: lifted oracle writes non-location (§3.7)")
            }
        }
    } else if !oracle
        && let Operand::List(ns) = &a[1]
        && ns.iter().any(|n| !location(n))
    {
        bail!("checker: lifted permutation call writes pseudo-location (§3.7)")
    }

    if sig.get("kind").and_then(|v| v.as_str())
        != Some(if oracle { "oracle" } else { "perm_action" })
    {
        bail!("checker: wrong callee kind (§4.2)")
    }
    let check_words = |actuals: &Operand, key: &str| -> Result<()> {
        let Operand::List(actuals) = actuals else {
            bail!("checker: call list (§3.3)")
        };
        let widths = sig
            .get(key)
            .and_then(|v| v.as_array())
            .ok_or_else(|| err("checker: callee signature (§9.1)"))?;
        if actuals.len() != widths.len() {
            bail!("checker: call arity (§3.3)")
        }
        for (n, w) in actuals.iter().zip(widths) {
            if names
                .get(n)
                .and_then(|t| width(t, o.header.ptr_width))
                .map(u64::from)
                != w.as_u64()
            {
                bail!("checker: call width mismatch (§3.3)")
            }
        }
        Ok(())
    };
    let (mem_index, arg_index) = if oracle {
        check_words(&a[2], "in")?;
        for (index, key) in [(3, "tgt"), (4, "fault")] {
            let name = opname(&a[index])?;
            if names
                .get(name)
                .and_then(|t| width(t, o.header.ptr_width))
                .map(u64::from)
                != sig.get(key).and_then(|v| v.as_u64())
            {
                bail!("checker: oracle target/fault width (§3.3)")
            }
        }
        let tk = opname(&a[1])?;
        if names.get(tk) != Some(&Type::Token) {
            bail!("checker: oracle token must be classical (§4.2)")
        }
        (5, 6)
    } else {
        check_words(&a[1], "mut")?;
        check_words(&a[2], "in")?;
        (3, 4)
    };
    let Operand::List(mems) = &a[mem_index] else {
        bail!("checker: memory list (§3.3)")
    };
    let mem_sig = sig.get("mem").and_then(|v| v.as_array()).unwrap();
    if mems.len() != mem_sig.len() {
        bail!("checker: memory arity (§3.3)")
    }
    for (n, p) in mems.iter().zip(mem_sig) {
        let caller_mem = &f.groups[if f.is_action() { 2 } else { 3 }];
        let Some(Type::Region(aa, w)) = names.get(n) else {
            bail!("checker: unknown memory region (§3.3)")
        };
        if p.get("A").and_then(|v| v.as_u64()) != Some(*aa as u64)
            || p.get("W").and_then(|v| v.as_u64()) != Some(*w as u64)
        {
            bail!("checker: memory geometry (§3.3)")
        }
        if p.get("role").and_then(|v| v.as_str()) == Some("mut")
            && caller_mem
                .iter()
                .any(|p| p.name == *n && p.role.as_deref() == Some("in"))
        {
            bail!("checker: in region passed to mut slot (§3.3)")
        }
        if !caller_mem.iter().any(|p| p.name == *n) {
            bail!("checker: lifted memory must be mapped caller region (§4.2)")
        }
    }
    let Operand::List(args) = &a[arg_index] else {
        bail!("checker: args list (§3.3)")
    };
    let formal = sig.get("args").and_then(|v| v.as_array()).unwrap();
    if args.len() != formal.len() {
        bail!("checker: classical arity (§3.3)")
    }
    for (n, t) in args.iter().zip(formal) {
        if names
            .get(n)
            .and_then(|t| t.signature_string().ok())
            .as_deref()
            != t.as_str()
        {
            bail!("checker: classical argument type (§3.3)")
        }
    }
    Ok(())
}
/// May-write summary for lifted calls. Recursive lproc SCCs reach a finite
/// fixed point because the effect is the single "may write memory" bit.
fn writes_memory(o: &Object, name: &str, visiting: &mut BTreeSet<String>) -> bool {
    if !visiting.insert(name.into()) {
        return false;
    }
    let Some(f) = o.function(name) else {
        return true;
    };
    all(f).iter().any(|i| {
        i.opcode.starts_with("lift.store.")
            || matches!(i.opcode.as_str(), "lift.alloc" | "lift.free")
            || i.opcode == "lift.call"
                && i.operands
                    .iter()
                    .any(|a| matches!(a, Operand::Sym(n) if writes_memory(o, n, visiting)))
            || i.opcode == "lift.perm_call"
                && match i.operands.first() {
                    Some(Operand::Sym(n)) => o
                        .symbol(n)
                        .map(|s| &s.signature)
                        .or_else(|| {
                            o.imports
                                .iter()
                                .find(|s| s.name == *n)
                                .map(|s| &s.signature)
                        })
                        .and_then(|s| s.get("mem"))
                        .and_then(|v| v.as_array())
                        .is_none_or(|ms| {
                            ms.iter()
                                .any(|m| m.get("role").and_then(|v| v.as_str()) == Some("mut"))
                        }),
                    _ => true,
                }
    })
}
fn matched_cleanup(o: &Object, f: &Function) -> Result<()> {
    if !all(f).iter().any(|i| i.opcode == "lift.oracle_uncall") {
        return Ok(());
    }
    let rows = f.blocks.iter().flat_map(|b| &b.instrs).collect::<Vec<_>>();
    let mut labels = BTreeMap::new();
    let mut offset = 0;
    for b in &f.blocks {
        labels.insert(b.name.as_str(), offset);
        offset += b.instrs.len();
    }
    // Identical operands denote a stack of dynamically distinct invocations.
    // Calls push, uncalls pop, and the remainder is retained for prefix reversal
    // (§5.5). Keep the minimum available stack depth on every incoming path.
    // Updates are monotone (push, pop, or invalidation), so componentwise minima
    // suffice for safety; no unbounded stack or iteration cutoff is required.
    let calls = rows
        .iter()
        .enumerate()
        .filter(|(_, i)| i.opcode == "lift.oracle_call")
        .map(|(pc, _)| pc)
        .collect::<Vec<_>>();
    let mut keys: Vec<usize> = Vec::new();
    for pc in calls {
        if !keys.iter().any(|k| rows[*k].operands == rows[pc].operands) {
            keys.push(pc);
        }
    }
    type Labels = BTreeMap<String, usize>;
    let mut work = vec![(0usize, vec![0usize; keys.len()], Labels::new())];
    let mut incoming: BTreeMap<(usize, Labels), Vec<usize>> = BTreeMap::new();
    while let Some((pc, mut pending, mut values)) = work.pop() {
        if pc >= rows.len() {
            continue;
        }
        let state_key = (pc, values.clone());
        if let Some(previous) = incoming.get_mut(&state_key) {
            let mut changed = false;
            for (old, new) in previous.iter_mut().zip(&pending) {
                if new < old {
                    *old = *new;
                    changed = true;
                }
            }
            if !changed {
                continue;
            }
            pending.clone_from(previous);
        } else {
            incoming.insert(state_key, pending.clone());
        }
        let i = rows[pc];
        let delivery = matches!(i.opcode.as_str(), "lift.oracle_call" | "lift.oracle_uncall");
        let own_key = delivery
            .then(|| keys.iter().position(|k| rows[*k].operands == i.operands))
            .flatten();
        if i.opcode == "lift.oracle_uncall" {
            let k =
                own_key.ok_or_else(|| err("checker: unmatched lifted oracle cleanup (§5.5)"))?;
            if pending[k] == 0 {
                bail!(
                    "checker: unmatched lifted oracle cleanup or changed read/delivery version on an incoming path (§5.5)"
                );
            }
            pending[k] -= 1;
        }
        let mut written = i.lhs.iter().cloned().collect::<BTreeSet<_>>();
        if matches!(i.opcode.as_str(), "lift.oracle_call" | "lift.oracle_uncall") {
            for k in [3, 4] {
                written.insert(opname(&i.operands[k])?.into());
            }
        }
        if i.opcode == "lift.perm_call"
            && let Operand::List(ns) = &i.operands[1]
        {
            written.extend(ns.iter().cloned());
        }
        let memory = i.opcode.starts_with("lift.store.")
            || i.opcode == "lift.call"
                && match &i.operands[0] {
                    Operand::Sym(n) => writes_memory(o, n, &mut BTreeSet::new()),
                    _ => true,
                }
            || i.opcode == "lift.perm_call" && writes_memory(o, &f.name, &mut BTreeSet::new());
        for (k, p) in keys.iter().enumerate() {
            let call = rows[*p];
            let Operand::List(ins) = &call.operands[2] else {
                unreachable!()
            };
            let Operand::List(mems) = &call.operands[5] else {
                unreachable!()
            };
            // Oracle deliveries XOR into target/fault, so other deliveries
            // commute when their inputs stay current. They are successor
            // versions, unlike an ordinary overwrite of a delivered outcome.
            let outputs_changed = !delivery
                && [3, 4]
                    .iter()
                    .any(|j| opname(&call.operands[*j]).is_ok_and(|n| written.contains(n)));
            if ins.iter().any(|n| written.contains(n))
                || memory && !mems.is_empty()
                || outputs_changed
            {
                pending[k] = 0;
            }
        }
        let read_label = |a: &Operand| match a {
            Operand::Name(n) => values.get(n).copied(),
            Operand::Label(n, _) => labels.get(n.as_str()).copied(),
            _ => None,
        };
        let outputs = match i.opcode.as_str() {
            "lift.label" | "lift.mov" => vec![read_label(&i.operands[0])],
            "lift.parmov" => match &i.operands[0] {
                Operand::List(ns) => ns.iter().map(|n| values.get(n).copied()).collect(),
                _ => unreachable!(),
            },
            "lift.select" => {
                let a = read_label(&i.operands[1]);
                let b = read_label(&i.operands[2]);
                vec![if a == b { a } else { None }]
            }
            _ => vec![None; i.lhs.len()],
        };
        for name in &written {
            values.remove(name);
        }
        for (name, value) in i.lhs.iter().zip(outputs) {
            if let Some(value) = value {
                values.insert(name.clone(), value);
            }
        }
        if i.opcode == "lift.oracle_call" {
            let k = own_key.expect("call has its own signature key");
            pending[k] = pending[k]
                .checked_add(1)
                .ok_or_else(|| err("host limit: invocation depth exceeds usize (§5.5)"))?;
        }
        let mut successors = Vec::new();
        match i.opcode.as_str() {
            "qjump" | "qbranch" => {
                for a in &i.operands {
                    if let Operand::Label(n, _) = a {
                        successors.push(labels[n.as_str()]);
                    }
                }
            }
            // §4.2 bounds qjumpr to the containing function, even when its
            // oracle index space also contains lprocs. Outside indices fault.
            "qjumpr" => {
                if let Some(destination) = values.get(opname(&i.operands[0])?) {
                    successors.push(*destination);
                } else {
                    successors.extend(0..rows.len());
                }
            }
            "lift.retval" | "lift.ret" | "fault" => {}
            _ => successors.push(pc + 1),
        }
        for next in successors {
            work.push((next, pending.clone(), values.clone()));
        }
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Expr {
    Zero,
    Atom(usize),
    Xor(Vec<usize>),
    And(Vec<usize>),
    Quantum(String, Vec<usize>, usize),
}
struct Algebra {
    nodes: Vec<Expr>,
    index: BTreeMap<Expr, usize>,
}
impl Algebra {
    fn new() -> Self {
        Self {
            nodes: vec![Expr::Zero],
            index: BTreeMap::from([(Expr::Zero, 0)]),
        }
    }
    fn intern(&mut self, e: Expr) -> usize {
        if let Some(id) = self.index.get(&e) {
            return *id;
        }
        let id = self.nodes.len();
        self.nodes.push(e.clone());
        self.index.insert(e, id);
        id
    }
    fn quantum_nodes(&self, id: usize, out: &mut BTreeSet<usize>) {
        match &self.nodes[id] {
            Expr::Quantum(..) => {
                out.insert(id);
            }
            Expr::Xor(xs) | Expr::And(xs) => {
                for x in xs {
                    self.quantum_nodes(*x, out);
                }
            }
            _ => {}
        }
    }
    fn xor(&mut self, a: usize, b: usize) -> usize {
        let mut terms = BTreeSet::new();
        for x in [a, b] {
            let xs = match &self.nodes[x] {
                Expr::Zero => Vec::new(),
                Expr::Xor(xs) => xs.clone(),
                _ => vec![x],
            };
            for x in xs {
                if !terms.insert(x) {
                    terms.remove(&x);
                }
            }
        }
        match terms.len() {
            0 => 0,
            1 => *terms.first().unwrap(),
            _ => self.intern(Expr::Xor(terms.into_iter().collect())),
        }
    }
    fn and(&mut self, terms: &[usize]) -> usize {
        let mut polynomial = BTreeSet::from([BTreeSet::<usize>::new()]);
        for term in terms {
            let summands = match &self.nodes[*term] {
                Expr::Zero => Vec::new(),
                Expr::Xor(xs) => xs.clone(),
                _ => vec![*term],
            };
            let mut product = BTreeSet::new();
            for left in &polynomial {
                for right in &summands {
                    let mut monomial = left.clone();
                    match &self.nodes[*right] {
                        Expr::And(xs) => monomial.extend(xs),
                        _ => {
                            monomial.insert(*right);
                        }
                    }
                    if !product.insert(monomial.clone()) {
                        product.remove(&monomial);
                    }
                }
            }
            polynomial = product;
        }
        let mut value = 0;
        for monomial in polynomial {
            let term = if monomial.len() == 1 {
                *monomial.first().unwrap()
            } else {
                self.intern(Expr::And(monomial.into_iter().collect()))
            };
            value = self.xor(value, term);
        }
        value
    }
}
fn adj_name(g: BaseGate) -> String {
    format!("{:?}", g.adjoint())
}
fn abstract_action(o: &Object, f: &Function, p: &Program) -> Result<()> {
    if let Some(binding) = o.schedule.as_ref().and_then(|s| s.get("bindings"))
        && binding
            .get("region")
            .and_then(|v| v.as_str())
            .is_some_and(|n| n.trim_start_matches('@') == f.name)
        && (binding
            .get("inherited_controls")
            .and_then(|v| v.as_u64())
            .is_some_and(|n| n > 0)
            || binding.get("variant").and_then(|v| v.as_str()) == Some("controlled")
            || p.circuits
                .iter()
                .flat_map(|c| &c.body)
                .any(|n| matches!(n,Node::Call{controls,..}if !controls.is_empty())))
    {
        bail!(
            "unimplemented: §6.4 inherited-control cleanup schedule key validation; generated-operation matching requires unpublished expansion library b-isa-spec-8r4; decision bennett-circuits-yz8 (milestone 2)"
        );
    }
    validate_pairs(p)?;
    let (flat, nonperm_calls) = crate::flatten::flatten_with_events(p)?;
    let top = p.top_circuit()?;
    let mut offset = 0;
    let mut readonly = BTreeSet::new();
    for b in &top.mut_bundles {
        offset += b.width as usize;
    }
    for b in &top.in_bundles {
        readonly.extend(offset..offset + b.width as usize);
        offset += b.width as usize;
    }
    for m in &top.mem {
        if m.role == Role::In {
            readonly.extend(offset..offset + m.width() as usize);
        }
        offset += m.width() as usize;
    }
    let ws = flat.workspace.iter().copied().collect::<BTreeSet<_>>();
    let mut borrowed = BTreeSet::new();
    let mut start = flat.interface.len();
    for w in &top.workspace {
        if w.kind == WsKind::Borrowed {
            borrowed.extend(start..start + w.width as usize);
        }
        start += w.width as usize;
    }
    borrow_frames(o, f, top, &flat, &borrowed)?;
    let reduced = cancel_adjoints(&flat.gates);
    let (_, _, closed) = action_values(
        flat.num_qubits,
        &ws,
        &borrowed,
        top.perm,
        &flat.gates,
        &nonperm_calls,
    )?;
    let (values, initial, _) =
        action_values(flat.num_qubits, &ws, &borrowed, top.perm, &reduced, &[])?;
    for q in ws {
        if values[q] != initial[q] {
            bail!("checker: workspace {q} has no exact matched cleanup (§6.2)")
        }
        if closed.contains(&q) {
            bail!("checker: workspace {q} cleanup window closed (§6.2)")
        }
    }
    Ok(())
}
/// Cancel exact inverse gates across disjoint supports only. In particular,
/// a use of an `other` value as a control blocks cancellation of its producer.
fn cancel_adjoints(gates: &[FlatGate]) -> Vec<FlatGate> {
    let mut reduced: Vec<FlatGate> = Vec::new();
    for g in gates {
        let touched = g
            .targets
            .iter()
            .chain(g.controls.iter().map(|(q, _)| q))
            .copied()
            .collect::<BTreeSet<_>>();
        let prior = reduced.iter().rposition(|h| {
            h.targets
                .iter()
                .chain(h.controls.iter().map(|(q, _)| q))
                .any(|q| touched.contains(q))
        });
        if let Some(index) = prior
            && reduced[index].gate == g.gate.adjoint()
            && reduced[index].controls == g.controls
            && reduced[index].targets == g.targets
        {
            reduced.remove(index);
        } else {
            reduced.push(g.clone());
        }
    }
    reduced
}
fn action_values(
    num_qubits: usize,
    ws: &BTreeSet<usize>,
    borrowed: &BTreeSet<usize>,
    perm_action: bool,
    gates: &[FlatGate],
    nonperm_calls: &[(usize, Vec<usize>)],
) -> Result<(Vec<usize>, Vec<usize>, BTreeSet<usize>)> {
    let mut algebra = Algebra::new();
    let one = algebra.intern(Expr::And(vec![]));
    let mut values = (0..num_qubits)
        .map(|i| {
            if ws.contains(&i) && !borrowed.contains(&i) {
                0
            } else {
                algebra.intern(Expr::Atom(i))
            }
        })
        .collect::<Vec<_>>();
    let initial = values.clone();
    let mut deps = vec![BTreeSet::new(); num_qubits];
    let mut closed = BTreeSet::new();
    let mut dirty_quantum = BTreeSet::new();
    let mut other = vec![false; num_qubits];
    let mut input_other = BTreeMap::new();
    for (ordinal, g) in gates.iter().enumerate() {
        for (_, touched) in nonperm_calls.iter().filter(|(n, _)| *n == ordinal) {
            for w in ws {
                if values[*w] != 0 && touched.iter().any(|q| deps[*w].contains(q)) {
                    closed.insert(*w);
                }
            }
        }
        let diagonal = matches!(
            g.gate,
            BaseGate::S
                | BaseGate::Sdg
                | BaseGate::T
                | BaseGate::Tdg
                | BaseGate::Cz
                | BaseGate::Rz(_)
        );
        let mut distinct = BTreeSet::new();
        for q in g.targets.iter().chain(g.controls.iter().map(|(q, _)| q)) {
            if !distinct.insert(*q) {
                bail!("checker: overlapping operands/effective control (§3.3)")
            }
        }

        if perm_action && !matches!(g.gate, BaseGate::X | BaseGate::Swap) {
            bail!("checker: permutation action contains non-permutation gate (§5.3)")
        }
        if diagonal {
            // A phase on a computational-basis expression is retained in
            // the declared interface semantics (T5). On an `other` value it
            // changes subsequent interference and must remain in its exact
            // inverse history.
            let touched = g
                .targets
                .iter()
                .chain(g.controls.iter().map(|(q, _)| q))
                .copied()
                .collect::<Vec<_>>();
            for q in touched {
                algebra.quantum_nodes(values[q], &mut dirty_quantum);
            }
            continue;
        }
        let perm = matches!(g.gate, BaseGate::X | BaseGate::Swap);
        if !perm {
            for q in &g.targets {
                for w in ws {
                    if deps[*w].contains(q) && values[*w] != 0 {
                        closed.insert(*w);
                    }
                }
            }
        }
        let controls = g
            .controls
            .iter()
            .map(|(q, p)| {
                if *p {
                    values[*q]
                } else {
                    algebra.xor(values[*q], one)
                }
            })
            .collect::<Vec<_>>();
        let condition = algebra.and(&controls);
        let mut dependency = BTreeSet::new();
        for (q, _) in &g.controls {
            dependency.insert(*q);
            dependency.extend(deps[*q].clone());
        }
        for (q, _) in &g.controls {
            algebra.quantum_nodes(values[*q], &mut dirty_quantum);
        }
        match g.gate {
            BaseGate::X => {
                let q = g.targets[0];
                values[q] = algebra.xor(values[q], condition);
                deps[q].extend(dependency);
                if values[q] == 0 {
                    deps[q].clear();
                }
            }
            BaseGate::Swap => {
                let (a, b) = (g.targets[0], g.targets[1]);
                if g.controls.is_empty() {
                    values.swap(a, b);
                    other.swap(a, b);
                } else {
                    let delta = algebra.xor(values[a], values[b]);
                    let delta = algebra.and(&[condition, delta]);
                    values[a] = algebra.xor(values[a], delta);
                    values[b] = algebra.xor(values[b], delta);
                    let both = other[a] || other[b];
                    other[a] = both;
                    other[b] = both;
                }
                dependency.extend(deps[a].clone());
                dependency.extend(deps[b].clone());
                deps[a] = dependency.clone();
                deps[b] = dependency;
            }
            _ => {
                for q in &g.targets {
                    let old = values[*q];
                    let prior = algebra.nodes[old].clone();
                    values[*q] = match prior {
                        Expr::Quantum(name, cs, input)
                            if name == adj_name(g.gate)
                                && cs == controls
                                && other[*q]
                                && !dirty_quantum.contains(&old) =>
                        {
                            other[*q] = input_other.get(&old).copied().unwrap_or(false);
                            input
                        }
                        _ => {
                            let node = algebra.intern(Expr::Quantum(
                                format!("{:?}", g.gate),
                                controls.clone(),
                                old,
                            ));
                            input_other.insert(node, other[*q]);
                            other[*q] = true;
                            node
                        }
                    };
                    deps[*q].extend(dependency.clone());
                    if values[*q] == 0 {
                        deps[*q].clear();
                    }
                }
            }
        }
    }
    Ok((values, initial, closed))
}
fn borrow_frames(
    o: &Object,
    f: &Function,
    top: &Circuit,
    flat: &Flat,
    b: &BTreeSet<usize>,
) -> Result<()> {
    let lemma = translation_frame(flat, b);
    if lemma.is_ok() {
        return Ok(());
    }
    if o.certificate.is_none() {
        return lemma;
    }
    crate::borrow::verify(o, f, top, flat, b)
}
fn translation_frame(flat: &Flat, b: &BTreeSet<usize>) -> Result<()> {
    if b.is_empty() {
        return Ok(());
    }
    let touches = |g: &FlatGate| {
        g.targets
            .iter()
            .chain(g.controls.iter().map(|(q, _)| q))
            .any(|q| b.contains(q))
    };
    let mut start = 0;
    for end in 0..=flat.gates.len() {
        if end < flat.gates.len() && matches!(flat.gates[end].gate, BaseGate::X | BaseGate::Swap) {
            continue;
        }
        if end < flat.gates.len() && touches(&flat.gates[end]) {
            bail!("checker: non-permutation instruction touches borrowed wires (§5.6)");
        }
        translation_component(&flat.gates[start..end], b)?;
        start = end + 1;
    }
    Ok(())
}
pub(crate) fn translation_component(gates: &[FlatGate], b: &BTreeSet<usize>) -> Result<()> {
    let touches = |g: &FlatGate| {
        g.targets
            .iter()
            .chain(g.controls.iter().map(|(q, _)| q))
            .any(|q| b.contains(q))
    };
    let first = gates.iter().position(touches);
    let Some(first) = first else { return Ok(()) };
    let last = gates.iter().rposition(touches).unwrap();
    let frame = &gates[first..=last];
    if frame.iter().any(|g| g.gate != BaseGate::X) {
        bail!("checker: borrow frame must use X-family flips (§5.6)")
    }
    let vlen = frame
        .iter()
        .take_while(|g| b.contains(&g.targets[0]))
        .count();
    if vlen == 0 {
        bail!("checker: borrow frame V target outside B (§5.6)")
    }
    let v = &frame[..vlen];
    let x = v
        .iter()
        .flat_map(|g| g.controls.iter().map(|(q, _)| *q))
        .collect::<BTreeSet<_>>();
    if !x.is_disjoint(b) {
        bail!("checker: borrow frame V controls intersect B (§5.6)")
    }
    let mut valid = false;
    for wlen in 1..frame.len() {
        if 2 * (vlen + wlen) != frame.len() {
            continue;
        }
        let w = &frame[vlen..vlen + wlen];
        let inv = &frame[vlen + wlen..2 * vlen + wlen];
        let w2 = &frame[2 * vlen + wlen..];
        if !v.iter().rev().eq(inv.iter()) || w != w2 {
            continue;
        }
        let controls = w
            .iter()
            .flat_map(|g| g.controls.iter().map(|(q, _)| *q))
            .collect::<BTreeSet<_>>();
        if w.iter().all(|g| {
            let t = g.targets[0];
            !x.contains(&t)
                && !b.contains(&t)
                && !controls.contains(&t)
                && g.controls.iter().filter(|(q, _)| b.contains(q)).count() <= 1
        }) {
            valid = true;
        }
    }
    if !valid {
        bail!(
            "checker: borrow frame fails translation lemma (§5.6); unimplemented: certificate-backed borrow frame (milestone 2)"
        )
    }
    Ok(())
}
pub fn validate(o: &Object) -> Result<()> {
    crate::metadata::validate(o)?;
    if let Some(cert) = &o.certificate {
        for region in cert
            .get("regions")
            .and_then(|v| v.as_object())
            .into_iter()
            .flat_map(|m| m.values())
        {
            for claim in region
                .get("claims")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                if matches!(
                    claim.get("kind").and_then(|v| v.as_str()),
                    Some("numerical" | "channel")
                ) || claim.get("expected").and_then(|v| v.as_str()) == Some("oracle")
                    || claim
                        .get("subject")
                        .and_then(|s| s.get("schedule_hash"))
                        .is_some()
                {
                    bail!(
                        "E_SCHEDULE_INFEASIBLE: executable/numerical/channel certificate replay requires expansion library b-isa-spec-8r4 (§12.1)"
                    )
                }
            }
        }
    }

    region_graph(o)?;
    for (label, value, keys, required) in [
        (
            "schedule",
            o.schedule.as_ref(),
            vec!["bindings", "entries"],
            vec!["bindings", "entries"],
        ),
        (
            "certificate",
            o.certificate.as_ref(),
            vec!["versions", "bindings", "producer", "regions", "composition"],
            vec!["versions", "bindings", "producer", "regions", "composition"],
        ),
        (
            "coverage",
            o.coverage.as_ref(),
            vec!["entries"],
            vec!["entries"],
        ),
    ] {
        if let Some(value) = value {
            let Some(map) = value.as_object() else {
                bail!("checker: {label} must be object (§9.1)")
            };
            if map.keys().any(|k| !keys.contains(&k.as_str()))
                || required.iter().any(|k| !map.contains_key(*k))
            {
                bail!("checker: {label} unknown or missing fields (§9.1)")
            }
        }
    }

    for f in &o.functions {
        schema(f, o)?;
        if matches!(f.kind, FnKind::Cc | FnKind::R) {
            classical_schema(o, f)?;
        }
        if o.layout.is_none() && f.kind == FnKind::Lproc {
            bail!("checker: lproc in memoryless object (§7.2)")
        }
        if f.kind == FnKind::Oracle && o.layout.is_none() && !f.groups[3].is_empty() {
            bail!("checker: memory slots in memoryless object (§7.2)")
        }
        if f.is_action() {
            for i in all(f) {
                if i.opcode == "qloop" && !matches!(i.operands.first(), Some(Operand::Int(_))) {
                    bail!(
                        "unimplemented: §16.1/§5.3 symbolic trip-count object validation (milestone 2)"
                    );
                }
            }
            action_readonly(o, f)?;
            crate::selectors::validate(o, f)?;
            if (!f.groups[3].is_empty() || !o.imports.is_empty()) && crate::history::verify(o, f)? {
                continue;
            }
            if !f.groups[3].is_empty() {
                if parameter_independent_action(o, f)? || composed_parameter_action(o, f)? {
                    continue;
                } else if f.groups[3].iter().all(|p| p.ty == Type::F64) {
                    symbolic_angle_action(o, f)?;
                } else {
                    static_parameter_action(o, f)?;
                }
                continue;
            }
            let p = crate::lower::lower_program(o, &f.name)
                .map_err(|e| err(format!("{} (§3.3/§5.1)", e.0)))?;
            abstract_action(o, f, &p)?;
        }
    }
    Ok(())
}

/// Erase only parameters syntactically absent from this computation.
/// This is universal quantification by independence, not execution at a
/// chosen integer or token. Remaining angle atoms use the exact history proof.
fn parameter_independent_action(o: &Object, f: &Function) -> Result<bool> {
    let mut pending = vec![f];
    let mut visited = BTreeSet::new();
    while let Some(cf) = pending.pop() {
        if !visited.insert(&cf.name) {
            continue;
        }
        for i in all(cf) {
            for a in &i.operands {
                if let Operand::Sym(n) = a {
                    if o.imports.iter().any(|import| import.name == *n) {
                        return Ok(false);
                    }
                    if let Some(callee) = o.function(n) {
                        pending.push(callee);
                    }
                }
            }
        }
    }
    let mut used = BTreeSet::new();
    for i in all(f) {
        for a in &i.operands {
            match a {
                Operand::Name(n) | Operand::Indexed(n, _) => {
                    used.insert(n.as_str());
                }
                Operand::List(ns) => {
                    used.extend(ns.iter().map(String::as_str));
                }
                _ => {}
            }
        }
    }
    if f.groups[3]
        .iter()
        .any(|p| p.ty != Type::F64 && used.contains(p.name.as_str()))
    {
        return Ok(false);
    }
    let mut independent = o.clone();
    let cf = independent
        .functions
        .iter_mut()
        .find(|cf| cf.name == f.name)
        .unwrap();
    cf.groups[3].retain(|p| p.ty == Type::F64 && used.contains(p.name.as_str()));
    let cf = cf.clone();
    if cf.groups[3].is_empty() {
        let program = crate::lower::lower_program(&independent, &cf.name)?;
        abstract_action(o, f, &program)?;
    } else {
        symbolic_angle_action(&independent, &cf)?;
    }
    Ok(true)
}

/// Composition of independently proved clean-workspace sections and opaque
/// interface contracts. A section must return *all* its workspace before the
/// next call; no assumption about that call's implementation enters the proof.
fn composed_parameter_action(o: &Object, f: &Function) -> Result<bool> {
    fn region_call(i: &Instr) -> bool {
        let mut op = i.opcode.as_str();
        let mut args = i.operands.as_slice();
        while matches!(op, "ctl" | "adj") {
            let offset = usize::from(op == "ctl");
            let Some(Operand::Name(next)) = args.get(offset) else {
                return false;
            };
            op = next;
            args = &args[offset + 1..];
        }
        matches!(op, "oracle_call" | "action_call" | "qinplace")
    }
    if f.blocks.len() != 1 || !f.blocks[0].params.is_empty() {
        return Ok(false);
    }
    let body = &f.blocks[0].instrs;
    // Predicate/structured contexts crossing section boundaries require a
    // joint symbolic state. Leave them to the general contract checker.
    if !body.iter().any(region_call)
        || body.iter().any(|i| {
            matches!(i.opcode.as_str(), "qpred" | "qunpred")
                || i.children
                    .iter()
                    .flatten()
                    .any(|i| !i.children.is_empty() || region_call(i))
        })
    {
        return Ok(false);
    }
    let mut sections: Vec<(bool, Vec<Instr>)> = Vec::new();
    for i in body {
        let call = region_call(i);
        if !call && sections.last().is_some_and(|(c, _)| !c) {
            sections.last_mut().unwrap().1.push(i.clone());
        } else {
            sections.push((call, vec![i.clone()]));
        }
    }
    // Do not invent bounds for the original action: its literal loop list
    // must agree before individual sections receive their own lists.
    let loop_bounds = |cf: &Function| -> Result<Vec<u64>> {
        all(cf)
            .iter()
            .filter(|i| i.opcode == "qloop")
            .map(|i| {
                i.operands
                    .first()
                    .ok_or_else(|| err("checker: qloop arity (§4.6)"))
                    .and_then(int)
            })
            .collect()
    };
    if loop_bounds(f)? != f.bounds {
        bail!("checker: action bounds differ from loop counts (§5.3)")
    }
    for (call, instructions) in sections {
        let mut section = f.clone();
        section.blocks[0].instrs = instructions;
        section.bounds = loop_bounds(&section)?;
        if !call {
            let mut live = BTreeSet::new();
            for i in all(&section) {
                match i.opcode.as_str() {
                    "qregion" => return Ok(false),
                    "qalloc" | "qborrow" | "qcopy" | "qload" => live.extend(i.lhs.iter().cloned()),
                    "qfree" | "qreturn" | "quncopy" | "qunload" => {
                        if let Some(Operand::Name(n)) = i.operands.last() {
                            live.remove(n);
                        }
                    }
                    _ => {}
                }
            }
            if !live.is_empty() {
                return Ok(false);
            }
        }
        let mut context = o.clone();
        *context
            .functions
            .iter_mut()
            .find(|cf| cf.name == f.name)
            .unwrap() = section.clone();
        if call {
            static_parameter_action(&context, &section)?;
        } else if !parameter_independent_action(&context, &section)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Angles are uninterpreted atoms here, not samples. The abstract gate
/// transfer uses only syntactic angle equality/negation, never trig or a
/// numerical identity. Encode distinct atoms in collision-free f64 tags so
/// the existing IR can carry them through calls and adjoints. Tags are never
/// simulated, exported, or used for bounds/circuit control decisions.
fn symbolic_angle_action(o: &Object, f: &Function) -> Result<()> {
    let mut forbidden = BTreeSet::new();
    for cf in &o.functions {
        for i in all(cf) {
            for a in &i.operands {
                let texts: &[String] = match a {
                    Operand::Float(s) | Operand::Int(s) => std::slice::from_ref(s),
                    Operand::List(ss) => ss,
                    _ => &[],
                };
                for text in texts {
                    if let Ok(v) = text.parse::<f64>() {
                        forbidden.insert(v.abs().to_bits());
                    }
                }
            }
        }
    }
    let mut tag = 1f64;
    let mut args = Vec::new();
    for _ in &f.groups[3] {
        while forbidden.contains(&tag.to_bits()) {
            tag += 1.;
        }
        args.push(crate::oracle::Argument::Float(tag));
        forbidden.insert(tag.to_bits());
        tag += 1.;
    }
    let p = crate::lower::lower_program_with_args(o, &f.name, &args)?;
    abstract_action(o, f, &p)
}

fn action_readonly(o: &Object, f: &Function) -> Result<()> {
    let readonly = f.groups[1]
        .iter()
        .map(|p| p.name.clone())
        .chain(
            f.groups[2]
                .iter()
                .filter(|p| p.role.as_deref() == Some("in"))
                .map(|p| p.name.clone()),
        )
        .collect::<BTreeSet<_>>();
    let base = |a: &Operand| -> Option<String> {
        match a {
            Operand::Name(n) | Operand::Indexed(n, _) => Some(n.clone()),
            _ => None,
        }
    };
    for i in all(f) {
        let mut head = i.opcode.as_str();
        let mut actuals = i.operands.as_slice();
        while head == "adj" || head == "ctl" {
            let offset = if head == "ctl" { 1 } else { 0 };
            let Some(Operand::Name(next)) = actuals.get(offset) else {
                bail!("checker: malformed controlled/adjoint call (§4.7)")
            };
            head = next;
            actuals = &actuals[offset + 1..];
        }
        let mut written = Vec::new();
        if head == "oracle_call" {
            for index in [3, 4] {
                if let Some(n) = actuals.get(index).and_then(base) {
                    written.push(n);
                }
            }
        }
        if head == "action_call" {
            if let Some(Operand::List(ns)) = actuals.get(1) {
                written.extend(ns.clone());
            }
            if let (Some(Operand::Sym(name)), Some(Operand::List(ns))) =
                (actuals.first(), actuals.get(3))
            {
                let cf = callee_contract(o, name)?;
                for (n, param) in ns.iter().zip(&cf.groups[2]) {
                    if param.role.as_deref() == Some("mut") {
                        written.push(n.clone());
                    }
                }
            }
        }
        if head == "qinplace"
            && let Some(n) = actuals.get(3).and_then(base)
        {
            written.push(n);
        }
        if written.iter().any(|n| readonly.contains(n)) {
            bail!("checker: in wire/region passed to mutable call slot (§5.1)")
        }
        let targets = match i.opcode.as_str() {
            "x" | "h" => vec![0],
            "rx" | "ry" => vec![1],
            "cx" => vec![1],
            "ccx" => vec![2],
            "mcx" => vec![1],
            "qxor" => vec![1],
            "qexch" | "swap" => vec![0, 1],
            "qexchm" => vec![0, 2],
            "qstore" => vec![0],
            _ => Vec::new(),
        };
        for t in targets {
            if i.operands
                .get(t)
                .and_then(base)
                .is_some_and(|n| readonly.contains(&n))
            {
                bail!("checker: non-diagonal write to in wire/region (§5.1)")
            }
        }
    }
    Ok(())
}

pub(crate) fn static_parameter_action(o: &Object, f: &Function) -> Result<()> {
    // Contract-only interface calls have no local cleanup obligations. Their
    // structured compositions are checked inductively, including zero-trip
    // bodies. Never choose a concrete token to validate all tokens.
    let names = f
        .groups
        .iter()
        .flatten()
        .map(|p| (p.name.clone(), p.ty.clone()))
        .collect::<BTreeMap<_, _>>();
    fn control(op: &Operand, names: &BTreeMap<String, Type>) -> Result<(String, u32)> {
        match op {
            Operand::Name(n) if names.get(n) == Some(&Type::Wire(1)) => Ok((n.clone(), 0)),
            Operand::Indexed(n, i) if matches!(names.get(n),Some(Type::Wire(w))if i<w) => {
                Ok((n.clone(), *i))
            }
            _ => bail!("checker: symbolic control requires one live interface wire (§3.3)"),
        }
    }
    fn prepare(
        is: &[Instr],
        names: &BTreeMap<String, Type>,
        inherited: &[(String, u32)],
        out: &mut Vec<Instr>,
    ) -> Result<()> {
        let mut active = inherited.to_vec();
        let mut pushed = Vec::new();
        for i in is {
            if i.opcode == "qpred" || i.opcode == "qunpred" {
                if i.operands.len() != 1 || !i.lhs.is_empty() || !i.children.is_empty() {
                    bail!("checker: predicate schema (§3.3)")
                }
                let c = control(&i.operands[0], names)?;
                if i.opcode == "qpred" {
                    if active.contains(&c) {
                        bail!("checker: duplicate effective control (§3.3)")
                    }
                    active.push(c.clone());
                    pushed.push(c);
                } else {
                    if pushed.pop() != Some(c.clone()) || active.pop() != Some(c) {
                        bail!("checker: unmatched predicate pop (§3.5)")
                    }
                }
                continue;
            }
            if i.opcode == "qif" || i.opcode == "qloop" {
                if !i.lhs.is_empty()
                    || i.children.is_empty()
                    || i.children.len() > if i.opcode == "qif" { 2 } else { 1 }
                {
                    bail!("checker: structured interface schema (§4.6)")
                }
                let mut inner = active.clone();
                if i.opcode == "qif" {
                    if i.operands.len() != 1 {
                        bail!("checker: qif arity (§4.6)")
                    }
                    let c = control(&i.operands[0], names)?;
                    if inner.contains(&c) {
                        bail!("checker: duplicate selector/effective control (§3.3)")
                    }
                    inner.push(c);
                } else {
                    let Some(Operand::Int(_)) = i.operands.first() else {
                        bail!(
                            "unimplemented: §5.3 symbolic trip-count bound validation (milestone 2)"
                        )
                    };
                    let n = int(&i.operands[0])?;
                    if i.operands.len() == 3 {
                        if !matches!(&i.operands[1],Operand::Name(s)if s=="while") {
                            bail!("checker: qloop while schema (§4.6)")
                        }
                        let Operand::Name(cond) = &i.operands[2] else {
                            bail!("checker: while condition bundle (§4.6)")
                        };
                        if !matches!(names.get(cond),Some(Type::Wire(w))if u64::from(*w)==n) {
                            bail!("checker: condition width/count (§4.6)")
                        }
                        // Each iteration's flag depends on its prefix of cond;
                        // reserve all its wires when checking the body.
                        for j in 0..n {
                            let c = (cond.clone(), j as u32);
                            if inner.contains(&c) {
                                bail!("checker: condition/effective control overlap (§3.3)")
                            };
                            inner.push(c);
                        }
                    } else if i.operands.len() != 1 {
                        bail!("checker: qloop arity (§4.6)")
                    }
                }
                for body in &i.children {
                    prepare(body, names, &inner, out)?;
                }
                continue;
            }
            let modified = i.opcode == "ctl" || i.opcode == "adj";
            let mut plain = i.clone();
            let mut context = active.clone();
            while plain.opcode == "ctl" || plain.opcode == "adj" {
                let count = if plain.opcode == "ctl" {
                    let c = control(
                        plain
                            .operands
                            .first()
                            .ok_or_else(|| err("checker: ctl arity (§3.3)"))?,
                        names,
                    )?;
                    if context.contains(&c) {
                        bail!("checker: duplicate effective control (§3.3)")
                    };
                    context.push(c);
                    1
                } else {
                    0
                };
                let Some(Operand::Name(next)) = plain.operands.get(count) else {
                    bail!("checker: call modifier schema (§4.7)")
                };
                plain.opcode = next.clone();
                plain.operands.drain(..count + 1);
            }
            if modified && !matches!(plain.opcode.as_str(), "oracle_call" | "action_call") {
                bail!("checker: modifiers require region call (§4.7)")
            }
            if !plain.children.is_empty() {
                bail!("checker: call has unexpected body (§3.3)")
            }
            let actuals = match plain.opcode.as_str() {
                "qinplace" => plain.operands.get(3..4),
                "oracle_call" => plain.operands.get(2..6),
                "action_call" => plain.operands.get(1..4),
                _ => None,
            };
            if let Some(actuals) = actuals {
                for a in actuals {
                    let ns = match a {
                        Operand::List(ns) => ns.clone(),
                        Operand::Name(n) => vec![n.clone()],
                        _ => bail!("checker: call operands (§3.3)"),
                    };
                    if ns.iter().any(|n| context.iter().any(|(c, _)| c == n)) {
                        bail!("checker: effective control overlaps call operand (§3.3)")
                    }
                }
            }
            out.push(plain);
        }
        if !pushed.is_empty() {
            bail!("checker: predicate stack not empty at structured exit (§3.5)")
        }
        Ok(())
    }
    if f.blocks.len() != 1 || !f.blocks[0].params.is_empty() {
        bail!(
            "unimplemented: §6 symbolic action CFG validation; decision bennett-circuits-7bu (milestone 2)"
        )
    }
    let mut calls = Vec::new();
    prepare(&f.blocks[0].instrs, &names, &[], &mut calls)?;
    for i in &calls {
        match i.opcode.as_str() {
            "action_call" => {
                if i.operands.len() != 5 || !i.lhs.is_empty() {
                    bail!("checker: symbolic action call schema (§3.3)");
                }
                let Operand::Sym(name) = &i.operands[0] else {
                    bail!("checker: action symbol (§4.7)")
                };
                let cf = callee_contract(o, name)?;
                if !cf.is_action() {
                    bail!("checker: action callee kind (§4.7)")
                }
                if f.kind == FnKind::PermAction && cf.kind != FnKind::PermAction {
                    bail!("checker: permutation action calls non-permutation action (§5.3)")
                }
                let mut distinct = BTreeSet::new();
                for (k, group) in [(1, 0), (2, 1), (3, 2)] {
                    let Operand::List(ns) = &i.operands[k] else {
                        bail!("checker: action list (§3.3)")
                    };
                    if ns.len() != cf.groups[group].len() {
                        bail!("checker: action interface arity (§3.3)")
                    }
                    for (n, p) in ns.iter().zip(&cf.groups[group]) {
                        if !distinct.insert(n) || names.get(n) != Some(&p.ty) {
                            bail!("checker: action operand overlap/type/width (§3.3)")
                        }
                        if group == 0 && !f.groups[0].iter().any(|p| p.name == *n) {
                            bail!("checker: action mut slot requires mutable caller bundle (§5.1)")
                        }
                        if group == 2
                            && p.role.as_deref() == Some("mut")
                            && !f.groups[2]
                                .iter()
                                .any(|p| p.name == *n && p.role.as_deref() == Some("mut"))
                        {
                            bail!("checker: in region passed to mut slot (§5.1)")
                        }
                    }
                }
                let Operand::List(args) = &i.operands[4] else {
                    bail!("checker: action argument list (§3.3)")
                };
                if args.len() != cf.groups[3].len() {
                    bail!("checker: classical argument arity (§3.3)")
                }
                for (n, p) in args.iter().zip(&cf.groups[3]) {
                    if let Some(t) = names.get(n) {
                        if *t != p.ty {
                            bail!("checker: classical argument type (§3.3)")
                        }
                    } else {
                        let valid = match p.ty {
                            Type::F64 => n.parse::<f64>().is_ok_and(f64::is_finite),
                            Type::Int(w) => n
                                .parse::<u128>()
                                .is_ok_and(|x| w == 128 || x < (1u128 << w)),
                            Type::Ptr => n.parse::<u128>().is_ok_and(|x| {
                                o.header.ptr_width == 128 || x < (1u128 << o.header.ptr_width)
                            }),
                            _ => false,
                        };
                        if !valid {
                            bail!("checker: classical literal type/range (§3.3)")
                        }
                    }
                }
            }
            "qinplace" => {
                if i.operands.len() != 4 || !i.lhs.is_empty() || !i.children.is_empty() {
                    bail!("checker: qinplace schema (§4.5)")
                }
                let tk = opname(&i.operands[2])?;
                if names.get(tk) != Some(&Type::Token) {
                    bail!("checker: qinplace token type (§4.5)")
                }
                let x = opname(&i.operands[3])?;
                let Some(Type::Wire(width)) = names.get(x) else {
                    bail!("checker: qinplace wire encoding (§4.5)")
                };
                if !f.groups[0].iter().any(|p| p.name == x) {
                    bail!("checker: qinplace requires mutable operand (§5.1)")
                }
                let mut fault = None;
                for op in &i.operands[..2] {
                    let Operand::Sym(n) = op else {
                        bail!("checker: qinplace oracle symbol (§4.5)")
                    };
                    let cf = o.function(n).ok_or_else(|| {
                        err("unimplemented: qinplace imported inverse contract; decision bennett-circuits-4pf (milestone 2)")
                    })?;
                    if cf.kind != FnKind::Oracle
                        || cf.groups[0].len() != 1
                        || cf.groups[0][0].ty.wire_width() != Some(*width)
                        || cf.groups[1][0].ty.wire_width() != Some(*width)
                        || !cf.groups[3].is_empty()
                        || !cf.groups[4].is_empty()
                    {
                        bail!("checker: qinplace unary inverse signature (§4.5)")
                    }
                    let fw = cf.groups[2][0].ty.wire_width();
                    if let Some(prior) = fault
                        && fw != prior
                    {
                        bail!("checker: qinplace fault width mismatch (§4.5)")
                    }
                    fault = Some(fw);
                }
            }
            "oracle_call" => {
                if i.operands.len() != 7 || !i.lhs.is_empty() || !i.children.is_empty() {
                    bail!("checker: oracle_call schema (§3.3)")
                }
                let mut lifted = i.clone();
                lifted.opcode = "lift.oracle_call".into();
                lifted_call(o, f, &lifted, &names)?;
                let mut operands = Vec::new();
                for op in &i.operands[2..6] {
                    match op {
                        Operand::Name(n) => operands.push(n.clone()),
                        Operand::List(ns) => operands.extend(ns.clone()),
                        _ => bail!("checker: oracle operands (§3.3)"),
                    }
                }
                if operands.iter().collect::<BTreeSet<_>>().len() != operands.len() {
                    bail!("checker: oracle operands overlap (§3.3)")
                }
                for index in [3, 4] {
                    let name = opname(&i.operands[index])?;
                    if !f.groups[0].iter().any(|p| p.name == name) {
                        bail!("checker: oracle output must be mutable (§5.1)")
                    }
                }
            }
            _ => bail!(
                "unimplemented: §6 symbolic classical action validation for {}; decisions bennett-circuits-7bu and bennett-circuits-z9r (milestone 2)",
                i.opcode
            ),
        }
    }
    Ok(())
}
#[derive(Clone, PartialEq)]
enum Pair {
    Copy(String, Vec<Ctrl>),
    Load(String, String, Vec<Ctrl>),
}
fn validate_pairs(p: &Program) -> Result<()> {
    for c in &p.circuits {
        let mut pairs: BTreeMap<String, (Pair, bool)> = BTreeMap::new();
        let mut regions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let close = |region: &str,
                     pairs: &mut BTreeMap<String, (Pair, bool)>,
                     regions: &BTreeMap<String, BTreeSet<String>>| {
            for (n, (_, closed)) in pairs {
                if regions.get(n).is_some_and(|rs| rs.contains(region)) {
                    *closed = true;
                }
            }
        };
        for node in &c.body {
            match node {
                Node::Copy { src, dst, controls } => {
                    pairs.insert(
                        dst.clone(),
                        (Pair::Copy(src.clone(), controls.clone()), false),
                    );
                    regions.insert(dst.clone(), regions.get(src).cloned().unwrap_or_default());
                }
                Node::Load {
                    region,
                    addr,
                    dst,
                    controls,
                } => {
                    pairs.insert(
                        dst.clone(),
                        (
                            Pair::Load(region.clone(), addr.clone(), controls.clone()),
                            false,
                        ),
                    );
                    let mut rs = regions.get(addr).cloned().unwrap_or_default();
                    rs.insert(region.clone());
                    regions.insert(dst.clone(), rs);
                }
                Node::Uncopy { src, dst, controls } => {
                    if pairs.remove(dst) != Some((Pair::Copy(src.clone(), controls.clone()), false))
                    {
                        bail!("checker: quncopy sequence/control mismatch or closed window (§6.2)")
                    }
                    regions.remove(dst);
                }
                Node::Unload {
                    region,
                    addr,
                    dst,
                    controls,
                } => {
                    if pairs.remove(dst)
                        != Some((
                            Pair::Load(region.clone(), addr.clone(), controls.clone()),
                            false,
                        ))
                    {
                        bail!(
                            "checker: qunload sequence/control mismatch or cell version advanced (§6.2)"
                        )
                    }
                    regions.remove(dst);
                }
                Node::Store { region, .. } => close(region, &mut pairs, &regions),
                Node::ExchM {
                    region, addr, val, ..
                } => {
                    close(region, &mut pairs, &regions);
                    let mut rs = regions.get(val).cloned().unwrap_or_default();
                    rs.extend(regions.get(addr).cloned().unwrap_or_default());
                    rs.insert(region.clone());
                    regions.insert(val.clone(), rs);
                }
                Node::Xor { src, tgt, .. } => {
                    let rs = regions.get(src).cloned().unwrap_or_default();
                    regions.entry(tgt.clone()).or_default().extend(rs);
                }
                Node::Exch { a, b, .. } => {
                    let mut rs = regions.get(a).cloned().unwrap_or_default();
                    rs.extend(regions.get(b).cloned().unwrap_or_default());
                    regions.insert(a.clone(), rs.clone());
                    regions.insert(b.clone(), rs);
                }
                Node::Call { callee, args, .. } => {
                    let cf = p
                        .circuit(callee)
                        .ok_or_else(|| err("checker: missing callee"))?;
                    let mut rs = BTreeSet::new();
                    for n in args {
                        rs.extend(regions.get(n).cloned().unwrap_or_default());
                    }
                    let offset = cf.mut_bundles.len() + cf.in_bundles.len();
                    for (m, n) in cf.mem.iter().zip(&args[offset..]) {
                        rs.insert(n.clone());
                        if m.role == Role::Mut {
                            close(n, &mut pairs, &regions);
                        }
                    }
                    for n in &args[..cf.mut_bundles.len()] {
                        regions.entry(n.clone()).or_default().extend(rs.clone());
                    }
                }
                Node::Gate(g) => {
                    let mut rs = BTreeSet::new();
                    for q in g.targets.iter().chain(g.controls.iter().map(|c| &c.qubit)) {
                        rs.extend(regions.get(&q.bundle).cloned().unwrap_or_default());
                    }
                    for q in &g.targets {
                        regions
                            .entry(q.bundle.clone())
                            .or_default()
                            .extend(rs.clone());
                    }
                }
            }
        }
        if !pairs.is_empty() {
            bail!("checker: unmatched copy/load cleanup (§6.2)")
        }
    }
    Ok(())
}
fn classical_schema(o: &Object, f: &Function) -> Result<()> {
    let types = f.groups[0]
        .iter()
        .map(|p| (p.name.clone(), p.ty.clone()))
        .collect::<BTreeMap<_, _>>();
    let handles: BTreeMap<String, Option<String>> = f.groups[0]
        .iter()
        .filter(|p| matches!(p.ty, Type::QReg(_)))
        .map(|p| (p.name.clone(), None))
        .collect();
    let viewed: BTreeSet<String> = BTreeSet::new();
    let mut known_types = types.clone();
    let mut analyzed = BTreeSet::new();
    let mut work = vec![(0usize, types, handles, viewed)];
    let mut seen = BTreeSet::new();
    loop {
        while let Some((bi, mut types, mut handles, mut viewed)) = work.pop() {
            if !seen.insert(format!("{bi}:{types:?}:{handles:?}:{viewed:?}")) {
                continue;
            }
            analyzed.insert(bi);
            let b = &f.blocks[bi];
            let mut edges = Vec::new();
            for (ordinal, i) in b.instrs.iter().enumerate() {
                let term = matches!(i.opcode.as_str(), "br" | "ret" | "trap");
                if term != (ordinal + 1 == b.instrs.len()) {
                    bail!("checker: classical block terminator position (§4.1)");
                }

                let a = &i.operands;
                let op = i.opcode.as_str();
                if o.layout.is_none()
                    && (matches!(op, "alloc" | "free")
                        || op.starts_with("load.")
                        || op.starts_with("store."))
                {
                    bail!("checker: classical memory in memoryless object (§7.2)")
                }
                if f.kind != FnKind::Cc && (op.starts_with("qreg.") || op == "measure") {
                    bail!("checker: qreg lifecycle and measure are CC-only (§4.8)")
                }

                let get = |a: &Operand| -> Result<Type> {
                    let n = opname(a)?;
                    types.get(n).cloned().ok_or_else(|| {
                        err(format!(
                            "checker: @{0} unknown classical value {n} (§3.3)",
                            f.name
                        ))
                    })
                };
                let arity = |n: usize, r: usize| -> Result<()> {
                    if a.len() != n || i.lhs.len() != r || !i.children.is_empty() {
                        bail!("checker: {op} operand/result arity (§3.3)")
                    }
                    Ok(())
                };
                let same = |a: &Type, b: &Type| -> Result<()> {
                    if a != b {
                        bail!("checker: {op} type mismatch (§3.3)")
                    }
                    Ok(())
                };
                let numeric = |t: &Type| -> Result<()> {
                    if !matches!(t, Type::Int(_) | Type::Ptr) {
                        bail!("checker: {op} requires integer/pointer (§3.3)")
                    }
                    Ok(())
                };
                let suffix = |prefix: &str| -> Result<u32> {
                    let text = op
                        .strip_prefix(prefix)
                        .ok_or_else(|| err("checker: missing width (§3.3)"))?;
                    let n = text
                        .parse::<u32>()
                        .map_err(|_| err("checker: invalid width (§3.3)"))?;
                    if n == 0 || n > 128 || n.to_string() != text {
                        bail!("checker: width outside i1..i128 (§3.3)")
                    }
                    Ok(n)
                };
                let mut result = None;
                match op {
                    "add" | "sub" | "mul" | "and" | "or" | "xor" | "shl" | "lshr" | "ashr"
                    | "udiv" | "urem" | "sdiv" | "srem" => {
                        arity(2, 1)?;
                        let t = get(&a[0])?;
                        numeric(&t)?;
                        same(&t, &get(&a[1])?)?;
                        result = Some(t);
                    }
                    "select" => {
                        arity(3, 1)?;
                        same(&get(&a[0])?, &Type::Int(1))?;
                        let t = get(&a[1])?;
                        same(&t, &get(&a[2])?)?;
                        result = Some(t);
                    }
                    "inttoptr" => {
                        arity(1, 1)?;
                        if !matches!(get(&a[0])?, Type::Int(_)) {
                            bail!("checker: inttoptr integer (§3.3)")
                        }
                        result = Some(Type::Ptr);
                    }
                    "alloc" => {
                        arity(1, 1)?;
                        numeric(&get(&a[0])?)?;
                        result = Some(Type::Ptr);
                    }
                    "free" => {
                        arity(1, 0)?;
                        same(&get(&a[0])?, &Type::Ptr)?;
                    }
                    "ret" => {
                        arity(f.groups[1].len(), 0)?;
                        for (a, p) in a.iter().zip(&f.groups[1]) {
                            same(&get(a)?, &p.ty)?;
                        }
                    }
                    "trap" => {
                        arity(1, 0)?;
                        if !matches!(a[0], Operand::Int(_)) {
                            bail!("checker: trap literal (§3.3)")
                        }
                    }
                    "br" => {
                        if !i.lhs.is_empty() {
                            bail!("checker: branch has results (§3.3)")
                        }
                        let offset = match a.len() {
                            1 => 0,
                            3 => {
                                same(&get(&a[0])?, &Type::Int(1))?;
                                1
                            }
                            _ => bail!("checker: branch arity (§3.3)"),
                        };
                        for label in &a[offset..] {
                            let Operand::Label(n, Some(args)) = label else {
                                bail!("checker: branch target and args (§3.3)")
                            };
                            let block = f
                                .blocks
                                .iter()
                                .find(|b| b.name == *n)
                                .ok_or_else(|| err("checker: unknown branch label (§3.3)"))?;
                            if args.len() != block.params.len() {
                                bail!("checker: branch parameter count (§3.3)")
                            }
                            for (n, p) in args.iter().zip(&block.params) {
                                same(&get(&Operand::Name(n.clone()))?, &p.ty)?;
                            }
                            edges.push((n.clone(), args.clone()));
                        }
                    }
                    "call" | "uncall" => {
                        let Some(Operand::Sym(n)) = a.first() else {
                            bail!("checker: classical call target (§3.3)")
                        };
                        let cf = callee_contract(o, n)?;
                        if !matches!(cf.kind, FnKind::Cc | FnKind::R)
                            || f.kind == FnKind::R && cf.kind == FnKind::Cc
                        {
                            bail!("checker: call tier (§3.1)")
                        }
                        if op == "call" {
                            arity(cf.groups[0].len() + 1, cf.groups[1].len())?;
                            for (a, p) in a[1..].iter().zip(&cf.groups[0]) {
                                same(&get(a)?, &p.ty)?;
                            }
                            for (n, p) in i.lhs.iter().zip(&cf.groups[1]) {
                                types.insert(n.clone(), p.ty.clone());
                            }
                        } else {
                            arity(3, 0)?;
                            for (index, ps) in [(1, &cf.groups[0]), (2, &cf.groups[1])] {
                                let Operand::List(ns) = &a[index] else {
                                    bail!("checker: uncall tuple (§3.3)")
                                };
                                if ns.len() != ps.len() {
                                    bail!("checker: uncall tuple arity (§3.3)")
                                }
                                for (n, p) in ns.iter().zip(ps) {
                                    same(&get(&Operand::Name(n.clone()))?, &p.ty)?;
                                }
                            }
                        }
                    }
                    "effect" => {
                        if f.kind != FnKind::R || a.is_empty() || !i.lhs.is_empty() {
                            bail!("checker: effect tier/schema (§4.1)")
                        }
                        for a in &a[1..] {
                            get(a)?;
                        }
                    }
                    "hostcall" => {
                        arity(3, 1)?;
                        if f.kind != FnKind::Cc {
                            bail!("checker: hostcall CC only (§4.1)")
                        }
                        same(&get(&a[1])?, &Type::Ptr)?;
                        numeric(&get(&a[2])?)?;
                        result = Some(Type::Int(o.header.ptr_width));
                    }
                    "qreg.new" => {
                        arity(2, 1)?;
                        if f.kind != FnKind::Cc {
                            bail!("checker: qreg.new CC only (§3.3)")
                        }
                        let n = int(&a[1])?;
                        let n = u32::try_from(n).map_err(|_| err("host limit: qreg width"))?;
                        result = Some(Type::QReg(n));
                        if handles.insert(i.lhs[0].clone(), None).is_some() {
                            bail!("checker: overwriting a live handle (§3.3)");
                        }
                    }
                    "qreg.free" | "measure" => {
                        arity(1, usize::from(op == "measure"))?;
                        let n = opname(&a[0])?;
                        if viewed.contains(n) || !handles.contains_key(n) || handles[n].is_some() {
                            bail!("checker: consumed/viewed qreg handle (§3.3)")
                        }
                        let Type::QReg(w) = get(&a[0])? else {
                            bail!("checker: expected qreg (§3.3)")
                        };
                        handles.remove(n);
                        if op == "measure" {
                            if w > 128 {
                                bail!("checker: measure width exceeds 128 (§3.3)")
                            }
                            result = Some(Type::Int(w));
                        }
                    }
                    "qreg.slice" => {
                        arity(3, 1)?;
                        let n = opname(&a[0])?;
                        if viewed.contains(n) || handles.get(n) != Some(&None) {
                            bail!("checker: qreg slice requires whole live handle (§3.3)")
                        }
                        let Type::QReg(w) = get(&a[0])? else {
                            bail!("checker: slice type (§3.3)")
                        };
                        let lo = int(&a[1])?;
                        let len = int(&a[2])?;
                        if lo.checked_add(len).is_none_or(|n| n > u64::from(w)) {
                            bail!("checker: slice bounds (§3.3)")
                        }
                        viewed.insert(n.into());
                        handles.insert(i.lhs[0].clone(), Some(n.into()));
                        result = Some(Type::QReg(len as u32));
                    }
                    "qreg.concat" => {
                        arity(2, 1)?;
                        let mut size = 0u32;
                        for a in a {
                            let n = opname(a)?;
                            if viewed.contains(n) || handles.get(n) != Some(&None) {
                                bail!("checker: concat requires live whole qregs (§3.3)")
                            }
                            let Type::QReg(w) = get(a)? else {
                                bail!("checker: concat type (§3.3)")
                            };
                            size = size
                                .checked_add(w)
                                .ok_or_else(|| err("host limit: concat width"))?;
                            handles.remove(n);
                        }
                        handles.insert(i.lhs[0].clone(), None);
                        result = Some(Type::QReg(size));
                    }
                    "action_call" | "oracle_call" | "adj" => {
                        let (offset, kind) = if op == "adj" {
                            if !matches!(a.first(),Some(Operand::Name(n))if n=="action_call"||n=="oracle_call")
                            {
                                bail!("checker: adj region call (§4.7)")
                            }
                            (1, opname(&a[0])?)
                        } else {
                            (0, op)
                        };
                        let a = &a[offset..];
                        let Some(Operand::Sym(n)) = a.first() else {
                            bail!("checker: region target (§4.7)")
                        };
                        let cf = callee_contract(o, n)?;
                        let mut actuals = Vec::new();
                        let mut expected = Vec::new();
                        if kind == "action_call" {
                            if a.len() != 5 || !cf.is_action() {
                                bail!("checker: action call schema (§4.7)")
                            }
                            for (index, params) in
                                [(1, &cf.groups[0]), (2, &cf.groups[1]), (3, &cf.groups[2])]
                            {
                                let Operand::List(ns) = &a[index] else {
                                    bail!("checker: region operand list (§4.7)")
                                };
                                actuals.extend(ns.clone());
                                expected.extend(params.iter().map(|p| p.ty.clone()));
                            }
                        } else {
                            if a.len() != 7 || cf.kind != FnKind::Oracle {
                                bail!("checker: oracle call schema (§4.7)")
                            }
                            same(&get(&a[1])?, &Type::Token)?;
                            let Operand::List(ins) = &a[2] else {
                                bail!("checker: oracle inputs (§4.7)")
                            };
                            actuals.extend(ins.clone());
                            expected.extend(cf.groups[0].iter().map(|p| p.ty.clone()));
                            for (index, group) in [(3, 1), (4, 2)] {
                                actuals.push(opname(&a[index])?.into());
                                expected.push(cf.groups[group][0].ty.clone());
                            }
                            let Operand::List(mems) = &a[5] else {
                                bail!("checker: oracle mem (§4.7)")
                            };
                            actuals.extend(mems.clone());
                            expected.extend(cf.groups[3].iter().map(|p| p.ty.clone()));
                        }
                        if actuals.len() != expected.len()
                            || actuals.iter().collect::<BTreeSet<_>>().len() != actuals.len()
                        {
                            bail!("checker: region interface arity/disjointness (§3.3)")
                        }
                        for (n, t) in actuals.iter().zip(expected) {
                            let want = match t {
                                Type::Wire(w) => w,
                                Type::Region(a, w) => (1u32
                                    .checked_shl(a)
                                    .ok_or_else(|| err("host limit: region geometry"))?)
                                .checked_mul(w)
                                .ok_or_else(|| err("host limit: region width"))?,
                                _ => bail!("checker: region interface type (§3.3)"),
                            };
                            if types.get(n) != Some(&Type::QReg(want))
                                || viewed.contains(n)
                                || !handles.contains_key(n)
                            {
                                bail!("checker: qreg region binding type/liveness (§3.3)")
                            }
                            if let Some(parent) = handles[n].clone() {
                                handles.remove(n);
                                viewed.remove(&parent);
                            }
                        }
                        let Operand::List(args) = a.last().unwrap() else {
                            bail!("checker: classical args tuple (§4.7)")
                        };
                        let params = &cf.groups[if kind == "oracle_call" { 4 } else { 3 }];
                        if args.len() != params.len() {
                            bail!("checker: region classical arity (§3.3)")
                        }
                        for (n, p) in args.iter().zip(params) {
                            same(&get(&Operand::Name(n.clone()))?, &p.ty)?;
                        }
                    }
                    _ if op.starts_with("const.i") => {
                        arity(1, 1)?;
                        if !matches!(a[0], Operand::Int(_)) {
                            bail!("checker: const literal (§3.3)")
                        }
                        result = Some(Type::Int(suffix("const.i")?));
                    }
                    _ if op.starts_with("icmp.") => {
                        arity(2, 1)?;
                        let t = get(&a[0])?;
                        numeric(&t)?;
                        same(&t, &get(&a[1])?)?;
                        if !["eq", "ne", "ult", "ule", "slt", "sle"].contains(&&op[5..]) {
                            bail!("checker: icmp predicate (§3.3)")
                        }
                        result = Some(Type::Int(1));
                    }
                    _ if op.starts_with("ptrtoint.i") => {
                        arity(1, 1)?;
                        same(&get(&a[0])?, &Type::Ptr)?;
                        result = Some(Type::Int(suffix("ptrtoint.i")?));
                    }
                    _ if op.starts_with("load.i") || op.starts_with("store.i") => {
                        let load = op.starts_with("load");
                        arity(if load { 1 } else { 2 }, usize::from(load))?;
                        let n = suffix(if load { "load.i" } else { "store.i" })?;
                        if n % 8 != 0 {
                            bail!("checker: byte memory width (§3.3)")
                        }
                        same(&get(&a[0])?, &Type::Ptr)?;
                        if load {
                            result = Some(Type::Int(n))
                        } else {
                            same(&get(&a[1])?, &Type::Int(n))?;
                        }
                    }
                    _ if op.starts_with("zext.i")
                        || op.starts_with("sext.i")
                        || op.starts_with("trunc.i") =>
                    {
                        arity(1, 1)?;
                        let Type::Int(w) = get(&a[0])? else {
                            bail!("checker: cast integer (§3.3)")
                        };
                        let n = suffix(if op.starts_with("zext") {
                            "zext.i"
                        } else if op.starts_with("sext") {
                            "sext.i"
                        } else {
                            "trunc.i"
                        })?;
                        if op.starts_with("trunc") && n >= w || !op.starts_with("trunc") && n <= w {
                            bail!("checker: cast strictly widens/narrows (§3.3)")
                        }
                        result = Some(Type::Int(n));
                    }
                    _ => bail!("unimplemented: §3.3 classical schema {op} (milestone 2)"),
                }
                if let Some(t) = result {
                    types.insert(i.lhs[0].clone(), t);
                }
            }
            if edges.is_empty() && (handles.values().any(Option::is_some) || !viewed.is_empty()) {
                bail!("checker: slice must be consumed by exactly one region call (§3.3)")
            }
            known_types.extend(types.clone());
            for (label, args) in edges {
                let next = f.blocks.iter().position(|b| b.name == label).unwrap();
                let mut ts = types.clone();
                let mut hs = handles.clone();
                let mut vs = viewed.clone();
                let mut rename = BTreeMap::new();
                for (n, p) in args.iter().zip(&f.blocks[next].params) {
                    ts.insert(p.name.clone(), p.ty.clone());
                    if matches!(p.ty, Type::QReg(_))
                        && (!handles.contains_key(n)
                            || rename.insert(n.clone(), p.name.clone()).is_some())
                    {
                        bail!("checker: branch passes a consumed/aliased qreg (§3.3)");
                    }
                }
                // Simultaneous renaming preserves view-parent relationships.
                for n in rename.keys() {
                    hs.remove(n);
                    vs.remove(n);
                }
                for (old, new) in &rename {
                    let parent = handles[old]
                        .as_ref()
                        .map(|n| rename.get(n).unwrap_or(n).clone());
                    if old != new && hs.contains_key(new) {
                        bail!("checker: branch overwrites live qreg (§3.3)");
                    }
                    hs.insert(new.clone(), parent);
                    if viewed.contains(old) {
                        vs.insert(new.clone());
                    }
                }
                for parent in hs.values_mut().flatten() {
                    if let Some(n) = rename.get(parent) {
                        *parent = n.clone();
                    }
                }
                work.push((next, ts, hs, vs));
            }
        }
        let Some(bi) = (0..f.blocks.len()).find(|bi| !analyzed.contains(bi)) else {
            break;
        };
        let mut types = known_types.clone();
        for p in &f.blocks[bi].params {
            types.insert(p.name.clone(), p.ty.clone());
        }
        let handles = types
            .iter()
            .filter(|(_, t)| matches!(t, Type::QReg(_)))
            .map(|(n, _)| (n.clone(), None))
            .collect();
        work.push((bi, types, handles, BTreeSet::new()));
    }
    Ok(())
}
pub(crate) fn region_graph(o: &Object) -> Result<()> {
    let functions = o
        .functions
        .iter()
        .filter(|f| {
            matches!(
                f.kind,
                FnKind::Oracle | FnKind::Lproc | FnKind::Action | FnKind::PermAction
            )
        })
        .collect::<Vec<_>>();
    let mut refs = BTreeMap::new();
    let mut region_edges = Vec::new();
    for f in &functions {
        let mut out = Vec::new();
        for i in all(f) {
            if matches!(
                i.opcode.as_str(),
                "lift.call"
                    | "lift.oracle_call"
                    | "lift.oracle_uncall"
                    | "lift.perm_call"
                    | "oracle_call"
                    | "action_call"
                    | "qinplace"
                    | "ctl"
                    | "adj"
            ) {
                for a in &i.operands {
                    if let Operand::Sym(n) = a {
                        out.push(n.clone());
                        if i.opcode != "lift.call" {
                            region_edges.push((f.name.clone(), n.clone()));
                        }
                    }
                }
            }
        }
        refs.insert(f.name.clone(), out);
    }
    let names = functions.iter().map(|f| f.name.as_str()).collect();
    for scc in crate::object::strongly_connected(&refs, &names) {
        if (scc.len() > 1 || refs[&scc[0]].contains(&scc[0]))
            && region_edges
                .iter()
                .any(|(a, b)| scc.contains(a) && scc.contains(b))
        {
            bail!("checker: recursive region calls; recursion exists only through lift.call (§5.1)")
        }
    }
    Ok(())
}
