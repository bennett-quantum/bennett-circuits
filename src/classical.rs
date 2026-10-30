//! Independent CC/R diagnostic executor, ISA §§3.4,7.1,7.3,7.4.
//! History stores deltas, not replay recipes: cancelled segments are never redone.
use crate::{
    Error, Result, bail,
    ir::BaseGate,
    json::Value,
    object::{Function, Object, Operand, Type},
};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq)]
enum V {
    Int(u128, u32),
    Q(u64),
}
impl V {
    fn int(&self) -> Result<u128> {
        match self {
            Self::Int(v, _) => Ok(*v),
            _ => bail!("CC: expected integer (§3.3)"),
        }
    }
    fn width(&self) -> Result<u32> {
        match self {
            Self::Int(_, w) => Ok(*w),
            _ => bail!("CC: expected integer (§3.3)"),
        }
    }
}
fn num(n: impl ToString) -> Value {
    Value::Int(n.to_string())
}
fn obj<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::Object(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn hex(bs: &[u8]) -> String {
    bs.iter().map(|b| format!("{b:02x}")).collect()
}
fn bytes(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        bail!("CC: odd hex length")
    };
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| Error("CC: invalid hex".into())))
        .collect()
}
fn mask(w: u32) -> u128 {
    if w == 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}
fn signed(v: u128, w: u32) -> i128 {
    if w == 128 {
        v as i128
    } else {
        ((v << (128 - w)) as i128) >> (128 - w)
    }
}
#[derive(Clone, Debug, PartialEq)]
struct Handle {
    wires: Vec<usize>,
    parent: Option<u64>,
    viewed: bool,
}
#[derive(Clone, Debug, PartialEq)]
struct State {
    memory: BTreeMap<u128, u8>,
    allocations: BTreeMap<u128, (u128, bool)>,
    bump: u128,
    clock: u64,
    console: Vec<u8>,
    entropy_pos: usize,
    block: BTreeMap<u128, Vec<u8>>,
    tape: Vec<Value>,
    handles: BTreeMap<u64, Handle>,
    next_handle: u64,
    quantum: BTreeMap<Vec<bool>, (f64, f64)>,
    host: Vec<Value>,
}
#[derive(Clone)]
struct Frame {
    pure: bool,
    measured: bool,
    reversed: bool,
    before: Option<State>,
    changes: Vec<(usize, String, Option<V>)>,
}
#[derive(Clone)]
struct Segment {
    name: String,
    args: Vec<V>,
    results: Vec<V>,
    first: usize,
    last: usize,
    reversed: bool,
}
struct Machine<'a> {
    object: &'a Object,
    state: State,
    entry: State,
    envs: Vec<BTreeMap<String, V>>,
    frames: Vec<Frame>,
    segments: Vec<Segment>,
    steps: u64,
    entropy: Vec<u8>,
    replies: Vec<Value>,
    draws: Vec<f64>,
    draw: usize,
    measurements: Vec<Value>,
    fault: Option<u64>,
}
impl<'a> Machine<'a> {
    fn new(object: &'a Object) -> Result<Self> {
        let bump = object
            .layout
            .as_ref()
            .and_then(|l| l.get("heap_base"))
            .and_then(Value::as_u64)
            .unwrap_or(1) as u128;
        let mut state = State {
            memory: BTreeMap::new(),
            allocations: BTreeMap::new(),
            bump,
            clock: 0,
            console: vec![],
            entropy_pos: 0,
            block: BTreeMap::new(),
            tape: vec![],
            handles: BTreeMap::new(),
            next_handle: 0,
            quantum: BTreeMap::from([(vec![], (1., 0.))]),
            host: vec![],
        };
        if let Some(gs) = object
            .layout
            .as_ref()
            .and_then(|l| l.get("globals"))
            .and_then(Value::as_object)
        {
            for g in gs.values() {
                let base = g
                    .get("base")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| Error("CC: global base".into()))?
                    as u128;
                let size = g
                    .get("size")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| Error("CC: global size".into()))?
                    as u128;
                let image = object
                    .data_entry(g.get("image").and_then(Value::as_str).unwrap())
                    .ok_or_else(|| Error("CC: global image".into()))?;
                state.allocations.insert(base, (size, true));
                for (i, b) in image.bytes.iter().enumerate() {
                    state.memory.insert(base + i as u128, *b);
                }
            }
        }
        Ok(Self {
            object,
            state: state.clone(),
            entry: state,
            envs: vec![],
            frames: vec![],
            segments: vec![],
            steps: 0,
            entropy: vec![],
            replies: vec![],
            draws: vec![],
            draw: 0,
            measurements: vec![],
            fault: None,
        })
    }
    fn fault(&mut self, n: u64) -> Result<()> {
        self.fault = Some(n);
        Err(Error(format!("CC/R fault {n}")))
    }
    fn get(&self, a: usize, o: &Operand) -> Result<V> {
        match o {
            Operand::Name(n) => self.envs[a]
                .get(n)
                .cloned()
                .ok_or_else(|| Error(format!("CC: undefined {n} (§7.4)"))),
            Operand::Int(n) => Ok(V::Int(
                n.parse().map_err(|_| Error("CC: integer literal".into()))?,
                self.object.header.ptr_width,
            )),
            _ => bail!("unimplemented: CC operand {o:?} (milestone 2)"),
        }
    }
    fn n(&self, a: usize, o: &Operand) -> Result<u128> {
        self.get(a, o)?.int()
    }
    fn set(&mut self, frame: usize, a: usize, n: &str, v: V) {
        let old = self.envs[a].insert(n.into(), v);
        self.frames[frame].changes.push((a, n.into(), old));
    }
    fn valid(&self, p: u128, n: usize) -> bool {
        self.state.allocations.iter().any(|(b, (s, live))| {
            *live && p >= *b && p.checked_add(n as u128).is_some_and(|e| e <= b + s)
        })
    }
    fn read(&mut self, p: u128, n: usize) -> Result<Vec<u8>> {
        if !self.valid(p, n) {
            self.fault(7)?;
        }
        Ok((0..n)
            .map(|i| *self.state.memory.get(&(p + i as u128)).unwrap_or(&0))
            .collect())
    }
    fn write(&mut self, p: u128, bs: &[u8]) -> Result<()> {
        if !self.valid(p, bs.len()) {
            self.fault(7)?;
        }
        for (i, b) in bs.iter().enumerate() {
            self.state.memory.insert(p + i as u128, *b);
        }
        Ok(())
    }
    fn undo(&mut self, first: usize, last: usize) -> Result<()> {
        if self.frames[first..=last]
            .iter()
            .any(|f| f.measured && !f.reversed)
        {
            bail!("E_NOT_REVERSIBLE: measure frame (§10)")
        };
        for j in (first..=last).rev() {
            if self.frames[j].reversed {
                continue;
            }
            let f = self.frames[j].clone();
            if let Some(s) = f.before {
                self.state = s;
            }
            for (a, n, old) in f.changes.into_iter().rev() {
                if let Some(v) = old {
                    self.envs[a].insert(n, v);
                } else {
                    self.envs[a].remove(&n);
                }
            }
            self.frames[j].reversed = true;
        }
        Ok(())
    }
    fn qid(&self, v: &V) -> Result<u64> {
        match v {
            V::Q(q) if self.state.handles.get(q).is_some_and(|h| !h.viewed) => Ok(*q),
            _ => bail!("CC: invalid qreg handle (§3.3)"),
        }
    }
    fn fresh_wires(&mut self, n: usize) -> Vec<usize> {
        let start = self.state.quantum.keys().next().unwrap().len();
        self.state.quantum = std::mem::take(&mut self.state.quantum)
            .into_iter()
            .map(|(mut b, z)| {
                b.resize(start + n, false);
                (b, z)
            })
            .collect();
        (start..start + n).collect()
    }
    fn handle(&mut self, wires: Vec<usize>, parent: Option<u64>) -> V {
        let id = self.state.next_handle;
        self.state.next_handle += 1;
        self.state.handles.insert(
            id,
            Handle {
                wires,
                parent,
                viewed: false,
            },
        );
        V::Q(id)
    }
    fn apply(&mut self, gate: BaseGate, cs: &[(usize, bool)], ts: &[usize]) {
        let mut out = BTreeMap::<Vec<bool>, (f64, f64)>::new();
        let add = |out: &mut BTreeMap<Vec<bool>, (f64, f64)>, key: Vec<bool>, z: (f64, f64)| {
            let x = out.entry(key).or_default();
            x.0 += z.0;
            x.1 += z.1;
        };
        for (b, z) in &self.state.quantum {
            if !cs.iter().all(|(q, p)| b[*q] == *p) {
                add(&mut out, b.clone(), *z);
                continue;
            }
            if gate == BaseGate::Cz {
                let z = if b[ts[0]] && b[ts[1]] {
                    (-z.0, -z.1)
                } else {
                    *z
                };
                add(&mut out, b.clone(), z);
                continue;
            }
            if gate == BaseGate::X || gate == BaseGate::Swap {
                let mut b = b.clone();
                if gate == BaseGate::X {
                    b[ts[0]] ^= true
                } else {
                    b.swap(ts[0], ts[1]);
                }
                add(&mut out, b, *z);
                continue;
            }
            let i = usize::from(b[ts[0]]);
            let h = std::f64::consts::FRAC_1_SQRT_2;
            let phase = |a: f64| (a.cos(), a.sin());
            let matrix = match gate {
                BaseGate::H => [[(h, 0.), (h, 0.)], [(h, 0.), (-h, 0.)]],
                BaseGate::S
                | BaseGate::Sdg
                | BaseGate::T
                | BaseGate::Tdg
                | BaseGate::Rz(_)
                | BaseGate::Cz => {
                    let (a, c) = match gate {
                        BaseGate::S => (0., std::f64::consts::FRAC_PI_2),
                        BaseGate::Sdg => (0., -std::f64::consts::FRAC_PI_2),
                        BaseGate::T => (0., std::f64::consts::FRAC_PI_4),
                        BaseGate::Tdg => (0., -std::f64::consts::FRAC_PI_4),
                        BaseGate::Rz(t) => (-t / 2., t / 2.),
                        _ => (0., std::f64::consts::PI),
                    };
                    [[phase(a), (0., 0.)], [(0., 0.), phase(c)]]
                }
                BaseGate::Rx(t) => {
                    let (c, s) = ((t / 2.).cos(), (t / 2.).sin());
                    [[(c, 0.), (0., -s)], [(0., -s), (c, 0.)]]
                }
                BaseGate::Ry(t) => {
                    let (c, s) = ((t / 2.).cos(), (t / 2.).sin());
                    [[(c, 0.), (-s, 0.)], [(s, 0.), (c, 0.)]]
                }
                _ => unreachable!(),
            };
            for (j, row) in matrix.iter().enumerate() {
                let mut key = b.clone();
                key[ts[0]] = j != 0;
                let v = row[i];
                add(
                    &mut out,
                    key,
                    (z.0 * v.0 - z.1 * v.1, z.0 * v.1 + z.1 * v.0),
                );
            }
        }
        out.retain(|_, z| *z != (0., 0.));
        self.state.quantum = out;
    }
    fn region(&mut self, a: usize, ops: &[Operand], adj: bool) -> Result<()> {
        let Operand::Sym(name) = &ops[0] else {
            bail!("CC: region symbol")
        };
        let mut args = vec![];
        for op in &ops[1..4] {
            let Operand::List(ns) = op else {
                bail!("CC: region args")
            };
            for n in ns {
                args.push(self.qid(&self.get(a, &Operand::Name(n.clone()))?)?);
            }
        }
        if !matches!(&ops[4],Operand::List(ns)if ns.is_empty()) {
            bail!("unimplemented: CC region classical arguments (milestone 2)")
        }
        let mut program = crate::lower::lower_program(self.object, name)?;
        if adj {
            let i = program
                .circuits
                .iter()
                .position(|c| c.name == program.top)
                .unwrap();
            program.circuits[i] = program.circuits[i].adjoint();
        }
        let flat = crate::flatten::flatten(&program)?;
        let mut map = vec![0; flat.num_qubits];
        let mut wires = vec![];
        for q in &args {
            wires.extend(self.state.handles[q].wires.clone());
        }
        if wires.len() != flat.interface.len() {
            bail!("CC: region interface width")
        };
        for (q, w) in flat.interface.iter().zip(wires) {
            map[*q] = w;
        }
        let fresh = self.fresh_wires(flat.workspace.len());
        for (q, w) in flat.workspace.iter().zip(&fresh) {
            map[*q] = *w;
        }
        for g in &flat.gates {
            let cs = g
                .controls
                .iter()
                .map(|(q, p)| (map[*q], *p))
                .collect::<Vec<_>>();
            let ts = g.targets.iter().map(|q| map[*q]).collect::<Vec<_>>();
            self.apply(g.gate, &cs, &ts);
        }
        if self
            .state
            .quantum
            .keys()
            .any(|b| fresh.iter().any(|q| b[*q]))
        {
            bail!("CC: region workspace not clean (§6.2)")
        }
        for q in args {
            if let Some(parent) = self.state.handles[&q].parent {
                self.state.handles.remove(&q);
                self.state.handles.get_mut(&parent).unwrap().viewed = false;
            }
        }
        Ok(())
    }
    fn execute(&mut self, f: &Function, args: Vec<V>, depth: usize) -> Result<Vec<V>> {
        if depth > 512 {
            bail!("CC: host recursion limit 512")
        };
        if args.len() != f.groups[0].len() {
            bail!("CC: argument arity (§3.3)")
        }
        let a = self.envs.len();
        self.envs.push(
            f.groups[0]
                .iter()
                .zip(args)
                .map(|(p, v)| (p.name.clone(), v))
                .collect(),
        );
        let mut block = 0;
        let mut pc = 0;
        loop {
            if self.steps >= 1_000_000 {
                bail!("CC: host diagnostic step limit 1000000")
            };
            self.steps += 1;
            let i = f
                .blocks
                .get(block)
                .and_then(|b| b.instrs.get(pc))
                .ok_or_else(|| Error("CC: missing terminator (§3.4)".into()))?
                .clone();
            pc += 1;
            let op = i.opcode.as_str();
            let os = &i.operands;
            let pure = op == "br"
                || op.starts_with("const.")
                || op.starts_with("icmp.")
                || [
                    "add", "sub", "mul", "and", "or", "xor", "shl", "lshr", "ashr", "udiv", "urem",
                    "sdiv", "srem", "select", "inttoptr",
                ]
                .contains(&op)
                || ["zext.", "sext.", "trunc.", "ptrtoint.", "load."]
                    .iter()
                    .any(|p| op.starts_with(p));
            let frame = self.frames.len();
            self.frames.push(Frame {
                pure,
                measured: op == "measure",
                reversed: false,
                before: if pure || ["call", "ret", "uncall", "trap"].contains(&op) {
                    None
                } else {
                    Some(self.state.clone())
                },
                changes: vec![],
            });
            let result: Option<V> = if let Some(w) = op.strip_prefix("const.i") {
                Some(V::Int(
                    self.n(a, &os[0])? & mask(w.parse().unwrap()),
                    w.parse().unwrap(),
                ))
            } else if [
                "add", "sub", "mul", "and", "or", "xor", "shl", "lshr", "ashr", "udiv", "urem",
                "sdiv", "srem",
            ]
            .contains(&op)
            {
                let va = self.get(a, &os[0])?;
                let w = va.width()?;
                let x = va.int()?;
                let y = self.n(a, &os[1])?;
                if ["udiv", "urem", "sdiv", "srem"].contains(&op) && y == 0 {
                    self.fault(5)?;
                }
                if ["sdiv", "srem"].contains(&op) && x == 1u128 << (w - 1) && y == mask(w) {
                    self.fault(6)?;
                }
                let v = match op {
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
                Some(V::Int(v & mask(w), w))
            } else if let Some(pred) = op.strip_prefix("icmp.") {
                let x = self.get(a, &os[0])?;
                let w = x.width()?;
                let x = x.int()?;
                let y = self.n(a, &os[1])?;
                Some(V::Int(
                    u128::from(match pred {
                        "eq" => x == y,
                        "ne" => x != y,
                        "ult" => x < y,
                        "ule" => x <= y,
                        "slt" => signed(x, w) < signed(y, w),
                        "sle" => signed(x, w) <= signed(y, w),
                        _ => bail!("CC: unknown predicate"),
                    }),
                    1,
                ))
            } else if ["zext.i", "sext.i", "trunc.i", "ptrtoint.i"]
                .iter()
                .any(|p| op.starts_with(p))
            {
                let w = op.rsplit_once('i').unwrap().1.parse::<u32>().unwrap();
                let v = self.get(a, &os[0])?;
                let x = if op.starts_with("sext") {
                    signed(v.int()?, v.width()?) as u128
                } else {
                    v.int()?
                };
                Some(V::Int(x & mask(w), w))
            } else if op == "select" {
                Some(self.get(a, &os[if self.n(a, &os[0])? != 0 { 1 } else { 2 }])?)
            } else if op == "inttoptr" {
                let w = self.object.header.ptr_width;
                Some(V::Int(self.n(a, &os[0])? & mask(w), w))
            } else if let Some(w) = op.strip_prefix("load.i") {
                let w = w.parse::<u32>().unwrap();
                let p = self.n(a, &os[0])?;
                let data = self.read(p, (w / 8) as usize)?;
                let mut x = [0u8; 16];
                x[..data.len()].copy_from_slice(&data);
                Some(V::Int(u128::from_le_bytes(x), w))
            } else if let Some(w) = op.strip_prefix("store.i") {
                let n = w.parse::<usize>().unwrap() / 8;
                let p = self.n(a, &os[0])?;
                let v = self.n(a, &os[1])?;
                self.write(p, &v.to_le_bytes()[..n])?;
                None
            } else {
                match op {
                    "alloc" => {
                        let n = self.n(a, &os[0])?;
                        let p = self.state.bump;
                        let layout =
                            self.object.layout.as_ref().ok_or_else(|| {
                                Error("CC: allocation without layout (§7.2)".into())
                            })?;
                        let heap_end = layout.get("heap_base").and_then(Value::as_u64).unwrap()
                            as u128
                            + layout
                                .get("heap_limit_max")
                                .and_then(Value::as_u64)
                                .unwrap() as u128;
                        let table_exp = layout.get("A_tab").and_then(Value::as_u64).unwrap();
                        let Some(capacity) = 1u128.checked_shl(
                            table_exp
                                .try_into()
                                .map_err(|_| Error("CC: table exponent host limit".into()))?,
                        ) else {
                            bail!("CC: table exponent host limit");
                        };
                        let Some(end) = p.checked_add(n.max(1)).filter(|e| {
                            *e <= heap_end && (self.state.allocations.len() as u128) < capacity
                        }) else {
                            self.fault(2)?;
                            unreachable!()
                        };
                        self.state.bump = end;
                        self.state.allocations.insert(p, (n, true));
                        self.state.memory.retain(|k, _| *k < p || *k >= end);
                        Some(V::Int(p, self.object.header.ptr_width))
                    }
                    "free" => {
                        let p = self.n(a, &os[0])?;
                        let pinned = self
                            .object
                            .layout
                            .as_ref()
                            .and_then(|l| l.get("globals"))
                            .and_then(Value::as_object)
                            .is_some_and(|gs| {
                                gs.values().any(|g| {
                                    g.get("base").and_then(Value::as_u64).map(u128::from) == Some(p)
                                })
                            });
                        if !self
                            .state
                            .allocations
                            .get(&p)
                            .is_some_and(|(_, live)| *live)
                            || pinned
                        {
                            self.fault(7)?;
                        }
                        self.state.allocations.get_mut(&p).unwrap().1 = false;
                        None
                    }
                    "br" => {
                        let which = if os.len() == 1 {
                            0
                        } else if self.n(a, &os[0])? != 0 {
                            1
                        } else {
                            2
                        };
                        let Operand::Label(label, actuals) = &os[which] else {
                            bail!("CC: branch label")
                        };
                        let values = actuals
                            .as_deref()
                            .unwrap_or(&[])
                            .iter()
                            .map(|n| self.get(a, &Operand::Name(n.clone())))
                            .collect::<Result<Vec<_>>>()?;
                        block = f
                            .blocks
                            .iter()
                            .position(|b| b.name == *label)
                            .ok_or_else(|| Error("CC: missing block".into()))?;
                        pc = 0;
                        for (p, v) in f.blocks[block].params.iter().zip(values) {
                            self.set(frame, a, &p.name, v);
                        }
                        None
                    }
                    "call" => {
                        let Operand::Sym(name) = &os[0] else {
                            bail!("CC: call symbol")
                        };
                        let cf = self
                            .object
                            .function(name)
                            .ok_or_else(|| {
                                Error("unimplemented: CC imported body (milestone 2)".into())
                            })?
                            .clone();
                        let args = os[1..]
                            .iter()
                            .map(|o| self.get(a, o))
                            .collect::<Result<Vec<_>>>()?;
                        let results = self.execute(&cf, args.clone(), depth + 1)?;
                        for (n, v) in i.lhs.iter().zip(&results) {
                            self.set(frame, a, n, v.clone());
                        }
                        self.segments.push(Segment {
                            name: name.clone(),
                            args,
                            results,
                            first: frame,
                            last: self.frames.len() - 1,
                            reversed: false,
                        });
                        None
                    }
                    "uncall" => {
                        let Operand::Sym(name) = &os[0] else {
                            bail!("CC: uncall symbol")
                        };
                        let lists = os[1..]
                            .iter()
                            .map(|op| {
                                let Operand::List(ns) = op else {
                                    bail!("CC: uncall list")
                                };
                                ns.iter()
                                    .map(|n| self.get(a, &Operand::Name(n.clone())))
                                    .collect::<Result<Vec<_>>>()
                            })
                            .collect::<Result<Vec<_>>>()?;
                        let Some(s) = self.segments.iter().rposition(|s| !s.reversed) else {
                            self.fault(4)?;
                            unreachable!()
                        };
                        let sg = self.segments[s].clone();
                        if sg.name != *name
                            || sg.args != lists[0]
                            || sg.results != lists[1]
                            || self.frames[sg.last + 1..frame]
                                .iter()
                                .any(|f| !f.pure && !f.reversed)
                            || self.frames[sg.first..=sg.last].iter().any(|f| f.measured)
                        {
                            self.fault(4)?;
                        }
                        let (first, last) = (sg.first, sg.last);
                        self.undo(first, last)?;
                        for sg in &mut self.segments {
                            if sg.first >= first && sg.last <= last {
                                sg.reversed = true;
                            }
                        }
                        self.frames[frame].reversed = true;
                        None
                    }
                    "ret" => return os.iter().map(|o| self.get(a, o)).collect(),
                    "trap" => {
                        self.fault(4)?;
                        None
                    }
                    "effect" => {
                        let Operand::Name(devop) = &os[0] else {
                            bail!("CC: effect device")
                        };
                        let (dev, oper) = devop
                            .split_once('.')
                            .ok_or_else(|| Error("CC: effect spelling".into()))?;
                        let args = os[1..]
                            .iter()
                            .map(|o| self.n(a, o))
                            .collect::<Result<Vec<_>>>()?;
                        let mut record = obj([
                            ("dev", Value::Str(dev.into())),
                            ("op", Value::Str(oper.into())),
                            ("args", Value::Array(args.iter().map(num).collect())),
                        ]);
                        let mut data = None;
                        match (dev, oper) {
                            ("clock", "read") => {
                                let bs = self.state.clock.to_le_bytes().to_vec();
                                self.write(args[0], &bs)?;
                                self.state.clock += 1;
                                data = Some(bs)
                            }
                            ("entropy", "read") => {
                                let n = usize::try_from(args[1])
                                    .map_err(|_| Error("CC: entropy host limit".into()))?;
                                if !self.valid(args[0], n) {
                                    self.fault(7)?;
                                }
                                if self
                                    .state
                                    .entropy_pos
                                    .checked_add(n)
                                    .is_none_or(|end| end > self.entropy.len())
                                {
                                    self.fault(10)?;
                                }
                                let bs = self.entropy
                                    [self.state.entropy_pos..self.state.entropy_pos + n]
                                    .to_vec();
                                self.write(args[0], &bs)?;
                                self.state.entropy_pos += n;
                                data = Some(bs)
                            }
                            ("console", "write") => {
                                let bs = self.read(
                                    args[0],
                                    usize::try_from(args[1])
                                        .map_err(|_| Error("CC: console host limit".into()))?,
                                )?;
                                self.state.console.extend(&bs);
                                data = Some(bs)
                            }
                            ("block", "write") => {
                                let bs = self.read(args[1], 512)?;
                                self.state.block.insert(args[0], bs);
                            }
                            ("block", "read") => {
                                let bs = self
                                    .state
                                    .block
                                    .get(&args[0])
                                    .cloned()
                                    .unwrap_or(vec![0; 512]);
                                self.write(args[1], &bs)?;
                            }
                            _ => {
                                self.fault(8)?;
                            }
                        }
                        if let Some(bs) = data {
                            let Value::Object(m) = &mut record else {
                                unreachable!()
                            };
                            m.insert("bytes".into(), Value::Str(hex(&bs)));
                        }
                        self.state.tape.push(record);
                        None
                    }
                    "hostcall" => {
                        let Operand::Name(service) = &os[0] else {
                            bail!("CC: host service")
                        };
                        let p = self.n(a, &os[1])?;
                        let n = usize::try_from(self.n(a, &os[2])?)
                            .map_err(|_| Error("CC: host capacity limit".into()))?;
                        let payload = self.read(p, n)?;
                        let reply = self
                            .replies
                            .iter()
                            .find(|r| {
                                r.get("service").and_then(Value::as_str) == Some(service)
                                    && r.get("payload").and_then(Value::as_str)
                                        == Some(&hex(&payload))
                            })
                            .and_then(|r| r.get("reply"))
                            .and_then(Value::as_str)
                            .ok_or_else(|| Error("CC: no matching host reply (§10)".into()))?;
                        let bs = bytes(reply)?;
                        self.write(p, &bs[..bs.len().min(n)])?;
                        self.state.host.push(obj([
                            ("address", num(p)),
                            ("capacity", num(n)),
                            ("payload", Value::Str(hex(&payload))),
                            ("reply", Value::Str(hex(&bs))),
                            ("service", Value::Str(service.clone())),
                        ]));
                        Some(V::Int(bs.len() as u128, self.object.header.ptr_width))
                    }
                    "qreg.new" => {
                        let n = usize::try_from(self.n(a, &os[1])?)
                            .map_err(|_| Error("CC: qreg host limit".into()))?;
                        let wires = self.fresh_wires(n);
                        Some(self.handle(wires, None))
                    }
                    "qreg.free" => {
                        let q = self.qid(&self.get(a, &os[0])?)?;
                        let h = &self.state.handles[&q];
                        if h.parent.is_some()
                            || self
                                .state
                                .quantum
                                .keys()
                                .any(|b| h.wires.iter().any(|q| b[*q]))
                        {
                            self.fault(4)?;
                        }
                        self.state.handles.remove(&q);
                        None
                    }
                    "qreg.slice" => {
                        let q = self.qid(&self.get(a, &os[0])?)?;
                        let lo = self.n(a, &os[1])? as usize;
                        let len = self.n(a, &os[2])? as usize;
                        let wires = self.state.handles[&q]
                            .wires
                            .get(lo..lo + len)
                            .ok_or_else(|| Error("CC: slice bounds".into()))?
                            .to_vec();
                        self.state.handles.get_mut(&q).unwrap().viewed = true;
                        Some(self.handle(wires, Some(q)))
                    }
                    "qreg.concat" => {
                        let qs = os
                            .iter()
                            .map(|o| self.get(a, o).and_then(|v| self.qid(&v)))
                            .collect::<Result<Vec<_>>>()?;
                        let mut wires = vec![];
                        for q in qs {
                            wires.extend(self.state.handles.remove(&q).unwrap().wires);
                        }
                        Some(self.handle(wires, None))
                    }
                    "action_call" => {
                        self.region(a, os, false)?;
                        None
                    }
                    "adj" if matches!(&os[0],Operand::Name(n)if n=="action_call") => {
                        self.region(a, &os[1..], true)?;
                        None
                    }
                    "measure" => {
                        let q = self.qid(&self.get(a, &os[0])?)?;
                        let h = self.state.handles[&q].clone();
                        if h.parent.is_some() {
                            self.fault(4)?;
                        }
                        let mut probs = BTreeMap::<Vec<bool>, f64>::new();
                        for (b, z) in &self.state.quantum {
                            let key = h.wires.iter().map(|q| b[*q]).collect::<Vec<_>>();
                            *probs.entry(key).or_default() += z.0 * z.0 + z.1 * z.1;
                        }
                        let d = *self.draws.get(self.draw).unwrap_or(&0.5);
                        self.draw += 1;
                        let mut cumulative = 0.;
                        let mut selected = None;
                        for (bits, p) in probs {
                            cumulative += p;
                            if d < cumulative {
                                selected = Some((bits, p));
                                break;
                            }
                        }
                        let (bits, p) = selected
                            .ok_or_else(|| Error("CC: invalid Born normalization".into()))?;
                        self.state
                            .quantum
                            .retain(|b, _| h.wires.iter().zip(&bits).all(|(q, v)| b[*q] == *v));
                        for z in self.state.quantum.values_mut() {
                            z.0 /= p.sqrt();
                            z.1 /= p.sqrt();
                        }
                        let value = bits
                            .iter()
                            .enumerate()
                            .fold(0u128, |x, (j, b)| x | (u128::from(*b) << j));
                        self.measurements.push(obj([
                            (
                                "bits",
                                Value::Str(
                                    bits.iter().map(|b| if *b { '1' } else { '0' }).collect(),
                                ),
                            ),
                            ("value", num(value)),
                        ]));
                        self.state.handles.remove(&q);
                        Some(V::Int(value, h.wires.len() as u32))
                    }
                    _ => bail!("unimplemented: CC/R diagnostic {op} (milestone 2)"),
                }
            };
            if let Some(v) = result {
                if i.lhs.len() != 1 {
                    bail!("CC: result arity (§3.3)")
                };
                self.set(frame, a, &i.lhs[0], v);
            }
        }
    }
}
/// Execute a classical entry with retained reversible history. Options are the
/// published diagnostic CLI's data inputs; no reference binary is invoked.
pub fn run(object: &Object, name: &str, options: &[String], reverse: bool) -> Result<Value> {
    let f = object
        .function(name)
        .ok_or_else(|| Error("CC: unknown entry".into()))?;
    let mut m = Machine::new(object)?;
    let mut inputs = vec![];
    let mut it = options.iter();
    while let Some(k) = it.next() {
        let v = it
            .next()
            .ok_or_else(|| Error(format!("CC: {k} needs value")))?;
        match k.as_str() {
            "--args" => {
                if !v.is_empty() {
                    inputs = v.split(',').map(str::to_owned).collect();
                }
            }
            "--entropy" => m.entropy = bytes(v)?,
            "--draws" => {
                m.draws = v
                    .split(',')
                    .map(|s| s.parse::<f64>().map_err(|_| Error("CC: draw".into())))
                    .collect::<Result<Vec<_>>>()?;
                if m.draws
                    .iter()
                    .any(|x| !x.is_finite() || *x < 0. || *x >= 1.)
                {
                    bail!("CC: draw must lie in [0,1)")
                }
            }
            "--replies" => {
                let t = std::fs::read_to_string(v)
                    .map_err(|e| Error(format!("CC: reply file: {e}")))?;
                m.replies = Value::parse(&t)?
                    .as_array()
                    .ok_or_else(|| Error("CC: reply array".into()))?
                    .to_vec()
            }
            _ => bail!("unimplemented: CC diagnostic option {k} (milestone 2)"),
        }
    }
    if inputs.len() != f.groups[0].len() {
        bail!("CC: entry argument arity (§3.3)")
    };
    let args = inputs
        .iter()
        .zip(&f.groups[0])
        .map(|(s, p)| {
            let w = match p.ty {
                Type::Int(w) => w,
                Type::Ptr => object.header.ptr_width,
                _ => bail!("unimplemented: CC entry argument type (milestone 2)"),
            };
            let n = s
                .parse::<u128>()
                .map_err(|_| Error("CC: input integer".into()))?;
            if n > mask(w) {
                bail!("CC: input range")
            };
            Ok(V::Int(n, w))
        })
        .collect::<Result<Vec<_>>>()?;
    let results = match m.execute(f, args, 0) {
        Ok(rs) => rs,
        Err(e) if m.fault.is_some() => {
            let _ = e;
            vec![]
        }
        Err(e) => return Err(e),
    };
    let result_values = results
        .iter()
        .zip(&f.groups[1])
        .map(|(v, p)| {
            Ok(obj([
                ("type", Value::Str(p.ty.signature_string()?)),
                ("value", Value::Str(v.int()?.to_string())),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    if reverse && !m.frames.is_empty() {
        m.undo(0, m.frames.len() - 1)?;
        if m.state != m.entry {
            bail!("CC: history reversal differs from entry (§7.4)")
        };
    }
    let fault_names = [
        "FAULT_NONE",
        "FAULT_BUDGET_STEPS",
        "FAULT_BUDGET_HEAP",
        "FAULT_BUDGET_DEPTH",
        "FAULT_UB_TRAP",
        "FAULT_UB_DIV0",
        "FAULT_UB_SDIV_OVERFLOW",
        "FAULT_UB_MEMFAULT",
        "FAULT_EFFECT_UNMODELLED",
        "FAULT_CALLEE",
        "FAULT_ENTROPY_EXHAUSTED",
    ];
    let digest = |s: &State| crate::sha256::sha256_hex(format!("{:?}", s.memory).as_bytes());
    Ok(obj([
        ("entry", Value::Str(name.into())),
        (
            "status",
            Value::Str(
                if reverse {
                    "REVERSED"
                } else if m.fault.is_some() {
                    "FAILED"
                } else {
                    "COMPLETED"
                }
                .into(),
            ),
        ),
        ("steps", num(m.steps)),
        ("results", Value::Array(result_values)),
        ("fault", m.fault.map_or(Value::Null, num)),
        (
            "faults",
            Value::Array(
                m.fault
                    .map(|n| {
                        obj([
                            ("code", num(n)),
                            ("name", Value::Str(fault_names[n as usize].into())),
                        ])
                    })
                    .into_iter()
                    .collect(),
            ),
        ),
        ("clock", num(m.state.clock)),
        ("console_hex", Value::Str(hex(&m.state.console))),
        ("effect_tape_length", num(m.state.tape.len())),
        ("tape", Value::Array(m.state.tape.clone())),
        ("host_requests", Value::Array(m.state.host.clone())),
        ("measurements", Value::Array(m.measurements.clone())),
        ("equals_entry", Value::Bool(m.state == m.entry)),
        (
            "memory_equals_entry",
            Value::Bool(m.state.memory == m.entry.memory),
        ),
        ("memory_digest", Value::Str(digest(&m.state))),
        ("entry_memory_digest", Value::Str(digest(&m.entry))),
        ("first_difference", Value::Null),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn program(body: &str, heap: u64, globals: &str) -> Object {
        Object::parse(&format!("bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=16\n\nsection symbols\n{{}}\n\nsection functions\nfn r @test () () () () ()\n^entry():\n{body}ret\n\nsection data\ng 00\n\nsection layout\n{{\"A_tab\":2,\"H\":12,\"S_cells\":0,\"globals\":{globals},\"heap_base\":16,\"heap_limit_max\":{heap},\"interface_regions\":{{}},\"null_size\":1,\"ptr_width\":16}}\n")).unwrap()
    }
    #[test]
    fn allocation_failures_are_atomic_and_zero_size_is_not_accessible() {
        let o = program("n = const.i16 2\np = alloc n\n", 1, "{}");
        let r = run(&o, "test", &[], false).unwrap();
        assert_eq!(r.get("fault").and_then(Value::as_u64), Some(2));
        assert_eq!(r.get("memory_equals_entry"), Some(&Value::Bool(true)));
        let o = program("n = const.i16 0\np = alloc n\nx = load.i8 p\n", 1, "{}");
        assert_eq!(
            run(&o, "test", &[], false)
                .unwrap()
                .get("fault")
                .and_then(Value::as_u64),
            Some(7)
        );
    }
    #[test]
    fn allocation_table_capacity_and_pinned_entries_are_enforced() {
        let o = program(
            "n = const.i16 1\np = alloc n\np = alloc n\np = alloc n\np = alloc n\np = alloc n\n",
            16,
            "{}",
        );
        assert_eq!(
            run(&o, "test", &[], false)
                .unwrap()
                .get("fault")
                .and_then(Value::as_u64),
            Some(2)
        );
        let o = program(
            "n = const.i16 2\np = inttoptr n\nfree p\n",
            16,
            r#"{"g":{"base":2,"image":"g","size":1}}"#,
        );
        assert_eq!(
            run(&o, "test", &[], false)
                .unwrap()
                .get("fault")
                .and_then(Value::as_u64),
            Some(7)
        );
    }
}
