//! Lowering of Q-tier action bodies (§5.3, rows of §4.3–§4.7) into the
//! hierarchical circuit IR. Every §4 Q row is implemented here or fails
//! with `unimplemented: <row> (milestone N)`; oracle specialisation
//! (§5.2) is milestone 2 (docs/PLAN.md).
//!
//! The input is a parsed `Object`; callers that need verified identity
//! run `Object::check` first (the CLI and the differential harness do).

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::*;
use crate::object::{FnKind, Function, Instr, Object, Operand, Type};
use crate::oracle::Argument;
use crate::{Error, Result, bail};

/// Lower `@action` and every action it transitively calls.
pub fn lower_program(obj: &Object, action: &str) -> Result<Program> {
    lower_program_with_args(obj, action, &[])
}
pub fn lower_program_with_args(obj: &Object, action: &str, args: &[Argument]) -> Result<Program> {
    let mut lowerer = Lowerer {
        obj,
        done: Vec::new(),
        active: Vec::new(),
    };
    lowerer.lower_named(action, args)?;
    Ok(Program {
        circuits: lowerer.done,
        top: specialised_name(action, args),
    })
}

struct Lowerer<'a> {
    obj: &'a Object,
    done: Vec<Circuit>,
    active: Vec<String>,
}

impl<'a> Lowerer<'a> {
    fn lower_named(&mut self, name: &str, args: &[Argument]) -> Result<()> {
        let key = specialised_name(name, args);
        if self.active.iter().any(|n| n == name) {
            bail!("lower: action call graph cycle through @{name} (§5.1: acyclic)");
        }
        if self.done.iter().any(|c| c.name == key) {
            return Ok(());
        }
        let Some(f) = self.obj.function(name) else {
            if self.obj.imports.iter().any(|i| i.name == name) {
                bail!(
                    "unimplemented: lowering imported action @{name} without a body; decision bennett-circuits-z9r (milestone 2)"
                );
            }
            bail!("lower: unknown action @{name}");
        };
        if !f.is_action() {
            bail!(
                "lower: @{name} is a {}, not an action; oracle specialisation is milestone 2",
                f.kind.symbol_kind()
            );
        }
        self.active.push(name.to_string());
        let mut fb = FnLower::new(self.obj, f, args)?;
        fb.circuit.name = key;
        fb.lower_body()?;
        // Discover callees in body order and lower them too.
        let callees = fb.callees.clone();
        let extra = fb.extra.clone();
        let circuit = fb.finish()?;
        for (callee, arguments) in callees {
            self.lower_named(&callee, &arguments)?;
        }
        self.active.pop();
        for c in extra {
            if !self.done.iter().any(|d| d.name == c.name) {
                self.done.push(c);
            }
        }
        self.done.insert(0, circuit);
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum BundleKind {
    Interface,
    CleanWs,
    BorrowedWs,
    RegionCells,
}

struct FnLower<'a> {
    obj: &'a Object,
    f: &'a Function,
    circuit: Circuit,
    /// Physical bundle name -> (width, kind).
    bundles: BTreeMap<String, (u32, BundleKind)>,
    live: BTreeSet<String>,
    ever: BTreeSet<String>,
    /// Textual name -> current physical bundle name.
    rename: BTreeMap<String, String>,
    pred: Vec<QubitRef>,
    callees: Vec<(String, Vec<Argument>)>,
    arguments: BTreeMap<String, Argument>,
    extra: Vec<Circuit>,
    /// (region name, set-bit wire indices) to reverse at function exit.
    region_images: Vec<(String, Vec<u32>)>,
    fresh_counter: u32,
}

impl<'a> FnLower<'a> {
    fn new(obj: &'a Object, f: &'a Function, args: &[Argument]) -> Result<FnLower<'a>> {
        if args.len() != f.groups[3].len() {
            bail!("lower: @{} classical argument arity mismatch", f.name)
        }
        let arguments = f.groups[3]
            .iter()
            .zip(args)
            .map(|(p, a)| {
                if let (Type::Ptr, Argument::Integer(text)) = (&p.ty, a) {
                    validate_integer(text, obj.header.ptr_width)?;
                }
                Ok((p.name.clone(), validate_arg(&p.ty, a)?))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        if f.blocks.len() != 1 || !f.blocks[0].params.is_empty() {
            bail!(
                "unimplemented: multi-block action @{}; decision bennett-circuits-7bu (milestone 2); Q actions lower from one structured block",
                f.name
            );
        }
        let mut circuit = Circuit {
            name: f.name.clone(),
            perm: f.kind == FnKind::PermAction,
            mut_bundles: Vec::new(),
            in_bundles: Vec::new(),
            mem: Vec::new(),
            workspace: Vec::new(),
            local_regions: Vec::new(),
            body: Vec::new(),
        };
        let mut bundles = BTreeMap::new();
        let mut live = BTreeSet::new();
        let declare = |name: &str,
                       width: u32,
                       bundles: &mut BTreeMap<String, (u32, BundleKind)>,
                       live: &mut BTreeSet<String>| {
            bundles.insert(name.to_string(), (width, BundleKind::Interface));
            live.insert(name.to_string());
        };
        for p in &f.groups[0] {
            let w = wire_width_of(&p.ty, &p.name)?;
            circuit.mut_bundles.push(Bundle {
                name: p.name.clone(),
                width: w,
                role: Role::Mut,
            });
            declare(&p.name, w, &mut bundles, &mut live);
        }
        for p in &f.groups[1] {
            let w = wire_width_of(&p.ty, &p.name)?;
            circuit.in_bundles.push(Bundle {
                name: p.name.clone(),
                width: w,
                role: Role::In,
            });
            declare(&p.name, w, &mut bundles, &mut live);
        }
        for p in &f.groups[2] {
            let Type::Region(a, w) = p.ty else {
                bail!(
                    "lower: @{} mem parameter {:?} is not a region",
                    f.name,
                    p.name
                );
            };
            let role = match p.role.as_deref() {
                Some("mut") => Role::Mut,
                Some("in") => Role::In,
                other => bail!("lower: @{} mem {:?} has role {other:?}", f.name, p.name),
            };
            let m = MemBundle {
                name: p.name.clone(),
                addr_width: a,
                cell_width: w,
                role,
            };
            let total = m.width();
            if total > u64::from(u32::MAX) {
                bail!(
                    "lower: @{} region {:?} too large ({total} wires, host limit)",
                    f.name,
                    p.name
                );
            }
            declare(&p.name, total as u32, &mut bundles, &mut live);
            circuit.mem.push(m);
        }
        Ok(FnLower {
            obj,
            f,
            circuit,
            bundles,
            live,
            ever: BTreeSet::new(),
            rename: BTreeMap::new(),
            pred: Vec::new(),
            callees: Vec::new(),
            arguments,
            extra: Vec::new(),
            region_images: Vec::new(),
            fresh_counter: 0,
        })
    }

    fn controls(&self) -> Vec<Ctrl> {
        self.pred.iter().cloned().map(Ctrl::pos).collect()
    }

    fn resolve(&self, textual: &str) -> String {
        self.rename
            .get(textual)
            .cloned()
            .unwrap_or_else(|| textual.to_string())
    }

    fn width(&self, physical: &str) -> Result<u32> {
        self.bundles.get(physical).map(|(w, _)| *w).ok_or_else(|| {
            Error(format!(
                "lower: @{} unknown bundle {physical:?}",
                self.f.name
            ))
        })
    }

    fn require_live(&self, physical: &str) -> Result<()> {
        if !self.live.contains(physical) {
            bail!(
                "lower: @{} bundle {physical:?} used after consume/release (§3.3 linear SSA)",
                self.f.name
            );
        }
        Ok(())
    }

    fn fresh(&mut self, textual: &str) -> String {
        if !self.ever.contains(textual) && !self.bundles.contains_key(textual) {
            self.ever.insert(textual.to_string());
            return textual.to_string();
        }
        loop {
            self.fresh_counter += 1;
            let candidate = format!("{textual}.{}", self.fresh_counter);
            if self.ever.insert(candidate.clone()) && !self.bundles.contains_key(&candidate) {
                return candidate;
            }
        }
    }

    fn produce(&mut self, textual: &str, width: u32, kind: WsKind, class: &str) -> String {
        let physical = self.fresh(textual);
        self.rename.insert(textual.to_string(), physical.clone());
        let bk = match kind {
            WsKind::Clean => BundleKind::CleanWs,
            WsKind::Borrowed => BundleKind::BorrowedWs,
            WsKind::LocalRegion => BundleKind::RegionCells,
        };
        self.bundles.insert(physical.clone(), (width, bk));
        self.live.insert(physical.clone());
        self.circuit.workspace.push(Workspace {
            name: physical.clone(),
            width,
            kind,
            class: class.to_string(),
        });
        physical
    }

    fn consume(&mut self, textual: &str, want: BundleKind, what: &str) -> Result<String> {
        let physical = self.resolve(textual);
        self.require_live(&physical)?;
        let (_, kind) = self.bundles[&physical];
        if kind != want {
            bail!(
                "lower: @{} {what} of {textual:?} does not match its allocation kind",
                self.f.name
            );
        }
        self.live.remove(&physical);
        Ok(physical)
    }

    fn bundle_operand(&self, op: &Operand, what: &str) -> Result<String> {
        match op {
            Operand::Name(n) => {
                let p = self.resolve(n);
                self.require_live(&p)?;
                Ok(p)
            }
            other => bail!(
                "lower: @{} {what} operand must be a bundle name, got {other:?}",
                self.f.name
            ),
        }
    }

    fn wire_operand(&self, op: &Operand, what: &str) -> Result<QubitRef> {
        match op {
            Operand::Indexed(n, i) => {
                let p = self.resolve(n);
                self.require_live(&p)?;
                if *i >= self.width(&p)? {
                    bail!(
                        "lower: @{} wire index {i} out of range for {n:?}",
                        self.f.name
                    );
                }
                Ok(QubitRef::new(&p, *i))
            }
            Operand::Name(n) => {
                let p = self.resolve(n);
                self.require_live(&p)?;
                if self.width(&p)? != 1 {
                    bail!(
                        "lower: @{} {what} operand {n:?} is a multi-wire bundle; index it",
                        self.f.name
                    );
                }
                Ok(QubitRef::new(&p, 0))
            }
            other => bail!(
                "lower: @{} {what} operand must be a wire, got {other:?}",
                self.f.name
            ),
        }
    }

    fn check_distinct(&self, gate: &GateInst, what: &str) -> Result<()> {
        let mut seen: Vec<&QubitRef> = Vec::new();
        for q in gate.controls.iter().map(|c| &c.qubit).chain(&gate.targets) {
            if seen.contains(&q) {
                bail!(
                    "lower: @{} {what} wire operands are not pairwise distinct (§3.3): {q:?} repeats",
                    self.f.name
                );
            }
            seen.push(q);
        }
        Ok(())
    }

    fn emit_gate(
        &mut self,
        gate: BaseGate,
        own_controls: Vec<QubitRef>,
        targets: Vec<QubitRef>,
    ) -> Result<()> {
        let own_count = own_controls.len();
        let mut controls = own_controls.into_iter().map(Ctrl::pos).collect::<Vec<_>>();
        controls.extend(self.controls());
        let inst = GateInst {
            own_controls: own_count,
            gate,
            controls,
            targets,
        };
        self.check_distinct(&inst, "gate")?;
        self.circuit.body.push(Node::Gate(inst));
        Ok(())
    }
}

fn wire_width_of(ty: &Type, name: &str) -> Result<u32> {
    ty.wire_width()
        .ok_or_else(|| Error(format!("lower: parameter {name:?} is not a wire bundle")))
}

fn int_literal(op: &Operand, row: &str, f: &str) -> Result<u64> {
    match op {
        Operand::Int(n) => n
            .parse::<u64>()
            .map_err(|_| Error(format!("lower: @{f} {row} count {n} out of range"))),
        other => bail!("unimplemented: {row} with non-literal count {other:?} (milestone 2)"),
    }
}

impl<'a> FnLower<'a> {
    fn declare_regions(&mut self, instrs: &[Instr]) -> Result<()> {
        for i in instrs {
            if i.opcode == "qregion" {
                self.lower_qregion(i)?;
            }
            for child in &i.children {
                self.declare_regions(child)?;
            }
        }
        Ok(())
    }

