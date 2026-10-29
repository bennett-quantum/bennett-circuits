//! Literal bounded oracle construction (§3.6/§5.2). Expansion temporaries
//! are private clean history, reversed along with the B machine frames.
use crate::{
    Error, Result, bail,
    ir::*,
    object::{FnKind, Function, Instr, Object, Operand, Type},
};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub steps: u64,
    pub heap: u64,
    pub stack: u64,
    pub depth: u64,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Argument {
    Integer(String),
    Float(f64),
    Token(Budget),
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Bit {
    C(bool),
    Q(QubitRef, bool),
}
type Word = Vec<Bit>;
fn constant(n: u128, w: usize) -> Word {
    (0..w).map(|i| Bit::C(i < 128 && n >> i & 1 != 0)).collect()
}
fn not(a: &Bit) -> Bit {
    match a {
        Bit::C(c) => Bit::C(!c),
        Bit::Q(q, p) => Bit::Q(q.clone(), !p),
    }
}
fn resize(a: &Word, w: usize) -> Word {
    let mut a = a.clone();
    a.resize(w, Bit::C(false));
    a.truncate(w);
    a
}
fn refs(name: &str, w: u32) -> Word {
    (0..w)
        .map(|i| Bit::Q(QubitRef::new(name, i), true))
        .collect()
}
fn number(v: &crate::json::Value, key: &str) -> Result<u64> {
    v.get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| Error(format!("oracle: layout missing integer {key}")))
}
fn literal(s: &str, w: usize) -> Result<Word> {
    let negative = s.starts_with('-');
    let s = s.strip_prefix('-').unwrap_or(s);
    let mut a = vec![false; w];
    for c in s.bytes() {
        if !c.is_ascii_digit() {
            bail!("oracle: invalid integer {s}");
        }
        let mut carry = (c - b'0') as u16;
        for b in &mut a {
            let n = (*b as u16) * 10 + carry;
            *b = n & 1 != 0;
            carry = n >> 1;
        }
    }
    if negative {
        let mut carry = true;
        for b in &mut a {
            *b = !*b;
            let n = *b as u8 + carry as u8;
            *b = n & 1 != 0;
            carry = n > 1;
        }
    }
    Ok(a.into_iter().map(Bit::C).collect())
}
struct Build {
    c: Circuit,
    serial: u64,
}
impl Build {
    fn alloc(&mut self, w: usize, label: &str) -> Word {
        let name = format!("${label}{}", self.serial);
        self.serial += 1;
        self.c.workspace.push(Workspace {
            name: name.clone(),
            width: w.try_into().expect("host limit: workspace width"),
            kind: WsKind::Clean,
            class: "any".into(),
        });
        refs(&name, w as u32)
    }
    fn flip(&mut self, t: &Bit, cs: &[Bit]) {
        let Bit::Q(t, true) = t else {
            panic!("compiler target must be a physical wire")
        };
        let mut controls: BTreeMap<QubitRef, bool> = BTreeMap::new();
        for c in cs {
            match c {
                Bit::C(false) => return,
                Bit::C(true) => {}
                Bit::Q(q, p) => {
                    if let Some(old) = controls.insert(q.clone(), *p)
                        && old != *p
                    {
                        return;
                    }
                }
            }
        }
        assert!(!controls.contains_key(t), "compiler self-controlled X");
        self.c.body.push(Node::Gate(GateInst {
            gate: BaseGate::X,
            controls: controls
                .into_iter()
                .map(|(qubit, positive)| Ctrl { qubit, positive })
                .collect(),
            targets: vec![t.clone()],
        }));
    }
    fn xor(&mut self, a: &Bit, b: &Bit) -> Bit {
        if *a == Bit::C(false) {
            return b.clone();
        }
        if *b == Bit::C(false) {
            return a.clone();
        }
        if *a == Bit::C(true) {
            return not(b);
        }
        if *b == Bit::C(true) {
            return not(a);
        }
        if a == b {
            return Bit::C(false);
        }
        if *a == not(b) {
            return Bit::C(true);
        }
        let t = self.alloc(1, "xor")[0].clone();
        self.flip(&t, std::slice::from_ref(a));
        self.flip(&t, std::slice::from_ref(b));
        t
    }
    fn and(&mut self, a: &Bit, b: &Bit) -> Bit {
        if *a == Bit::C(false) || *b == Bit::C(false) {
            return Bit::C(false);
        }
        if *a == Bit::C(true) {
            return b.clone();
        }
        if *b == Bit::C(true) || a == b {
            return a.clone();
        }
        if *a == not(b) {
            return Bit::C(false);
        }
        let t = self.alloc(1, "and")[0].clone();
        self.flip(&t, &[a.clone(), b.clone()]);
        t
    }
    fn or(&mut self, a: &Bit, b: &Bit) -> Bit {
        not(&self.and(&not(a), &not(b)))
    }
    fn mux(&mut self, p: &Bit, a: &Bit, b: &Bit) -> Bit {
        if a == b {
            return a.clone();
        }
        let d = self.xor(a, b);
        let d = self.and(p, &d);
        self.xor(b, &d)
    }
    fn muxw(&mut self, p: &Bit, a: &Word, b: &Word) -> Word {
        assert_eq!(a.len(), b.len());
        a.iter().zip(b).map(|(a, b)| self.mux(p, a, b)).collect()
    }
    fn eq(&mut self, a: &Word, b: &Word) -> Bit {
        let n = a.len().max(b.len());
        let (a, b) = (resize(a, n), resize(b, n));
        let mut p = Bit::C(true);
        for (a, b) in a.iter().zip(&b) {
            let d = not(&self.xor(a, b));
            p = self.and(&p, &d);
        }
        p
    }
    fn ult(&mut self, a: &Word, b: &Word) -> Bit {
        let n = a.len().max(b.len());
        let (a, b) = (resize(a, n), resize(b, n));
        let mut p = Bit::C(false);
        for (a, b) in a.iter().zip(&b) {
            let eq = not(&self.xor(a, b));
            let lt = self.and(&not(a), b);
            let prev = self.and(&eq, &p);
            p = self.or(&lt, &prev);
        }
        p
    }
    fn add(&mut self, a: &Word, b: &Word, sub: bool) -> Word {
        assert_eq!(a.len(), b.len());
        let mut c = Bit::C(sub);
        a.iter()
            .zip(b)
            .map(|(a, b)| {
                let b = if sub { not(b) } else { b.clone() };
                let x = self.xor(a, &b);
                let s = self.xor(&x, &c);
                let ab = self.and(a, &b);
                let xc = self.and(&x, &c);
                c = self.or(&ab, &xc);
                s
            })
            .collect()
    }
    fn neg(&mut self, a: &Word) -> Word {
        self.add(&constant(0, a.len()), a, true)
    }
    fn bin(&mut self, op: &str, a: &Word, b: &Word) -> Result<Word> {
        let n = a.len();
        Ok(match op {
            "add" => self.add(a, b, false),
            "sub" => self.add(a, b, true),
            "and" => a.iter().zip(b).map(|(a, b)| self.and(a, b)).collect(),
            "or" => a.iter().zip(b).map(|(a, b)| self.or(a, b)).collect(),
            "xor" => a.iter().zip(b).map(|(a, b)| self.xor(a, b)).collect(),
            "mul" => {
                let mut v = constant(0, n);
                for (i, bit) in b.iter().enumerate() {
                    let mut row = constant(0, n);
                    for j in i..n {
                        row[j] = self.and(&a[j - i], bit)
                    }
                    v = self.add(&v, &row, false)
                }
                v
            }
            "shl" | "lshr" | "ashr" => {
                let mut v = a.clone();
                for (i, bit) in b.iter().enumerate() {
                    let shift = 1usize.checked_shl(i as u32).unwrap_or(usize::MAX);
                    let fill = if op == "ashr" {
                        v[n - 1].clone()
                    } else {
                        Bit::C(false)
                    };
                    let shifted = (0..n)
                        .map(|j| {
                            if op == "shl" {
                                if j >= shift {
                                    v[j - shift].clone()
                                } else {
                                    Bit::C(false)
                                }
                            } else {
                                j.checked_add(shift)
                                    .filter(|k| *k < n)
                                    .map_or(fill.clone(), |k| v[k].clone())
                            }
                        })
                        .collect();
                    v = self.muxw(bit, &shifted, &v);
                } // ISA shifts >= N are zero, including ashr.
                let big = not(&self.ult(b, &constant(n as u128, n)));
                self.muxw(&big, &constant(0, n), &v)
            }
            "udiv" | "urem" | "sdiv" | "srem" => {
                let signed = op.starts_with('s');
                let aa = if signed {
                    let neg = self.neg(a);
                    self.muxw(&a[n - 1], &neg, a)
                } else {
                    a.clone()
                };
                let bb = if signed {
                    let neg = self.neg(b);
                    self.muxw(&b[n - 1], &neg, b)
                } else {
                    b.clone()
                };
                let mut rem = constant(0, n + 1);
                let mut quo = constant(0, n);
                for i in (0..n).rev() {
                    rem.rotate_right(1);
                    rem[0] = aa[i].clone();
                    let ge = not(&self.ult(&rem, &resize(&bb, n + 1)));
                    let diff = self.add(&rem, &resize(&bb, n + 1), true);
                    rem = self.muxw(&ge, &diff, &rem);
                    quo[i] = ge;
                }
                let mut v = if op.ends_with("rem") {
                    resize(&rem, n)
                } else {
                    quo
                };
                if signed {
                    let sign = if op == "srem" {
                        a[n - 1].clone()
                    } else {
                        self.xor(&a[n - 1], &b[n - 1])
                    };
                    let neg = self.neg(&v);
                    v = self.muxw(&sign, &neg, &v);
                }
                v
            }
            _ => bail!("unimplemented: lift.{op} (milestone 2)"),
        })
    }
    fn copy(&mut self, src: &Word, dst: &Word, cs: &[Bit]) {
        assert_eq!(src.len(), dst.len());
        for (s, t) in src.iter().zip(dst) {
            let mut c = cs.to_vec();
            c.push(s.clone());
            self.flip(t, &c);
        }
    }
    fn exchange(&mut self, a: &Word, b: &Word, cs: &[Bit]) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            if a == b {
                continue;
            }
            self.copy(&vec![a.clone()], &vec![b.clone()], cs);
            self.copy(&vec![b.clone()], &vec![a.clone()], cs);
            self.copy(&vec![a.clone()], &vec![b.clone()], cs);
        }
    }
    fn assign(&mut self, values: Vec<(Word, Word)>, cs: &[Bit]) {
        let slots = values
            .iter()
            .map(|(v, _)| {
                let slot = self.alloc(v.len(), "disp");
                self.copy(v, &slot, cs);
                slot
            })
            .collect::<Vec<_>>();
        for ((_, d), s) in values.iter().zip(slots) {
            self.exchange(&s, d, cs);
        }
    }
    fn equal_xor(&mut self, a: &Word, n: usize, t: &Bit) {
        let cs = a
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if i < usize::BITS as usize && n >> i & 1 != 0 {
                    b.clone()
                } else {
                    not(b)
                }
            })
            .collect::<Vec<_>>();
        self.flip(t, &cs);
    }
}
struct Machine<'a> {
    obj: &'a Object,
    root: &'a Function,
    funcs: Vec<&'a Function>,
    rows: Vec<(usize, &'a Instr)>,
    labels: BTreeMap<(String, String), usize>,
    loc: BTreeMap<(String, String), Word>,
    pseudo: BTreeMap<String, Word>,
    args: BTreeMap<String, Argument>,
    callees: Vec<Circuit>,
    pc: Word,
    pc2: Word,
    faulted: Word,
    code: Word,
    result: Word,
    b: Build,
    budget: Budget,
    mem: Vec<Word>,
    mem_addresses: Vec<usize>,
    table: Vec<(Word, Word, Word, Word)>,
    stack: Vec<Word>,
    heap: Word,
    atop: Word,
    sp: Word,
    fp: Word,
    depth: Word,
    ids: Vec<Word>,
    widths: Vec<usize>,
    p: usize,
    cw: usize,
    report_steps: bool,
    probes: Vec<String>,
}
impl<'a> Machine<'a> {
    fn read(&self, f: usize, op: &Operand) -> Result<Word> {
        match op {
            Operand::Name(n) => self
                .loc
                .get(&(self.funcs[f].name.clone(), n.clone()))
                .or_else(|| if f == 0 { self.pseudo.get(n) } else { None })
                .cloned()
                .ok_or_else(|| {
                    Error(format!(
                        "oracle: @{} unknown location {n}",
                        self.funcs[f].name
                    ))
                }),
            Operand::Int(s) => literal(s, self.obj.header.ptr_width as usize),
            _ => bail!("oracle: invalid read {op:?}"),
        }
    }
    fn dst(&self, f: usize, n: &str) -> Result<Word> {
        self.loc
            .get(&(self.funcs[f].name.clone(), n.into()))
            .cloned()
            .ok_or_else(|| Error(format!("oracle: invalid destination {n}")))
    }
    fn addr(&self, f: usize, op: &Operand) -> Result<u64> {
        let Operand::Sym(n) = op else {
            bail!("oracle: expected symbol address")
        };
        let l = self
            .obj
            .layout
            .as_ref()
            .ok_or_else(|| Error("oracle: address without layout (§7.2)".into()))?;
        if let Some(v) = l.get("globals").and_then(|v| v.get(n)) {
            return number(v, "base");
        }
        if f == 0
            && let Some(v) = l
                .get("interface_regions")
                .and_then(|v| v.get(&self.root.name))
                .and_then(|v| v.get(n))
        {
            return number(v, "base");
        }
        bail!("oracle: invalid address @{n} (§4.2)")
    }
    fn inrange(&mut self, p: &Word, n: &Word) -> Bit {
        let w = p.len().max(n.len()) + 1;
        let end = self.b.add(&resize(p, w), &resize(n, w), false);
        let mut ok = Bit::C(false);
        for (i, (base, size, live, _)) in self.table.iter().enumerate() {
            let idx = self
                .b
                .ult(&constant(i as u128, self.atop.len()), &self.atop);
            let lo = not(&self.b.ult(p, base));
            let top = self.b.add(&resize(base, w), &resize(size, w), false);
            let hi = not(&self.b.ult(&top, &end));
            let a = self.b.and(&idx, &live[0]);
            let a = self.b.and(&a, &lo);
            let a = self.b.and(&a, &hi);
            ok = self.b.or(&ok, &a);
        }
        ok
    }
    fn array_read(&mut self, arr: &[Word], idx: &Word, w: usize) -> Word {
        let mut out = constant(0, w);
        for (i, cell) in arr.iter().enumerate() {
            let at = self.b.eq(idx, &constant(i as u128, idx.len()));
            for (j, value) in out.iter_mut().enumerate() {
                let bit = self.b.and(&at, cell.get(j).unwrap_or(&Bit::C(false)));
                *value = self.b.xor(value, &bit);
            }
        }
        out
    }
    fn array_xor(&mut self, arr: &[Word], idx: &Word, v: &Word, cs: &[Bit]) {
        for (i, cell) in arr.iter().enumerate() {
            let at = self.b.eq(idx, &constant(i as u128, idx.len()));
            let mut cs = cs.to_vec();
            cs.push(at);
            self.b.copy(v, &cell[..v.len()].to_vec(), &cs);
        }
    }
    fn array_exchange(&mut self, arr: &[Word], idx: &Word, v: &Word, cs: &[Bit]) {
        for (i, cell) in arr.iter().enumerate() {
            let at = self.b.eq(idx, &constant(i as u128, idx.len()));
            let mut cs = cs.to_vec();
            cs.push(at);
            self.b.exchange(v, &cell[..v.len()].to_vec(), &cs);
        }
    }
    fn memory_read(&mut self, idx: &Word) -> Word {
        let mut out = constant(0, 8);
        for (addr, cell) in self.mem_addresses.iter().zip(&self.mem) {
            let at = self.b.eq(idx, &constant(*addr as u128, idx.len()));
            for j in 0..8 {
                let v = self.b.and(&at, &cell[j]);
                out[j] = self.b.xor(&out[j], &v);
            }
        }
        out
    }
    fn memory_exchange(&mut self, idx: &Word, v: &Word, cs: &[Bit]) {
        for (addr, cell) in self.mem_addresses.iter().zip(&self.mem) {
            let at = self.b.eq(idx, &constant(*addr as u128, idx.len()));
            let mut controls = cs.to_vec();
            controls.push(at);
            self.b.exchange(v, cell, &controls);
        }
    }
    fn offset(&mut self, base: &Word, k: usize) -> Word {
        self.b.add(base, &constant(k as u128, base.len()), false)
    }
    fn label(&self, f: usize, op: &Operand) -> Result<usize> {
        let Operand::Label(n, None) = op else {
            bail!("oracle: invalid label {op:?}")
        };
        self.labels
            .get(&(self.funcs[f].name.clone(), n.clone()))
            .copied()
            .ok_or_else(|| Error(format!("oracle: missing label ^{n}")))
    }
    fn row(&mut self, k: usize, at: &Bit, flag: &Bit, succ: &Word) -> Result<Word> {
        let (fi, i) = self.rows[k];
        let op = i.opcode.strip_prefix("lift.").unwrap_or(&i.opcode);
        let operands = &i.operands;
        let mut next = constant((k + 1) as u128, self.p);
        let halt = constant(self.rows.len() as u128, self.p);
        let mut bad = Bit::C(false);
        let mut recorded = false;
        macro_rules! capture_flag {
            () => {
                self.b.flip(flag, &[at.clone(), bad.clone()]);
                recorded = true;
            };
        }

        let mut code = constant(0, self.code.len());
        let mut values = Vec::new();
        let d = if let Some(n) = i.lhs.first() {
            Some(self.dst(fi, n)?)
        } else {
            None
        };
        let w = d.as_ref().map_or(0, Vec::len);
        match op {
            "perm_call" => {
                let Operand::Sym(name) = &operands[0] else {
                    bail!("oracle: perm_call needs symbol")
                };

                let mut actuals = Vec::new();
                for operand in &operands[1..3] {
                    let Operand::List(names) = operand else {
                        bail!("oracle: perm_call needs lists")
                    };
                    for n in names {
                        actuals.push(self.read(fi, &Operand::Name(n.clone()))?);
                    }
                }
                let Operand::List(mems) = &operands[3] else {
                    bail!("oracle: perm_call memory list")
                };
                for n in mems {
                    let param = self.root.groups[3]
                        .iter()
                        .find(|p| p.name == *n)
                        .ok_or_else(|| Error("oracle: invalid perm_call memory region".into()))?;
                    let Type::Region(a, w) = param.ty else {
                        unreachable!()
                    };
                    actuals.push(refs(n, ((1u64 << a) * u64::from(w)) as u32));
                }
                let Operand::List(args) = &operands[4] else {
                    bail!("oracle: perm_call argument list")
                };
                let arguments =
                    args.iter()
                        .map(|n| {
                            self.args.get(n).cloned().ok_or_else(|| {
                                Error(format!("oracle: unknown classical argument {n}"))
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                let program = crate::lower::lower_program_with_args(self.obj, name, &arguments)?;
                self.inline_program(&program, &actuals, std::slice::from_ref(at))?;
            }
            "oracle_call" | "oracle_uncall" => {
                let Operand::Sym(name) = &operands[0] else {
                    bail!("oracle: nested call needs symbol")
                };
                let Operand::Name(token) = &operands[1] else {
                    bail!("oracle: nested call needs token")
                };
                let Some(Argument::Token(budget)) = self.args.get(token) else {
                    bail!("oracle: missing budget token {token}")
                };
                let cf = self.obj.function(name).ok_or_else(|| {
                    Error(format!(
                        "unimplemented: imported oracle @{name} (milestone 2)"
                    ))
                })?;
                let Operand::List(actual_args) = &operands[6] else {
                    bail!("oracle: call args list")
                };
                let arguments = actual_args
                    .iter()
                    .map(|n| {
                        self.args
                            .get(n)
                            .cloned()
                            .ok_or_else(|| Error(format!("oracle: missing classical argument {n}")))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let program = specialise(self.obj, name, *budget, &arguments)?;
                let Operand::List(ins) = &operands[2] else {
                    bail!("oracle: input list")
                };
                let Operand::List(mems) = &operands[5] else {
                    bail!("oracle: memory list")
                };
                let mut actuals = vec![self.read(fi, &operands[3])?, self.read(fi, &operands[4])?];
                for n in ins {
                    actuals.push(self.read(fi, &Operand::Name(n.clone()))?);
                }
                for n in mems {
                    let param = self.root.groups[3]
                        .iter()
                        .find(|p| p.name == *n)
                        .ok_or_else(|| Error("oracle: invalid nested memory region".into()))?;
                    let Type::Region(a, w) = param.ty else {
                        unreachable!()
                    };
                    actuals.push(refs(n, ((1u64 << a) * u64::from(w)) as u32));
                }
                let _ = cf;
                self.inline_program(&program, &actuals, std::slice::from_ref(at))?;
            }
            "mov" => values.push((self.read(fi, &operands[0])?, d.clone().unwrap())),
            "parmov" => {
                let Operand::List(src) = &operands[0] else {
                    bail!("oracle: parmov needs list")
                };
                for (s, d) in src.iter().zip(&i.lhs) {
                    values.push((self.read(fi, &Operand::Name(s.clone()))?, self.dst(fi, d)?));
                }
            }
            "label" => values.push((
                constant(self.label(fi, &operands[0])? as u128, w),
                d.clone().unwrap(),
            )),
            "addr" => values.push((
                constant(self.addr(fi, &operands[0])? as u128, w),
                d.clone().unwrap(),
            )),
            "select" => {
                let p = self.read(fi, &operands[0])?;
                let a = self.read(fi, &operands[1])?;
                let bb = self.read(fi, &operands[2])?;
                values.push((self.b.muxw(&p[0], &a, &bb), d.clone().unwrap()));
            }
            "qjump" => next = constant(self.label(fi, &operands[0])? as u128, self.p),
            "qbranch" => {
                let p = self.read(fi, &operands[0])?;
                next = self.b.muxw(
                    &p[0],
                    &constant(self.label(fi, &operands[1])? as u128, self.p),
                    &constant(self.label(fi, &operands[2])? as u128, self.p),
                );
            }
            "qjumpr" => {
                let a = self.read(fi, &operands[0])?;
                let first = self.rows.iter().position(|(f, _)| *f == fi).unwrap();
                let end = self.rows.iter().rposition(|(f, _)| *f == fi).unwrap() + 1;
                let lo = self.b.ult(&a, &constant(first as u128, a.len()));
                let hi = not(&self.b.ult(&a, &constant(end as u128, a.len())));
                bad = self.b.or(&lo, &hi);
                code = constant(4, self.code.len());
                next = resize(&a, self.p);
                self.b.copy(&next, succ, std::slice::from_ref(at));
                next = succ.clone();
            }
            "fault" => {
                bad = Bit::C(true);
                let Operand::Int(c) = &operands[0] else {
                    bail!("oracle: fault code must be literal")
                };
                code = literal(c, self.code.len())?;
            }
            "faultif" => {
                bad = self.read(fi, &operands[0])?[0].clone();
                let Operand::Int(c) = &operands[1] else {
                    bail!("oracle: faultif code must be literal")
                };
                code = literal(c, self.code.len())?;
            }
            "retval" => {
                let v = self.read(fi, &operands[0])?;
                self.b.copy(&v, &self.result, std::slice::from_ref(at));
                next = halt.clone();
            }
            "boundscheck" => {
                let p = self.read(fi, &operands[0])?;
                let n = self.read(fi, &operands[1])?;
                bad = not(&self.inrange(&p, &n));
                code = constant(7, self.code.len());
            }
            "alloc" => {
                let n = self.read(fi, &operands[0])?;
                let cw = n.len().max(self.heap.len()) + 1;
                let z = self.b.eq(&n, &constant(0, n.len()));
                let size = self.b.muxw(&z, &constant(1, cw), &resize(&n, cw));
                let used = self.b.add(&resize(&self.heap, cw), &size, false);
                let heap_bad = self.b.ult(&constant(self.budget.heap as u128, cw), &used);
                let tab_bad = not(&self.b.ult(
                    &self.atop,
                    &constant(self.table.len() as u128, self.atop.len()),
                ));
                bad = self.b.or(&heap_bad, &tab_bad);
                code = constant(2, self.code.len());
                capture_flag!();
                let cs = vec![at.clone(), not(&bad)];
                let base = number(self.obj.layout.as_ref().unwrap(), "heap_base")?;
                let addr = self
                    .b
                    .add(&resize(&self.heap, w), &constant(base as u128, w), false);
                let wslot = self.b.alloc(w, "alloc_disp_w");
                let entry = self
                    .b
                    .alloc(2 * self.obj.header.ptr_width as usize + 2, "alloc_entry");
                let mut entry_value = resize(&addr, self.obj.header.ptr_width as usize);
                entry_value.extend(resize(&n, self.obj.header.ptr_width as usize));
                entry_value.push(Bit::C(true));
                entry_value.push(Bit::C(false));
                self.b.copy(&entry_value, &entry, &cs);
                let heap_slot = self.b.alloc(self.heap.len(), "alloc_disp_heap");
                let atop_slot = self.b.alloc(self.atop.len(), "alloc_disp_atop");
                let inc = self.b.add(&self.atop, &constant(1, self.atop.len()), false);
                self.b.copy(&addr, &wslot, &cs);
                self.b
                    .copy(&resize(&used, self.heap.len()), &heap_slot, &cs);
                self.b.copy(&inc, &atop_slot, &cs);
                self.b.exchange(&wslot, &d.clone().unwrap(), &cs);
                let table = self.table.clone();
                for (j, (base, size, live, pinned)) in table.iter().enumerate() {
                    let atj = self.b.eq(&self.atop, &constant(j as u128, self.atop.len()));
                    let mut controls = cs.clone();
                    controls.push(atj);
                    let cell = [base.clone(), size.clone(), live.clone(), pinned.clone()].concat();
                    self.b.exchange(&entry, &cell, &controls);
                }
                self.b.exchange(&heap_slot, &self.heap, &cs);
                self.b.exchange(&atop_slot, &self.atop, &cs);
            }
            "free" => {
                let p = self.read(fi, &operands[0])?;
                let mut found = Bit::C(false);
                let mut matches = Vec::new();
                for (j, (base, _, live, pinned)) in self.table.iter().enumerate() {
                    let ix = self
                        .b
                        .ult(&constant(j as u128, self.atop.len()), &self.atop);
                    let eq = self.b.eq(&p, base);
                    let m = self.b.and(&ix, &eq);
                    let a = self.b.and(&m, &live[0]);
                    let a = self.b.and(&a, &not(&pinned[0]));
                    found = self.b.or(&found, &a);
                    matches.push(m);
                }
                bad = not(&found);
                code = constant(7, self.code.len());
                capture_flag!();
                for (m, (_, _, live, _)) in matches.iter().zip(&self.table) {
                    self.b.flip(&live[0], &[at.clone(), not(&bad), m.clone()]);
                }
            }
            "call" => {
                let Operand::Sym(name) = &operands[0] else {
                    bail!("oracle: lift.call needs callee")
                };
                let cf = self
                    .funcs
                    .iter()
                    .position(|f| f.name == *name)
                    .ok_or_else(|| Error(format!("oracle: unresolved lproc @{name}")))?;
                let callee = self.funcs[cf];
                let ls = self.locations(cf);
                let size = 3 + ls.len();
                let sp1 = self.b.add(
                    &resize(&self.sp, self.sp.len() + 1),
                    &constant(size as u128, self.sp.len() + 1),
                    false,
                );
                let dep1 = self.b.add(
                    &resize(&self.depth, self.depth.len() + 1),
                    &constant(1, self.depth.len() + 1),
                    false,
                );
                let over_sp = self
                    .b
                    .ult(&constant(self.budget.stack as u128, sp1.len()), &sp1);
                let over_depth = self
                    .b
                    .ult(&constant(self.budget.depth as u128, dep1.len()), &dep1);
                bad = self.b.or(&over_sp, &over_depth);
                code = constant(3, self.code.len());
                capture_flag!();
                let cs = vec![at.clone(), not(&bad)];
                let Operand::List(actuals) = &operands[1] else {
                    bail!("oracle: call args must be list")
                };
                let mut captured = Vec::new();
                for a in actuals {
                    let a = self.read(fi, &Operand::Name(a.clone()))?;
                    let slot = self.b.alloc(a.len(), "capture");
                    self.b.copy(&a, &slot, &cs);
                    captured.push(slot);
                }
                let stack = self.stack.clone();
                for (j, l) in ls.iter().enumerate() {
                    let idx = self.offset(&self.sp.clone(), 3 + j);
                    self.array_exchange(&stack, &idx, l, &cs);
                }
                let ret = constant((k + 1) as u128, self.cw);
                self.array_xor(&stack, &self.sp.clone(), &ret, &cs);
                let idx = self.offset(&self.sp.clone(), 1);
                self.array_xor(&stack, &idx, &resize(&self.fp, self.cw), &cs);
                let id = self.location_id(fi, &i.lhs[0])?;
                let idx = self.offset(&self.sp.clone(), 2);
                self.array_xor(&stack, &idx, &constant(id as u128, self.cw), &cs);
                for (p, a) in callee.groups[0].iter().zip(captured) {
                    self.b.copy(&a, &self.dst(cf, &p.name)?, &cs);
                }
                self.b.assign(
                    vec![
                        (self.sp.clone(), self.fp.clone()),
                        (resize(&sp1, self.sp.len()), self.sp.clone()),
                        (resize(&dep1, self.depth.len()), self.depth.clone()),
                    ],
                    &cs,
                );
                next = constant(
                    self.rows.iter().position(|(f, _)| *f == cf).unwrap() as u128,
                    self.p,
                );
            }
            "ret" => {
                bad = self.b.eq(&self.depth, &constant(0, self.depth.len()));
                code = constant(4, self.code.len());
                capture_flag!();
                let cs = vec![at.clone(), not(&bad)];
                let stack = self.stack.clone();
                next = resize(&self.array_read(&stack, &self.fp.clone(), self.cw), self.p);
                self.b.copy(&next, succ, std::slice::from_ref(at));
                next = succ.clone();
                let v = self.read(fi, &operands[0])?;
                let slot = self.b.alloc(v.len(), "delivery");
                self.b.copy(&v, &slot, &cs);
                for (j, l) in self.locations(fi).iter().enumerate() {
                    let idx = self.offset(&self.fp.clone(), 3 + j);
                    self.array_exchange(&stack, &idx, l, &cs);
                }
                let idx = self.offset(&self.fp.clone(), 2);
                let id = self.array_read(&stack, &idx, self.cw);
                for (j, l) in self.ids.clone().iter().enumerate() {
                    if self.widths[j] != v.len() {
                        continue;
                    }
                    let eq = self.b.eq(&id, &constant(j as u128, self.cw));
                    self.b.exchange(&slot, l, &[at.clone(), not(&bad), eq]);
                }
                let idx = self.offset(&self.fp.clone(), 1);
                let old_fp = self.array_read(&stack, &idx, self.cw);
                let depth = self
                    .b
                    .add(&self.depth, &constant(1, self.depth.len()), true);
                self.b.assign(
                    vec![
                        (resize(&old_fp, self.fp.len()), self.fp.clone()),
                        (depth, self.depth.clone()),
                    ],
                    &cs,
                );
            }
            _ if op.starts_with("load.i") || op.starts_with("store.i") => {
                let n = op
                    .split_once(".i")
                    .unwrap()
                    .1
                    .parse::<usize>()
                    .map_err(|_| Error("oracle: invalid memory width".into()))?;
                let p = self.read(fi, &operands[0])?;
                bad = not(&self.inrange(&p, &constant((n / 8) as u128, p.len())));
                code = constant(7, self.code.len());
                capture_flag!();
                let cs = vec![at.clone(), not(&bad)];
                if op.starts_with("load") {
                    let mut value = Vec::new();
                    for j in 0..n / 8 {
                        let idx = self.offset(&p, j);
                        value.extend(self.memory_read(&idx));
                    }
                    self.b.assign(vec![(value, d.clone().unwrap())], &cs);
                } else {
                    let v = self.read(fi, &operands[1])?;
                    let slot = self.b.alloc(n, "store_disp");
                    self.b.copy(&v, &slot, &cs);
                    for j in 0..n / 8 {
                        let idx = self.offset(&p, j);
                        self.memory_exchange(&idx, &slot[j * 8..j * 8 + 8].to_vec(), &cs);
                    }
                }
            }
            _ if op.starts_with("const.i") => {
                let Operand::Int(c) = &operands[0] else {
                    bail!("oracle: const must be integer")
                };
                values.push((literal(c, w)?, d.clone().unwrap()));
            }
            _ if op.starts_with("zext.i")
                || op.starts_with("sext.i")
                || op.starts_with("trunc.i") =>
            {
                let mut a = self.read(fi, &operands[0])?;
                if op.starts_with("sext") {
                    a.resize(w, a.last().unwrap().clone())
                }
                values.push((resize(&a, w), d.clone().unwrap()));
            }
            _ if op.starts_with("icmp.") => {
                let a = self.read(fi, &operands[0])?;
                let bb = self.read(fi, &operands[1])?;
                let pred = &op[5..];
                let eq = self.b.eq(&a, &bb);
                let lt = if pred.starts_with('s') {
                    let signs = self.b.xor(a.last().unwrap(), bb.last().unwrap());
                    let ul = self.b.ult(&a, &bb);
                    self.b.mux(&signs, a.last().unwrap(), &ul)
                } else {
                    self.b.ult(&a, &bb)
                };
                let v = match pred {
                    "eq" => eq,
                    "ne" => not(&eq),
                    "ult" | "slt" => lt,
                    "ule" | "sle" => self.b.or(&lt, &eq),
                    _ => bail!("oracle: invalid icmp predicate {pred}"),
                };
                values.push((vec![v], d.clone().unwrap()));
            }
            "add" | "sub" | "mul" | "and" | "or" | "xor" | "shl" | "lshr" | "ashr" | "udiv"
            | "urem" | "sdiv" | "srem" => {
                let a = self.read(fi, &operands[0])?;
                let bb = self.read(fi, &operands[1])?;
                if op.ends_with("div") || op.ends_with("rem") {
                    bad = self.b.eq(&bb, &constant(0, bb.len()));
                    code = constant(5, self.code.len());
                    if op.starts_with('s') {
                        let mut min = constant(0, a.len());
                        *min.last_mut().unwrap() = Bit::C(true);
                        let x = self.b.eq(&a, &min);
                        let y = self.b.eq(&bb, &vec![Bit::C(true); bb.len()]);
                        let overflow = self.b.and(&x, &y);
                        bad = self.b.or(&bad, &overflow);
                        code = self.b.muxw(&overflow, &constant(6, self.code.len()), &code);
                    }
                }
                let value = self.b.bin(op, &a, &bb)?;
                values.push((value, d.clone().unwrap()));
            }
            _ => bail!("unimplemented: {} (milestone 2)", i.opcode),
        }
        if !recorded {
            self.b.flip(flag, &[at.clone(), bad.clone()]);
        }
        if !values.is_empty() {
            self.b.assign(values, &[at.clone(), not(&bad)]);
        }
        let successor = self.b.muxw(flag, &halt, &next);
        self.b.copy(&code, &self.code, &[at.clone(), flag.clone()]);
        self.b.flip(&self.faulted[0], &[at.clone(), flag.clone()]);
        Ok(successor)
    }
    fn inline_program(&mut self, program: &Program, actuals: &[Word], cs: &[Bit]) -> Result<()> {
        // Preserve each specialised oracle as a subcircuit. Capturing aliases
        // in dedicated interface bundles is unnecessary: the call binds bundles.
        let prefix = format!("$oracle{}", self.b.serial);
        self.b.serial += 1;
        let mut circuits = program.circuits.clone();
        for c in &mut circuits {
            c.name = format!("{prefix}{}", c.name);
            for n in &mut c.body {
                if let Node::Call { callee, .. } = n {
                    *callee = format!("{prefix}{callee}");
                }
            }
        }
        let mut args = Vec::new();
        for word in actuals {
            let Some(Bit::Q(q, true)) = word.first() else {
                bail!("unimplemented: non-location nested oracle operand (milestone 2)")
            };
            if word
                .iter()
                .enumerate()
                .any(|(i, b)| *b != Bit::Q(QubitRef::new(&q.bundle, i as u32), true))
            {
                bail!("oracle: call operand is not a whole bundle")
            };
            args.push(q.bundle.clone());
        }
        let controls = cs
            .iter()
            .filter_map(|b| match b {
                Bit::C(true) => None,
                Bit::C(false) => None,
                Bit::Q(q, p) => Some(Ctrl {
                    qubit: q.clone(),
                    positive: *p,
                }),
            })
            .collect();
        if !cs.contains(&Bit::C(false)) {
            self.b.c.body.push(Node::Call {
                callee: format!("{prefix}{}", program.top),
                adjoint: false,
                controls,
                args,
            });
        }
        self.callees.extend(circuits);
        Ok(())
    }
    fn locations(&self, f: usize) -> Vec<Word> {
        let f = self.funcs[f];
        let names = if f.kind == FnKind::Lproc {
            f.groups[0]
                .iter()
                .chain(&f.groups[1])
                .map(|p| p.name.clone())
                .chain(f.regs.iter().map(|p| p.name.clone()))
                .collect::<Vec<_>>()
        } else {
            f.regs.iter().map(|p| p.name.clone()).collect()
        };
        names
            .iter()
            .map(|n| self.loc[&(f.name.clone(), n.clone())].clone())
            .collect()
    }
    fn location_id(&self, f: usize, n: &str) -> Result<usize> {
        let word = self.dst(f, n)?;
        self.ids
            .iter()
            .position(|w| *w == word)
            .ok_or_else(|| Error("oracle: location id absent".into()))
    }
    fn possible_successors(&self, k: usize) -> Result<Vec<usize>> {
        let n = self.rows.len();
        if k == n {
            return Ok(vec![n]);
        }
        let (fi, i) = self.rows[k];
        let op = i.opcode.strip_prefix("lift.").unwrap_or(&i.opcode);
        Ok(match op {
            "qjump" => vec![self.label(fi, &i.operands[0])?],
            "qbranch" => vec![
                self.label(fi, &i.operands[1])?,
                self.label(fi, &i.operands[2])?,
            ],
            "qjumpr" => self
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.0 == fi)
                .map(|(j, _)| j)
                .chain(std::iter::once(n))
                .collect(),
            "call" => {
                let Operand::Sym(name) = &i.operands[0] else {
                    bail!("oracle: call symbol")
                };
                vec![
                    self.rows
                        .iter()
                        .position(|(f, _)| self.funcs[*f].name == *name)
                        .unwrap(),
                    n,
                ]
            }
            "ret" => (0..=n).collect(),
            "retval" | "fault" => vec![n],
            _ => vec![k + 1, n],
        })
    }
    fn forward(&mut self) -> Result<()> {
        let n = self.rows.len();
        let mut possible = BTreeSet::from([0]);
        for _ in 0..self.budget.steps {
            let dispatch = self.b.alloc(n + 1, "dispatch");
            for k in &possible {
                self.b.equal_xor(&self.pc, *k, &dispatch[*k]);
            }
            if self.report_steps {
                let name = format!("$executed_{}", self.probes.len());
                self.b.c.mut_bundles.push(Bundle {
                    name: name.clone(),
                    width: 1,
                    role: Role::Mut,
                });
                let controls = if possible.contains(&n) {
                    vec![not(&dispatch[n])]
                } else {
                    Vec::new()
                };
                self.b.flip(&refs(&name, 1)[0], &controls);
                self.probes.push(name);
            }
            let old = self.b.alloc(self.p, "old_pc");
            let flag = self.b.alloc(1, "flag");
            let succ = self.b.alloc(self.p, "succ");
            let mut successors = Vec::new();
            for k in possible.iter().copied().filter(|k| *k < n) {
                successors.push((k, self.row(k, &dispatch[k], &flag[0], &succ)?));
            }
            for (k, next) in successors {
                self.b
                    .copy(&next, &self.pc2, std::slice::from_ref(&dispatch[k]));
            }
            if possible.contains(&n) {
                self.b.copy(
                    &constant(n as u128, self.p),
                    &self.pc2,
                    &[dispatch[n].clone()],
                );
            }
            self.b.exchange(&self.pc, &self.pc2, &[]);
            self.b.exchange(&self.pc2, &old, &[]);
            let mut next = BTreeSet::new();
            for k in &possible {
                self.b.equal_xor(&old, *k, &dispatch[*k]);
                next.extend(self.possible_successors(*k)?);
            }
            possible = next;
        }
        Ok(())
    }
}
/// Specialise one oracle with fixed classical arguments and budget. Interface
/// order is target, fault, inputs, memory; all of D remains quantum input.
pub fn specialise(obj: &Object, name: &str, budget: Budget, args: &[Argument]) -> Result<Program> {
    Ok(specialise_impl(obj, name, budget, args, false)?.0)
}
/// Diagnostic execution includes one output bit per active forward row. These
/// observer bits are never part of an exported oracle's interface.
pub(crate) fn specialise_probe(
    obj: &Object,
    name: &str,
    budget: Budget,
    args: &[Argument],
) -> Result<(Program, usize)> {
    specialise_impl(obj, name, budget, args, true)
}
fn specialise_impl(
    obj: &Object,
    name: &str,
    budget: Budget,
    args: &[Argument],
    report_steps: bool,
) -> Result<(Program, usize)> {
    crate::checker::region_graph(obj)?;
    let root = obj
        .function(name)
        .ok_or_else(|| Error(format!("oracle: unknown @{name}")))?;
    if root.kind != FnKind::Oracle {
        bail!("oracle: @{name} is not an oracle")
    }
    if root.groups[4].len() != args.len() {
        bail!("oracle: @{name} classical argument arity mismatch")
    }
    let mut c = Circuit {
        name: name.into(),
        perm: true,
        mut_bundles: Vec::new(),
        in_bundles: Vec::new(),
        mem: Vec::new(),
        workspace: Vec::new(),
        local_regions: Vec::new(),
        body: Vec::new(),
    };
    for p in root.groups[1].iter().chain(&root.groups[2]) {
        c.mut_bundles.push(Bundle {
            name: p.name.clone(),
            width: p
                .ty
                .wire_width()
                .ok_or_else(|| Error("oracle: invalid output width".into()))?,
            role: Role::Mut,
        })
    }
    for p in &root.groups[0] {
        c.in_bundles.push(Bundle {
            name: p.name.clone(),
            width: p.ty.wire_width().unwrap(),
            role: Role::In,
        });
    }
    for p in &root.groups[3] {
        let Type::Region(a, w) = p.ty else {
            bail!("oracle: invalid memory interface")
        };
        c.mem.push(MemBundle {
            name: p.name.clone(),
            addr_width: a,
            cell_width: w,
            role: Role::In,
        });
    }
    let mut b = Build { c, serial: 0 };
    let hashes = obj.identity()?;
    let mut reachable = BTreeSet::new();
    let mut queue = vec![root];
    while let Some(f) = queue.pop() {
        for i in f.blocks.iter().flat_map(|b| &b.instrs) {
            if i.opcode == "lift.call" {
                let Operand::Sym(n) = &i.operands[0] else {
                    bail!("oracle: invalid call")
                };
                if reachable.insert(n.clone()) {
                    queue.push(
                        obj.function(n)
                            .ok_or_else(|| Error(format!("oracle: unresolved @{n}")))?,
                    );
                }
            }
        }
    }
    let mut procs = reachable
        .into_iter()
        .map(|n| obj.function(&n).unwrap())
        .collect::<Vec<_>>();
    let refs_of = procs
        .iter()
        .map(|f| {
            (
                f.name.clone(),
                f.blocks
                    .iter()
                    .flat_map(|b| &b.instrs)
                    .filter(|i| i.opcode == "lift.call")
                    .filter_map(|i| {
                        if let Operand::Sym(n) = &i.operands[0] {
                            Some(n.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let defined = procs
        .iter()
        .map(|f| f.name.as_str())
        .collect::<BTreeSet<_>>();
    let groups = crate::object::strongly_connected(&refs_of, &defined);
    let mut keys = BTreeMap::new();
    for group in groups {
        let cyclic = group.len() > 1 || refs_of[&group[0]].contains(&group[0]);
        let key = if cyclic {
            let mut bytes = Vec::new();
            for n in &group {
                bytes.extend(obj.semantic_envelope(n)?);
            }
            crate::sha256::sha256_hex(&bytes)
        } else {
            hashes[&group[0]]
                .closure_hash
                .clone()
                .ok_or_else(|| Error("oracle: unresolved lproc closure".into()))?
        };
        for (index, n) in group.iter().enumerate() {
            keys.insert(n.clone(), (key.clone(), index));
        }
    }
    procs.sort_by_key(|f| keys[&f.name].clone());
    let mut funcs = vec![root];
    funcs.extend(procs);
    let mut rows = Vec::new();
    let mut labels = BTreeMap::new();
    for (fi, f) in funcs.iter().enumerate() {
        for block in &f.blocks {
            labels.insert((f.name.clone(), block.name.clone()), rows.len());
            for i in &block.instrs {
                rows.push((fi, i));
            }
        }
    }
    let p = (usize::BITS - rows.len().leading_zeros()).max(1) as usize;
    let pc = b.alloc(p, "pc");
    let pc2 = b.alloc(p, "pc_prime");
    let faulted = b.alloc(1, "faulted");
    let code = b.alloc(root.groups[2][0].ty.wire_width().unwrap() as usize, "code");
    let result = b.alloc(
        root.groups[1][0].ty.wire_width().unwrap() as usize,
        "result",
    );
    let mut loc = BTreeMap::new();
    let mut ids = Vec::new();
    let mut widths = Vec::new();
    for f in &funcs {
        let decls = if f.kind == FnKind::Lproc {
            f.groups[0]
                .iter()
                .chain(&f.groups[1])
                .map(|p| (&p.name, &p.ty))
                .chain(f.regs.iter().map(|p| (&p.name, &p.ty)))
                .collect::<Vec<_>>()
        } else {
            f.regs.iter().map(|p| (&p.name, &p.ty)).collect()
        };
        for (n, t) in decls {
            let w = t.wire_width().unwrap() as usize;
            let a = b.alloc(w, "loc");
            ids.push(a.clone());
            widths.push(w);
            loc.insert((f.name.clone(), n.clone()), a);
        }
    }
    let mut pseudo = BTreeMap::new();
    for param in &root.groups[0] {
        pseudo.insert(
            param.name.clone(),
            refs(&param.name, param.ty.wire_width().unwrap()),
        );
    }
    let mut arguments = BTreeMap::new();
    for (p, a) in root.groups[4].iter().zip(args) {
        crate::lower::validate_arg(&p.ty, a)?;
        arguments.insert(p.name.clone(), a.clone());
        if let (Type::Int(w), Argument::Integer(s)) = (&p.ty, a) {
            crate::lower::validate_arg(&p.ty, a)?;
            pseudo.insert(p.name.clone(), literal(s, *w as usize)?);
        } else if let (Type::Ptr, Argument::Integer(s)) = (&p.ty, a) {
            crate::lower::validate_integer(s, obj.header.ptr_width)?;
            pseudo.insert(p.name.clone(), literal(s, obj.header.ptr_width as usize)?);
        } else if !matches!(
            (&p.ty, a),
            (Type::Token, Argument::Token(_)) | (Type::F64, Argument::Float(_))
        ) {
            bail!("oracle: wrong argument type for {}", p.name)
        }
    }
    let mut mem = Vec::new();
    let mut mem_addresses = Vec::new();
    let mut table = Vec::new();
    let mut stack = Vec::new();
    let mut heap = Vec::new();
    let mut atop = Vec::new();
    let mut sp = Vec::new();
    let mut fp = Vec::new();
    let mut depth = Vec::new();
    let ptr = obj.header.ptr_width as usize;
    let mut cw = p
        .max(widths.iter().copied().max().unwrap_or(1))
        .max((usize::BITS - ids.len().leading_zeros()) as usize);
    if let Some(layout) = &obj.layout {
        let h = number(layout, "H")?;
        let at = number(layout, "A_tab")?;
        let sc = number(layout, "S_cells")?;
        if h >= usize::BITS as u64 || at >= usize::BITS as u64 || sc >= usize::BITS as u64 {
            bail!("host limit: oracle array exponent exceeds address space")
        }
        if budget.heap > number(layout, "heap_limit_max")?
            || budget.stack > 1u64.checked_shl(sc as u32).unwrap_or(0)
        {
            bail!("oracle: invalid budget (§5.4)")
        }
        cw = cw.max(sc as usize + 1);
        heap = b.alloc(h as usize + 2, "heap_used");
        atop = b.alloc(at as usize + 1, "atop");
        sp = b.alloc(sc as usize + 1, "sp");
        fp = b.alloc(sc as usize + 1, "fp");
        let dc = (u128::BITS - (u128::from(budget.depth) + 1).leading_zeros()).max(1) as usize;
        depth = b.alloc(dc, "depth");
        for _ in 0..1usize << at {
            table.push((
                b.alloc(ptr, "base"),
                b.alloc(ptr, "size"),
                b.alloc(1, "live"),
                b.alloc(1, "pinned"),
            ));
        }
        for _ in 0..budget.stack as usize {
            stack.push(b.alloc(cw, "stack"));
        }
        let mut pre = Vec::new();
        if let Some(globals) = layout.get("globals").and_then(|v| v.as_object()) {
            for (n, v) in globals {
                let base = number(v, "base")? as usize;
                let size = number(v, "size")? as usize;
                let image = v
                    .get("image")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Error(format!("oracle: global {n} image missing")))?;
                let data = obj
                    .data_entry(image)
                    .ok_or_else(|| Error(format!("oracle: missing image @{image}")))?;
                for (j, byte) in data.bytes.iter().enumerate() {
                    mem_addresses.push(base + j);
                    let cell = b.alloc(8, "byte");
                    b.copy(&constant(*byte as u128, 8), &cell, &[]);
                    mem.push(cell);
                }
                pre.push((base, size));
            }
        }
        if let Some(regions) = layout
            .get("interface_regions")
            .and_then(|v| v.get(name))
            .and_then(|v| v.as_object())
        {
            for (n, v) in regions {
                let base = number(v, "base")? as usize;
                let m =
                    b.c.mem
                        .iter()
                        .find(|m| m.name == *n)
                        .ok_or_else(|| Error("oracle: invalid mapped region".into()))?;
                if m.cell_width != 8 {
                    bail!("oracle: mapped region must have W=8 (§7.2)")
                }
                let cells = m.num_cells() as usize;
                for j in 0..cells {
                    mem_addresses.push(base + j);
                    mem.push(refs(n, (cells * 8) as u32)[j * 8..j * 8 + 8].to_vec());
                }
                pre.push((base, cells));
            }
        }
        let heap_base = number(layout, "heap_base")? as usize;
        for j in 0..budget.heap as usize {
            mem_addresses.push(heap_base + j);
            mem.push(b.alloc(8, "heap_byte"));
        }
        if pre.len() > table.len() {
            bail!("oracle: too many allocator pre-entries (§7.2)")
        }
        for (j, (base, size)) in pre.iter().enumerate() {
            let (ba, sz, li, pi) = &table[j];
            b.copy(&constant(*base as u128, ptr), ba, &[]);
            b.copy(&constant(*size as u128, ptr), sz, &[]);
            b.flip(&li[0], &[]);
            b.flip(&pi[0], &[]);
        }
        b.copy(&constant(pre.len() as u128, atop.len()), &atop, &[]);
    } else if budget.heap != 0 || budget.stack != 0 || budget.depth != 0 {
        bail!("oracle: memoryless budget limits must be zero (§5.4)")
    }
    let mut m = Machine {
        obj,
        root,
        funcs,
        rows,
        labels,
        loc,
        pseudo,
        args: arguments,
        callees: Vec::new(),
        pc,
        pc2,
        faulted,
        code,
        result,
        b,
        budget,
        mem,
        mem_addresses,
        table,
        stack,
        heap,
        atop,
        sp,
        fp,
        depth,
        ids,
        widths,
        p,
        cw,
        report_steps,
        probes: Vec::new(),
    };
    m.forward()?;
    let forward = m.b.c.body.iter().filter(|node|!matches!(node,Node::Gate(g)if g.targets.iter().any(|q|m.probes.contains(&q.bundle)))).cloned().collect::<Vec<_>>();
    let halt = m.b.alloc(1, "commit_halt");
    m.b.equal_xor(&m.pc, m.rows.len(), &halt[0]);
    let tgt = refs(&root.groups[1][0].name, m.result.len() as u32);
    let fr = refs(&root.groups[2][0].name, m.code.len() as u32);
    m.b.copy(&m.result, &tgt, &[halt[0].clone(), not(&m.faulted[0])]);
    m.b.copy(&m.code, &fr, &[m.faulted[0].clone()]);
    m.b.copy(&constant(1, fr.len()), &fr, &[not(&halt[0])]);
    m.b.equal_xor(&m.pc, m.rows.len(), &halt[0]);
    m.b.c.body.extend(forward.iter().rev().map(Node::adjoint));
    let probe_count = m.probes.len();
    Ok((
        Program {
            circuits: {
                let mut c = vec![m.b.c];
                c.extend(m.callees);
                c
            },
            top: name.into(),
        },
        probe_count,
    ))
}
