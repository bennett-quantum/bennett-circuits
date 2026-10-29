//! ISA 0.8 producer validation. Independent of stored identity and execution.
use crate::{
    Error, Result, bail,
    flatten::{Flat, FlatGate, flatten},
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
                    "checker: {} illegal in lifted body (§4.2 exact-permutation only)",
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
                    let cf = o.function(n).ok_or_else(|| {
                        err(format!(
                            "unimplemented: imported lproc schema @{n} (milestone 2)"
                        ))
                    })?;
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
    matched_cleanup(f)?;
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
        let Some(Type::Region(aa, w)) = names.get(n) else {
            bail!("checker: unknown memory region (§3.3)")
        };
        if p.get("A").and_then(|v| v.as_u64()) != Some(*aa as u64)
            || p.get("W").and_then(|v| v.as_u64()) != Some(*w as u64)
        {
            bail!("checker: memory geometry (§3.3)")
        }
        if !f.groups[3].iter().any(|p| p.name == *n) {
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
fn matched_cleanup(f: &Function) -> Result<()> {
    if all(f).iter().any(|i| i.opcode == "lift.oracle_uncall")
        && (f.blocks.len() != 1 || all(f).iter().any(|i| i.opcode == "qjumpr"))
    {
        bail!("unimplemented: §5.5 control-flow matched cleanup analysis (milestone 2)")
    }

    // Path-sensitive callers with indirect jumps need the complete reaching-
    // invocation analysis. Until implemented, refuse that rule explicitly.
    let mut pending: Vec<(&Instr, BTreeSet<String>)> = Vec::new();
    for i in all(f) {
        if i.opcode == "lift.oracle_uncall" {
            let Some(pos) = pending.iter().rposition(|(p, _)| p.operands == i.operands) else {
                bail!("checker: unmatched lifted oracle cleanup (§5.5)")
            };
            if !pending[pos].1.is_empty() {
                bail!("checker: lifted oracle inputs changed before cleanup (§5.5)")
            }
            pending.remove(pos);
        } else if i.opcode == "lift.oracle_call" {
            pending.push((i, BTreeSet::new()));
        } else {
            for (p, changed) in &mut pending {
                let Operand::List(ins) = &p.operands[2] else {
                    unreachable!()
                };
                for n in &i.lhs {
                    if ins.contains(n) {
                        changed.insert(n.clone());
                    }
                }
                if i.opcode.starts_with("lift.store") {
                    changed.insert("memory".into());
                }
            }
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
fn abstract_action(p: &Program) -> Result<()> {
    validate_pairs(p)?;
    let flat = flatten(p)?;
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
    borrow_frames(&flat, &borrowed)?;
    let mut algebra = Algebra::new();
    let one = algebra.intern(Expr::And(vec![]));
    let mut values = (0..flat.num_qubits)
        .map(|i| {
            if ws.contains(&i) && !borrowed.contains(&i) {
                0
            } else {
                algebra.intern(Expr::Atom(i))
            }
        })
        .collect::<Vec<_>>();
    let initial = values.clone();
    let mut deps = vec![BTreeSet::new(); flat.num_qubits];
    let mut closed = BTreeSet::new();
    for g in &flat.gates {
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

        if top.perm && !matches!(g.gate, BaseGate::X | BaseGate::Swap) {
            bail!("checker: permutation action contains non-permutation gate (§5.3)")
        }
        if diagonal {
            continue;
        }
        let perm = matches!(g.gate, BaseGate::X | BaseGate::Swap);
        if !perm {
            for q in &g.targets {
                for w in &ws {
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
                } else {
                    let delta = algebra.xor(values[a], values[b]);
                    let delta = algebra.and(&[condition, delta]);
                    values[a] = algebra.xor(values[a], delta);
                    values[b] = algebra.xor(values[b], delta);
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
                            if name == adj_name(g.gate) && cs == controls =>
                        {
                            input
                        }
                        _ => algebra.intern(Expr::Quantum(
                            format!("{:?}", g.gate),
                            controls.clone(),
                            old,
                        )),
                    };
                    deps[*q].extend(dependency.clone());
                    if values[*q] == 0 {
                        deps[*q].clear();
                    }
                }
            }
        }
    }
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
fn borrow_frames(flat: &Flat, b: &BTreeSet<usize>) -> Result<()> {
    if b.is_empty() {
        return Ok(());
    }
    let touches = |g: &FlatGate| {
        g.targets
            .iter()
            .chain(g.controls.iter().map(|(q, _)| q))
            .any(|q| b.contains(q))
    };
    let first = flat.gates.iter().position(touches);
    let Some(first) = first else { return Ok(()) };
    let last = flat.gates.iter().rposition(touches).unwrap();
    let frame = &flat.gates[first..=last];
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
            vec!["versions", "bindings", "entries"],
            vec!["versions", "bindings", "entries"],
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
            action_readonly(o, f)?;
            if !f.groups[3].is_empty() {
                static_parameter_action(o, f)?;
                continue;
            }
            let p = crate::lower::lower_program(o, &f.name)
                .map_err(|e| err(format!("{} (§3.3/§5.1)", e.0)))?;
            abstract_action(&p)?;
        }
    }
    Ok(())
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
                if let Some(cf) = o.function(name) {
                    for (n, param) in ns.iter().zip(&cf.groups[2]) {
                        if param.role.as_deref() == Some("mut") {
                            written.push(n.clone());
                        }
                    }
                } else {
                    bail!("unimplemented: imported action in-wire contract (milestone 2)")
                }
            }
        }
        if head == "qinplace" {
            if let Some(n) = actuals.get(3).and_then(base) {
                written.push(n);
            }
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

fn static_parameter_action(o: &Object, f: &Function) -> Result<()> {
    // Contract-only straight-line calls with interface operands have no local
    // cleanup obligations. Never choose a concrete token to validate all tokens.
    let names = f
        .groups
        .iter()
        .flatten()
        .map(|p| (p.name.clone(), p.ty.clone()))
        .collect::<BTreeMap<_, _>>();
    for i in all(f) {
        match i.opcode.as_str() {
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
                "unimplemented: §6 symbolic classical action validation for {} (milestone 2)",
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
    if f.blocks.len() > 1 && all(f).iter().any(|i| i.opcode.starts_with("qreg.")) {
        bail!("unimplemented: §3.3 qreg handle liveness across CFG joins (milestone 2)")
    }

    let mut types = f.groups[0]
        .iter()
        .map(|p| (p.name.clone(), p.ty.clone()))
        .collect::<BTreeMap<_, _>>();
    for b in &f.blocks {
        for p in &b.params {
            types.insert(p.name.clone(), p.ty.clone());
        }
    }
    let mut handles: BTreeMap<String, Option<String>> = f.groups[0]
        .iter()
        .filter(|p| matches!(p.ty, Type::QReg(_)))
        .map(|p| (p.name.clone(), None))
        .collect();
    let mut viewed: BTreeSet<String> = BTreeSet::new();
    for b in &f.blocks {
        for i in &b.instrs {
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
                "add" | "sub" | "mul" | "and" | "or" | "xor" | "shl" | "lshr" | "ashr" | "udiv"
                | "urem" | "sdiv" | "srem" => {
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
                    }
                }
                "call" | "uncall" => {
                    let Some(Operand::Sym(n)) = a.first() else {
                        bail!("checker: classical call target (§3.3)")
                    };
                    let cf = o.function(n).ok_or_else(|| {
                        err(format!(
                            "unimplemented: imported classical call schema @{n} (milestone 2)"
                        ))
                    })?;
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
                    handles.insert(i.lhs[0].clone(), None);
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
                    let cf = o.function(n).ok_or_else(|| {
                        err(format!(
                            "unimplemented: imported region schema @{n} (milestone 2)"
                        ))
                    })?;
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
    }
    if handles.values().any(Option::is_some) || !viewed.is_empty() {
        bail!("checker: slice must be consumed by exactly one region call (§3.3)")
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