    fn lower_body(&mut self) -> Result<()> {
        // Clone the instruction list so the borrow of `self.f` ends.
        let mut instrs = self.f.blocks[0].instrs.clone();
        fn substitute(is: &mut [Instr], args: &BTreeMap<String, Argument>) {
            for i in is {
                for o in &mut i.operands {
                    if let Operand::Name(n) = o
                        && let Some(a) = args.get(n)
                    {
                        *o = match a {
                            Argument::Integer(n) => Operand::Int(n.clone()),
                            Argument::Float(v) => Operand::Float(v.to_string()),
                            Argument::Token(_) => continue,
                        };
                    }
                }
                for child in &mut i.children {
                    substitute(child, args);
                }
            }
        }
        substitute(&mut instrs, &self.arguments);
        // Literal bounds are checked by Object::check. A symbolic trip count
        // is resolved only here, so compare it to the same text-order bound
        // before unrolling (including nested and zero-trip bodies).
        fn specialised_bounds(
            original: &[Instr],
            resolved: &[Instr],
            bounds: &[u64],
            ordinal: &mut usize,
            args: &BTreeMap<String, Argument>,
            name: &str,
        ) -> Result<()> {
            for (before, after) in original.iter().zip(resolved) {
                if before.opcode == "qloop" {
                    if let Some(Operand::Name(n)) = before.operands.first()
                        && args.contains_key(n)
                    {
                        let count = after
                            .operands
                            .first()
                            .ok_or_else(|| Error("lower: qloop arity (§4.6)".into()))?;
                        let count = int_literal(count, "qloop", name)?;
                        if bounds.get(*ordinal) != Some(&count) {
                            bail!(
                                "lower: @{name} specialised loop {ordinal} count {count} differs from declared bounds (§5.3)"
                            );
                        }
                    }
                    *ordinal += 1;
                }
                for (a, b) in before.children.iter().zip(&after.children) {
                    specialised_bounds(a, b, bounds, ordinal, args, name)?;
                }
            }
            Ok(())
        }
        specialised_bounds(
            &self.f.blocks[0].instrs,
            &instrs,
            &self.f.bounds,
            &mut 0,
            &self.arguments,
            &self.f.name,
        )?;
        self.declare_regions(&instrs)?;
        self.lower_seq(&instrs, false)
    }

    fn finish(mut self) -> Result<Circuit> {
        if !self.pred.is_empty() {
            bail!(
                "lower: @{} predicate stack non-empty at exit (§3.5)",
                self.f.name
            );
        }
        // Local regions: reverse image gates and release, reverse order
        // (§4.6). The body must have restored the cells to their image;
        // simulation verifies cleanliness after these flips.
        let images = std::mem::take(&mut self.region_images);
        for (name, bits) in images.iter().rev() {
            for &i in bits {
                self.circuit.body.push(Node::Gate(GateInst {
                    own_controls: 0,
                    gate: BaseGate::X,
                    controls: Vec::new(),
                    targets: vec![QubitRef::new(name, i)],
                }));
            }
            self.live.remove(name);
        }
        for w in &self.circuit.workspace {
            if self.live.contains(&w.name) {
                bail!(
                    "lower: @{} workspace {:?} never freed/returned/uncomputed (§5.1 explicit cleanup)",
                    self.f.name,
                    w.name
                );
            }
        }
        for (name, _) in self.circuit.interface_bundles() {
            if !self.live.contains(&name) {
                bail!(
                    "lower: @{} interface bundle {name:?} consumed by the body",
                    self.f.name
                );
            }
        }
        self.circuit.check_widths()?;
        Ok(self.circuit)
    }

    fn lower_seq(&mut self, instrs: &[Instr], nested: bool) -> Result<()> {
        for instr in instrs {
            self.lower_instr(instr, nested)?;
        }
        Ok(())
    }

    fn expect_operands(&self, instr: &Instr, n: usize) -> Result<()> {
        if instr.operands.len() != n {
            bail!(
                "lower: @{} {} expects {n} operands, got {}",
                self.f.name,
                instr.opcode,
                instr.operands.len()
            );
        }
        Ok(())
    }

    fn lhs1(&self, instr: &Instr) -> Result<String> {
        if instr.lhs.len() != 1 {
            bail!(
                "lower: @{} {} expects exactly one result",
                self.f.name,
                instr.opcode
            );
        }
        Ok(instr.lhs[0].clone())
    }

    fn no_lhs_children(&self, instr: &Instr) -> Result<()> {
        if !instr.children.is_empty() {
            bail!(
                "lower: @{} {} takes no brace group",
                self.f.name,
                instr.opcode
            );
        }
        Ok(())
    }

    fn angle(&self, instr: &Instr) -> Result<f64> {
        match &instr.operands[0] {
            Operand::Float(t) | Operand::Int(t) => {
                let v: f64 = t
                    .parse()
                    .map_err(|_| Error(format!("lower: @{} bad angle {t:?}", self.f.name)))?;
                if !v.is_finite() {
                    bail!("lower: @{} non-finite angle {t:?} (§3.1)", self.f.name);
                }
                Ok(v)
            }
            other => bail!(
                "unimplemented: {} with non-literal angle (milestone 2): {other:?}",
                instr.opcode
            ),
        }
    }

    fn lower_instr(&mut self, instr: &Instr, _nested: bool) -> Result<()> {
        let op = instr.opcode.as_str();
        match op {
            // -- §4.4 gates -------------------------------------------------
            "x" | "h" | "s" | "sdg" | "t" | "tdg" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 1)?;
                let gate = match op {
                    "x" => BaseGate::X,
                    "h" => BaseGate::H,
                    "s" => BaseGate::S,
                    "sdg" => BaseGate::Sdg,
                    "t" => BaseGate::T,
                    _ => BaseGate::Tdg,
                };
                let t = self.wire_operand(&instr.operands[0], op)?;
                self.emit_gate(gate, Vec::new(), vec![t])
            }
            "cx" | "cz" | "swap" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 2)?;
                let a = self.wire_operand(&instr.operands[0], op)?;
                let b = self.wire_operand(&instr.operands[1], op)?;
                match op {
                    "cx" => self.emit_gate(BaseGate::X, vec![a], vec![b]),
                    "cz" => self.emit_gate(BaseGate::Cz, Vec::new(), vec![a, b]),
                    _ => self.emit_gate(BaseGate::Swap, Vec::new(), vec![a, b]),
                }
            }
            "ccx" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 3)?;
                let c1 = self.wire_operand(&instr.operands[0], op)?;
                let c2 = self.wire_operand(&instr.operands[1], op)?;
                let t = self.wire_operand(&instr.operands[2], op)?;
                self.emit_gate(BaseGate::X, vec![c1, c2], vec![t])
            }
            "mcx" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 2)?;
                let Operand::List(cs) = &instr.operands[0] else {
                    bail!(
                        "lower: @{} mcx controls must be a parenthesised list",
                        self.f.name
                    );
                };
                let mut ctrls = Vec::new();
                for c in cs.clone() {
                    let operand = if let Some((name, index)) =
                        c.strip_suffix(']').and_then(|s| s.split_once('['))
                    {
                        Operand::Indexed(
                            name.into(),
                            index
                                .parse()
                                .map_err(|_| Error("lower: invalid mcx index".into()))?,
                        )
                    } else {
                        Operand::Name(c)
                    };
                    ctrls.push(self.wire_operand(&operand, op)?);
                }
                let t = self.wire_operand(&instr.operands[1], op)?;
                self.emit_gate(BaseGate::X, ctrls, vec![t])
            }
            "rx" | "ry" | "rz" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 2)?;
                let theta = self.angle(instr)?;
                let gate = match op {
                    "rx" => BaseGate::Rx(theta),
                    "ry" => BaseGate::Ry(theta),
                    _ => BaseGate::Rz(theta),
                };
                let t = self.wire_operand(&instr.operands[1], op)?;
                self.emit_gate(gate, Vec::new(), vec![t])
            }
            // -- §4.3 allocation ---------------------------------------------
            "qalloc" | "qborrow" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 2)?;
                let class = match &instr.operands[0] {
                    Operand::Name(n) => n.clone(),
                    other => bail!(
                        "lower: @{} {op} class must be a name, got {other:?}",
                        self.f.name
                    ),
                };
                let n = int_literal(&instr.operands[1], op, &self.f.name)?;
                let width = u32::try_from(n).map_err(|_| {
                    Error(format!(
                        "lower: @{} {op} count too large (host limit)",
                        self.f.name
                    ))
                })?;
                let dst = self.lhs1(instr)?;
                let kind = if op == "qalloc" {
                    WsKind::Clean
                } else {
                    WsKind::Borrowed
                };
                self.produce(&dst, width, kind, &class);
                Ok(())
            }
            "qfree" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 1)?;
                let Operand::Name(n) = &instr.operands[0] else {
                    bail!("lower: @{} qfree operand must be a bundle", self.f.name);
                };
                self.consume(&n.clone(), BundleKind::CleanWs, "qfree")?;
                Ok(())
            }
            "qreturn" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 1)?;
                let Operand::Name(n) = &instr.operands[0] else {
                    bail!("lower: @{} qreturn operand must be a bundle", self.f.name);
                };
                self.consume(&n.clone(), BundleKind::BorrowedWs, "qreturn")?;
                Ok(())
            }
            // -- §4.5 copy / XOR / exchange ----------------------------------
            "qcopy" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 1)?;
                let src = self.bundle_operand(&instr.operands[0], op)?;
                let width = self.width(&src)?;
                let dst = self.lhs1(instr)?;
                let dst = self.produce(&dst, width, WsKind::Clean, "any");
                let controls = self.controls();
                self.circuit.body.push(Node::Copy { src, dst, controls });
                Ok(())
            }
            "quncopy" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 2)?;
                let src = self.bundle_operand(&instr.operands[0], op)?;
                let Operand::Name(d) = &instr.operands[1] else {
                    bail!("lower: @{} quncopy target must be a bundle", self.f.name);
                };
                let dst = self.consume(&d.clone(), BundleKind::CleanWs, "quncopy")?;
                let controls = self.controls();
                self.circuit.body.push(Node::Uncopy { src, dst, controls });
                Ok(())
            }
            "qxor" | "qexch" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 2)?;
                let a = self.bundle_operand(&instr.operands[0], op)?;
                let b = self.bundle_operand(&instr.operands[1], op)?;
                if self.width(&a)? != self.width(&b)? {
                    bail!("lower: @{} {op} bundle widths differ", self.f.name);
                }
                let controls = self.controls();
                if op == "qxor" {
                    self.circuit.body.push(Node::Xor {
                        src: a,
                        tgt: b,
                        controls,
                    });
                } else {
                    self.circuit.body.push(Node::Exch { a, b, controls });
                }
                Ok(())
            }
            "qinplace" => self.lower_qinplace(instr),
            // -- §4.6 regions and quantum memory -----------------------------
            "qregion" => Ok(()),
            "qload" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 2)?;
                let (region, addr, cell_w) = self.region_and_addr(instr, 0, 1)?;
                let dst = self.lhs1(instr)?;
                let dst = self.produce(&dst, cell_w, WsKind::Clean, "any");
                let controls = self.controls();
                self.circuit.body.push(Node::Load {
                    region,
                    addr,
                    dst,
                    controls,
                });
                Ok(())
            }
            "qunload" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 3)?;
                let (region, addr, _) = self.region_and_addr(instr, 0, 1)?;
                let Operand::Name(d) = &instr.operands[2] else {
                    bail!("lower: @{} qunload value must be a bundle", self.f.name);
                };
                let dst = self.consume(&d.clone(), BundleKind::CleanWs, "qunload")?;
                let controls = self.controls();
                self.circuit.body.push(Node::Unload {
                    region,
                    addr,
                    dst,
                    controls,
                });
                Ok(())
            }
            "qstore" | "qexchm" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 3)?;
                let (region, addr, cell_w) = self.region_and_addr(instr, 0, 1)?;
                let val = self.bundle_operand(&instr.operands[2], op)?;
                if self.width(&val)? != cell_w {
                    bail!("lower: @{} {op} value width != cell width", self.f.name);
                }
                let controls = self.controls();
                if op == "qstore" {
                    self.circuit.body.push(Node::Store {
                        region,
                        addr,
                        val,
                        controls,
                    });
                } else {
                    self.circuit.body.push(Node::ExchM {
                        region,
                        addr,
                        val,
                        controls,
                    });
                }
                Ok(())
            }
            "qpred" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 1)?;
                let w = self.wire_operand(&instr.operands[0], op)?;
                self.pred.push(w);
                Ok(())
            }
            "qunpred" => {
                self.no_lhs_children(instr)?;
                self.expect_operands(instr, 1)?;
                let w = self.wire_operand(&instr.operands[0], op)?;
                match self.pred.pop() {
                    Some(top) if top == w => Ok(()),
                    other => bail!(
                        "lower: @{} qunpred {w:?} does not match predicate stack top {other:?} (§3.5)",
                        self.f.name
                    ),
                }
            }
            "qif" => self.lower_qif(instr),
            "qloop" => self.lower_qloop(instr),
            // -- §4.7 region calls --------------------------------------------
            "action_call" | "oracle_call" | "adj" | "ctl" => self.lower_call(instr),
            other => bail!(
                "lower: @{} opcode {other:?} is not a Q-action row (§5.3 body grammar)",
                self.f.name
            ),
        }
    }

    fn region_and_addr(
        &self,
        instr: &Instr,
        ri: usize,
        ai: usize,
    ) -> Result<(String, String, u32)> {
        let Operand::Name(r) = &instr.operands[ri] else {
            bail!(
                "lower: @{} {} region must be a name",
                self.f.name,
                instr.opcode
            );
        };
        let region = self.resolve(r);
        self.require_live(&region)?;
        let Some((a_w, cell_w)) = self.circuit.region_geom(&region) else {
            bail!("lower: @{} {r:?} is not a region", self.f.name);
        };
        if a_w >= 26 {
            bail!(
                "lower: @{} region {r:?} address width {a_w} too large (host limit)",
                self.f.name
            );
        }
        let addr = self.bundle_operand(&instr.operands[ai], &instr.opcode)?;
        if self.width(&addr)? != a_w {
            bail!(
                "lower: @{} {} address width {} != region address width {a_w}",
                self.f.name,
                instr.opcode,
                self.width(&addr)?
            );
        }
        Ok((region, addr, cell_w))
    }

    fn lower_qregion(&mut self, instr: &Instr) -> Result<()> {
        self.no_lhs_children(instr)?;
        if instr.operands.len() != 3 && instr.operands.len() != 4 {
            bail!("lower: @{} qregion expects name A W [@image]", self.f.name);
        }
        let Operand::Name(name) = &instr.operands[0] else {
            bail!("lower: @{} qregion name must be a name", self.f.name);
        };
        if self.bundles.contains_key(name) {
            bail!("lower: duplicate local region {name:?}");
        }
        let a = int_literal(&instr.operands[1], "qregion", &self.f.name)?;
        let w = int_literal(&instr.operands[2], "qregion", &self.f.name)?;
        if a >= 26 {
            bail!(
                "lower: @{} qregion address width {a} too large (host limit)",
                self.f.name
            );
        }
        let cells = 1u64 << a;
        let total = cells
            .checked_mul(w)
            .ok_or_else(|| Error("lower: qregion size overflow".into()))?;
        if total > u64::from(u32::MAX) {
            bail!(
                "lower: @{} qregion too large ({total} wires, host limit)",
                self.f.name
            );
        }
        // Image bits: byte k contributes bits [8k, 8k+8) (§4.6).
        let mut set_bits: Vec<u32> = Vec::new();
        if let Some(image_op) = instr.operands.get(3) {
            let Operand::Sym(image) = image_op else {
                bail!("lower: @{} qregion image must be @data", self.f.name);
            };
            let Some(entry) = self.obj.data_entry(image) else {
                bail!(
                    "lower: @{} qregion image @{image} has no data entry",
                    self.f.name
                );
            };
            if (entry.bytes.len() as u64) * 8 != total {
                bail!(
                    "lower: @{} qregion image @{image} is {} bytes, expected {} (= 2^A*W/8)",
                    self.f.name,
                    entry.bytes.len(),
                    total / 8
                );
            }
            for (k, byte) in entry.bytes.iter().enumerate() {
                for b in 0..8 {
                    if (byte >> b) & 1 == 1 {
                        set_bits.push((8 * k + b) as u32);
                    }
                }
            }
        }
        let physical = self.produce(name, total as u32, WsKind::LocalRegion, "any");
        self.circuit.local_regions.push(RegionInfo {
            name: physical.clone(),
            addr_width: a as u32,
            cell_width: w as u32,
        });
        for &i in &set_bits {
            self.circuit.body.push(Node::Gate(GateInst {
                own_controls: 0,
                gate: BaseGate::X,
                controls: Vec::new(),
                targets: vec![QubitRef::new(&physical, i)],
            }));
        }
        self.region_images.push((physical, set_bits));
        Ok(())
    }

    fn lower_scoped(&mut self, body: &[Instr]) -> Result<()> {
        let pred = self.pred.clone();
        self.lower_seq(body, true)?;
        if self.pred != pred {
            bail!("lower: unbalanced predicate stack in structured body (§3.5)");
        }
        Ok(())
    }

    fn lower_qif(&mut self, instr: &Instr) -> Result<()> {
        self.expect_operands(instr, 1)?;
        if instr.children.is_empty() || instr.children.len() > 2 {
            bail!(
                "lower: @{} qif expects one or two brace groups",
                self.f.name
            );
        }
        let w = self.wire_operand(&instr.operands[0], "qif")?;
        // §4.6 expansion, verbatim.
        self.pred.push(w.clone());
        let then = instr.children[0].clone();
        self.lower_scoped(&then)?;
        self.pred.pop();
        if instr.children.len() == 2 {
            self.emit_gate(BaseGate::X, Vec::new(), vec![w.clone()])?;
            self.pred.push(w.clone());
            let els = instr.children[1].clone();
            self.lower_scoped(&els)?;
            self.pred.pop();
            self.emit_gate(BaseGate::X, Vec::new(), vec![w])?;
        }
        Ok(())
    }

    fn lower_qloop(&mut self, instr: &Instr) -> Result<()> {
        if instr.children.len() != 1 {
            bail!("lower: @{} qloop expects one brace group", self.f.name);
        }
        let body = instr.children[0].clone();
        match instr.operands.len() {
            1 => {
                let n = int_literal(&instr.operands[0], "qloop", &self.f.name)?;
                if n > 1_000_000 {
                    bail!(
                        "lower: @{} qloop count {n} too large (host limit)",
                        self.f.name
                    );
                }
                for _ in 0..n {
                    self.lower_scoped(&body)?;
                }
                Ok(())
            }
            3 => {
                // qloop n while cond (§4.6 flag construction).
                let n = int_literal(&instr.operands[0], "qloop while", &self.f.name)?;
                if !matches!(&instr.operands[1], Operand::Name(k) if k == "while") {
                    bail!("lower: @{} malformed qloop operands", self.f.name);
                }
                if n == 0 || n > 1_000_000 {
                    bail!("lower: @{} qloop while count {n} out of range", self.f.name);
                }
                let cond = self.bundle_operand(&instr.operands[2], "qloop while")?;
                if u64::from(self.width(&cond)?) != n {
                    bail!(
                        "lower: @{} qloop while cond width must equal the trip count (§4.6)",
                        self.f.name
                    );
                }
                let n = n as u32;
                let mut flags = Vec::new();
                for i in 0..n {
                    flags.push(self.produce(
                        &format!("$qloopwhile.flag{}", i + 1),
                        1,
                        WsKind::Clean,
                        "any",
                    ));
                }
                let cond_at = |i: u32| QubitRef::new(&cond, i);
                self.emit_gate(
                    BaseGate::X,
                    vec![cond_at(0)],
                    vec![QubitRef::new(&flags[0], 0)],
                )?;
                for i in 0..n {
                    self.pred.push(QubitRef::new(&flags[i as usize], 0));
                    self.lower_scoped(&body)?;
                    self.pred.pop();
                    if i + 1 < n {
                        self.emit_gate(
                            BaseGate::X,
                            vec![QubitRef::new(&flags[i as usize], 0), cond_at(i + 1)],
                            vec![QubitRef::new(&flags[(i + 1) as usize], 0)],
                        )?;
                    }
                }
                for i in (1..n).rev() {
                    self.emit_gate(
                        BaseGate::X,
                        vec![QubitRef::new(&flags[(i - 1) as usize], 0), cond_at(i)],
                        vec![QubitRef::new(&flags[i as usize], 0)],
                    )?;
                }
                self.emit_gate(
                    BaseGate::X,
                    vec![cond_at(0)],
                    vec![QubitRef::new(&flags[0], 0)],
                )?;
                for flag in flags {
                    self.live.remove(&flag);
                }
                Ok(())
            }
            _ => bail!("lower: @{} malformed qloop operands", self.f.name),
        }
    }

    fn lower_call(&mut self, instr: &Instr) -> Result<()> {
        self.no_lhs_children(instr)?;
        // Unpeel `ctl w` / `adj` prefixes (§4.7).
        let mut head = instr.opcode.clone();
        let mut operands: Vec<Operand> = instr.operands.clone();
        let mut adjoint = false;
        let mut extra_ctrl: Option<QubitRef> = None;
        loop {
            match head.as_str() {
                "adj" => {
                    if adjoint {
                        bail!("lower: @{} double adj on a call", self.f.name);
                    }
                    adjoint = true;
                    let Some(Operand::Name(next)) = operands.first() else {
                        bail!("lower: @{} malformed adj call", self.f.name);
                    };
                    head = next.clone();
                    operands.remove(0);
                }
                "ctl" => {
                    if extra_ctrl.is_some() || operands.len() < 2 {
                        bail!("lower: @{} malformed ctl call", self.f.name);
                    }
                    extra_ctrl = Some(self.wire_operand(&operands[0].clone(), "ctl")?);
                    let Operand::Name(next) = operands[1].clone() else {
                        bail!("lower: @{} malformed ctl call", self.f.name);
                    };
                    head = next;
                    operands.drain(..2);
                }
                _ => break,
            }
        }
        match head.as_str() {
            "oracle_call" => {
                let mut controls = self.controls();
                if let Some(c) = extra_ctrl {
                    controls.push(Ctrl::pos(c));
                }
                self.oracle_call(&operands, controls)
            }
            "action_call" => {
                if operands.len() != 5 {
                    bail!(
                        "lower: @{} action_call expects callee and four lists (§4.7)",
                        self.f.name
                    );
                }
                let Operand::Sym(callee) = operands[0].clone() else {
                    bail!("lower: @{} action_call callee must be @name", self.f.name);
                };
                let lists: Vec<Vec<String>> = operands[1..]
                    .iter()
                    .map(|o| match o {
                        Operand::List(l) => Ok(l.clone()),
                        other => bail!(
                            "lower: @{} action_call argument must be a list, got {other:?}",
                            self.f.name
                        ),
                    })
                    .collect::<Result<Vec<_>>>()?;
                let (muts, ins, mems, cargs) = (&lists[0], &lists[1], &lists[2], &lists[3]);

                let Some(cf) = self.obj.function(&callee) else {
                    if self.obj.imports.iter().any(|i| i.name == callee) {
                        bail!(
                            "unimplemented: imported action @{callee} workspace effects; decision bennett-circuits-z9r (milestone 2)"
                        );
                    }
                    bail!("lower: @{} calls unknown action @{callee}", self.f.name);
                };
                if !cf.is_action() {
                    bail!(
                        "lower: @{} action_call target @{callee} is not an action",
                        self.f.name
                    );
                }
                let arguments = resolve_args(cargs, &cf.groups[3], &self.arguments)?;
                // Width-check against the callee header.
                let mut args: Vec<String> = Vec::new();
                for (actuals, formals, group) in
                    [(muts, &cf.groups[0], "mut"), (ins, &cf.groups[1], "in")]
                {
                    if actuals.len() != formals.len() {
                        bail!(
                            "lower: @{} action_call @{callee} {group} arity mismatch",
                            self.f.name
                        );
                    }
                    for (a, formal) in actuals.iter().zip(formals) {
                        let p = self.bundle_operand(&Operand::Name(a.clone()), "action_call")?;
                        if u64::from(self.width(&p)?)
                            != u64::from(wire_width_of(&formal.ty, &formal.name)?)
                        {
                            bail!(
                                "lower: @{} action_call @{callee} {group} width mismatch on {a:?}",
                                self.f.name
                            );
                        }
                        args.push(p);
                    }
                }
                if mems.len() != cf.groups[2].len() {
                    bail!(
                        "lower: @{} action_call @{callee} mem arity mismatch",
                        self.f.name
                    );
                }
                for (a, formal) in mems.iter().zip(&cf.groups[2]) {
                    let Type::Region(fa, fw) = formal.ty else {
                        bail!("lower: @{callee} mem formal is not a region");
                    };
                    let p = self.resolve(a);
                    self.require_live(&p)?;
                    if self.circuit.region_geom(&p) != Some((fa, fw)) {
                        bail!(
                            "lower: @{} action_call @{callee} mem geometry mismatch on {a:?}",
                            self.f.name
                        );
                    }
                    args.push(p);
                }
                let mut controls = self.controls();
                if let Some(c) = extra_ctrl {
                    controls.push(Ctrl::pos(c));
                }
                if args.iter().collect::<BTreeSet<_>>().len() != args.len()
                    || controls.iter().any(|c| args.contains(&c.qubit.bundle))
                {
                    bail!("lower: action call interface/effective controls overlap (§3.3)")
                }
                let item = (callee.clone(), arguments.clone());
                if !self.callees.contains(&item) {
                    self.callees.push(item);
                }
                self.circuit.body.push(Node::Call {
                    callee: specialised_name(&callee, &arguments),
                    adjoint,
                    controls,
                    args,
                });
                Ok(())
            }
            other => bail!(
                "lower: @{} {other:?} is not a region call (§4.7)",
                self.f.name
            ),
        }
    }
}

fn specialised_name(name: &str, args: &[Argument]) -> String {
    if args.is_empty() {
        name.into()
    } else {
        format!(
            "{name}${}",
            &crate::sha256::sha256_hex(format!("{args:?}").as_bytes())[..16]
        )
    }
}
pub(crate) fn validate_integer(text: &str, width: u32) -> Result<()> {
    let negative = text.starts_with('-');
    let text = text.strip_prefix('-').unwrap_or(text);
    if width == 0 || text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        bail!("lower: invalid integer argument (§3.3)")
    }
    let mut ds = text.bytes().map(|b| b - b'0').collect::<Vec<_>>();
    let (mut bits, mut ones) = (0u32, 0u32);
    while ds.iter().any(|d| *d != 0) {
        let mut carry = 0;
        for d in &mut ds {
            let n = carry * 10 + *d;
            *d = n / 2;
            carry = n % 2;
        }
        bits += 1;
        ones += u32::from(carry);
        if bits > width {
            bail!("lower: integer argument out of range at width {width} (§3.3)")
        }
    }
    if negative && bits == width && ones != 1 {
        bail!("lower: negative integer argument outside signed range at width {width} (§3.3)")
    }
    Ok(())
}
pub(crate) fn validate_arg(ty: &Type, a: &Argument) -> Result<Argument> {
    match (ty, a) {
        (Type::Token, Argument::Token(_)) => {}
        (Type::F64, Argument::Float(v)) if v.is_finite() => {}
        (Type::Int(w), Argument::Integer(text)) => {
            if *w > 128 {
                bail!("lower: integer type exceeds i128 (§3.3)")
            }
            validate_integer(text, *w)?;
        }
        (Type::Ptr, Argument::Integer(text)) => {
            if text.strip_prefix('-').unwrap_or(text).is_empty()
                || !text
                    .strip_prefix('-')
                    .unwrap_or(text)
                    .bytes()
                    .all(|b| b.is_ascii_digit())
            {
                bail!("lower: invalid pointer argument (§3.3)")
            }
        }
        _ => bail!("lower: classical argument type mismatch (§3.3)"),
    }
    Ok(a.clone())
}
fn resolve_args(
    actuals: &[String],
    formals: &[crate::object::Param],
    env: &BTreeMap<String, Argument>,
) -> Result<Vec<Argument>> {
    if actuals.len() != formals.len() {
        bail!("lower: classical argument arity (§3.3)")
    }
    actuals
        .iter()
        .zip(formals)
        .map(|(n, p)| {
            let a = if let Some(a) = env.get(n) {
                a.clone()
            } else {
                match p.ty {
                    Type::Int(_) | Type::Ptr => Argument::Integer(n.clone()),
                    Type::F64 => Argument::Float(
                        n.parse()
                            .map_err(|_| Error("lower: invalid f64 argument".into()))?,
                    ),
                    _ => bail!("lower: unknown classical argument {n}"),
                }
            };
            validate_arg(&p.ty, &a)
        })
        .collect()
}
impl FnLower<'_> {
    fn oracle_call(&mut self, ops: &[Operand], controls: Vec<Ctrl>) -> Result<()> {
        if ops.len() != 7 {
            bail!("lower: oracle_call needs complete interface (§4.7)")
        }
        let Operand::Sym(name) = &ops[0] else {
            bail!("lower: oracle_call symbol (§4.7)")
        };
        let Operand::Name(token) = &ops[1] else {
            bail!("lower: oracle token (§5.4)")
        };
        let Some(Argument::Token(budget)) = self.arguments.get(token) else {
            bail!("lower: missing oracle token {token}")
        };
        let cf = self.obj.function(name).ok_or_else(|| {
            Error(format!(
                "unimplemented: imported oracle @{name}; decision bennett-circuits-z9r (milestone 2)"
            ))
        })?;
        let Operand::List(ins) = &ops[2] else {
            bail!("lower: oracle input list")
        };
        let Operand::List(mems) = &ops[5] else {
            bail!("lower: oracle memory list")
        };
        let Operand::List(cargs) = &ops[6] else {
            bail!("lower: oracle args list")
        };
        let arguments = resolve_args(cargs, &cf.groups[4], &self.arguments)?;
        let program = crate::oracle::specialise(self.obj, name, *budget, &arguments)?;
        let key = specialised_name(name, &[vec![Argument::Token(*budget)], arguments].concat());
        let mut circuits = program.circuits;
        for c in &mut circuits {
            let old = c.name.clone();
            c.name = if old == program.top {
                key.clone()
            } else {
                format!("{key}${old}")
            };
            for node in &mut c.body {
                if let Node::Call { callee, .. } = node {
                    *callee = if *callee == program.top {
                        key.clone()
                    } else {
                        format!("{key}${callee}")
                    }
                }
            }
        }
        let mut args = vec![
            self.bundle_operand(&ops[3], "oracle target")?,
            self.bundle_operand(&ops[4], "oracle fault")?,
        ];
        for n in ins.iter().chain(mems) {
            args.push(self.bundle_operand(&Operand::Name(n.clone()), "oracle input/memory")?);
        }
        let c = circuits.iter().find(|c| c.name == key).unwrap();
        let formals = c.interface_bundles();
        if args.len() != formals.len() {
            bail!("lower: oracle interface arity (§3.3)")
        }
        let mut distinct = BTreeSet::new();
        for (n, (_, w)) in args.iter().zip(formals) {
            if self.width(n)? as u64 != w {
                bail!("lower: oracle interface width (§3.3)")
            }
            if !distinct.insert(n.clone()) {
                bail!("lower: oracle interface operands overlap (§3.3)")
            }
            if controls.iter().any(|c| c.qubit.bundle == *n) {
                bail!("lower: oracle control overlaps interface (§3.3)")
            }
        }
        for c in circuits {
            if !self.extra.iter().any(|d| d.name == c.name) {
                self.extra.push(c);
            }
        }
        self.circuit.body.push(Node::Call {
            callee: key,
            adjoint: false,
            controls,
            args,
        });
        Ok(())
    }
    fn lower_qinplace(&mut self, i: &Instr) -> Result<()> {
        self.expect_operands(i, 4)?;
        let Operand::Sym(g) = &i.operands[0] else {
            bail!("lower: qinplace oracle")
        };
        let Operand::Sym(inv) = &i.operands[1] else {
            bail!("lower: qinplace inverse")
        };
        let Operand::Name(token) = &i.operands[2] else {
            bail!("lower: qinplace token (§4.5)")
        };
        let Some(Argument::Token(budget)) = self.arguments.get(token) else {
            bail!("lower: missing qinplace budget token (§4.5)")
        };
        crate::proof::inverse_pair(self.obj, g, inv, *budget)?;
        let x = self.bundle_operand(&i.operands[3], "qinplace")?;
        let w = self.width(&x)?;
        let f = self
            .obj
            .function(g)
            .ok_or_else(|| Error("lower: missing qinplace oracle".into()))?;
        let fw = f.groups[2][0].ty.wire_width().unwrap();
        let t = self.produce("$inplace_t", w, WsKind::Clean, "any");
        let fr = self.produce("$inplace_f", fw, WsKind::Clean, "any");
        let call = |name: &str| {
            vec![
                Operand::Sym(name.into()),
                i.operands[2].clone(),
                Operand::List(vec![x.clone()]),
                Operand::Name(t.clone()),
                Operand::Name(fr.clone()),
                Operand::List(vec![]),
                Operand::List(vec![]),
            ]
        };
        self.oracle_call(&call(g), self.controls())?;
        self.circuit.body.push(Node::Exch {
            a: x.clone(),
            b: t.clone(),
            controls: self.controls(),
        });
        self.oracle_call(&call(inv), self.controls())?;
        self.live.remove(&t);
        self.live.remove(&fr);
        Ok(())
    }
}
