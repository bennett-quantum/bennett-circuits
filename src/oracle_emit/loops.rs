//! A proved natural loop, retaining immutable iteration snapshots for replay.
//! No partial iteration is skipped unless every row in its cycle is total.
use super::*;
use source::Read;
mod eval;
mod proof;
use eval::Eval;
use proof::Proof;
/// Original-row charges, independent of the number of generated gates.
#[derive(Clone, Debug)]
pub struct LoopInfo {
    pub header: usize,
    pub entry_rows: u64,
    pub continue_rows: u64,
    pub exit_rows: u64,
    pub iterations: u64,
    pub live_locations: Vec<usize>,
}
pub(super) struct LoopPlan {
    proof: Proof,
    carried: BTreeMap<usize, Word>,
    active: Bit,
    base: Eval,
    prefix: Layout,
    iteration: Layout,
    commit: Layout,
}
impl LoopPlan {
    pub fn prepare(plan: &Plan, source: &source::Source) -> Option<(machine::Machine, Self)> {
        let proof = proof::recognize(plan, source)?;
        let mut b = Build::new(source.interface.len());
        let pc = b.alloc(1); // coherent halted bit
        let code = b.alloc(source.fault.len());
        let result = b.alloc(source.target.len());
        let carried = proof
            .live
            .iter()
            .map(|id| (*id, b.alloc(plan.locations()[*id].width)))
            .collect();
        let active = b.alloc(1)[0].clone();
        let machine = machine::Machine {
            banks: BTreeMap::new(),
            tags: BTreeMap::new(),
            pc,
            code,
            result,
            stack: vec![],
            fixed: b.next,
            halt: 1,
        };
        let mut loops = Self {
            proof,
            carried,
            active,
            base: Eval::new(plan, source),
            prefix: Layout::default(),
            iteration: Layout::default(),
            commit: Layout::default(),
        };
        let (built, base) = loops.prologue(plan, source, &machine);
        loops.prefix = replay::layout(&built, machine.fixed);
        drop(built);
        loops.base = base;
        let start = machine.fixed + loops.prefix.alloc;
        loops.iteration = replay::layout(&loops.iteration(source, &machine, start), start);
        loops.commit = replay::layout(&loops.commit(source, &machine, start), start);
        Some((machine, loops))
    }
    pub fn info(&self) -> &LoopInfo {
        &self.proof.info
    }
    fn outcome(
        &self,
        source: &source::Source,
        machine: &machine::Machine,
        eval: &Eval,
        guard: &Bit,
        b: &mut Build,
    ) {
        let (payload, code) = eval.outcome.as_ref().expect("proved terminal path");
        assert_eq!(payload.len(), source.target.len());
        b.copy(payload, &machine.result, &[guard.clone()]);
        b.copy(code, &machine.code, &[guard.clone()]);
        b.flip(&machine.pc[0], &[guard.clone()]);
    }
    fn prologue(
        &self,
        plan: &Plan,
        source: &source::Source,
        machine: &machine::Machine,
    ) -> (Build, Eval) {
        let mut b = Build::new(machine.fixed);
        let mut base = None;
        for (index, path) in self.proof.prefix.iter().enumerate() {
            let mut e = Eval::new(plan, source);
            e.path(source, path, &mut b);
            if index == self.proof.entry {
                for (id, dst) in &self.carried {
                    b.copy(&e.env[*id], dst, &[]);
                }
                // Too short for entry: no terminal opportunity will be unrolled.
                b.flip(&self.active, &[e.guard.clone()]);
                base = Some(e);
            } else if path.len() as u64 <= plan.binding().budget.steps {
                self.outcome(source, machine, &e, &e.guard, &mut b);
            }
        }
        (b, base.expect("unique entry path"))
    }
    fn iteration(
        &self,
        source: &source::Source,
        machine: &machine::Machine,
        start: usize,
    ) -> Build {
        let mut b = Build::new(start);
        let mut e = self.base.clone();
        let mut snapshots = BTreeMap::new();
        for (id, word) in &self.carried {
            let old = b.alloc(word.len());
            b.copy(word, &old, &[]);
            e.env[*id] = old.clone();
            snapshots.insert(*id, old);
        }
        e.path(source, &self.proof.body, &mut b);
        let branch = self.proof.body.last().expect("loop branch");
        let condition = e.read(source, &source.rows[branch.key.pc].operands[0])[0].clone();
        let exiting = if self.proof.exit_when {
            condition
        } else {
            not(&condition)
        };
        let done = b.and(&self.active, &exiting);
        let next_active = b.and(&self.active, &not(&exiting));
        let mut exit = e.clone();
        exit.path(source, &self.proof.exiting, &mut b);
        self.outcome(source, machine, &exit, &done, &mut b);
        e.path(source, &self.proof.continuing, &mut b);
        // All RHSs refer to immutable snapshots or SSA temporaries. Clearing
        // the fixed words first therefore preserves arbitrary parallel aliases.
        for (id, word) in &self.carried {
            b.copy(&snapshots[id], word, &[]);
            b.copy(&e.env[*id], word, &[]);
        }
        // A fresh retained decision is needed even when and() aliases a bit.
        let next = b.alloc(1)[0].clone();
        b.copy(&vec![next_active], &vec![next.clone()], &[]);
        b.exchange(&self.active, &next, &[]);
        b
    }
    fn commit(&self, source: &source::Source, machine: &machine::Machine, start: usize) -> Build {
        let mut b = Build::new(start);
        b.copy(
            &machine.result,
            &source.target.clone().map(|q| Bit::Q(q, true)).collect(),
            &[],
        );
        b.copy(
            &machine.code,
            &source.fault.clone().map(|q| Bit::Q(q, true)).collect(),
            &[],
        );
        b.copy(
            &constant(1, source.fault.len()),
            &source.fault.clone().map(|q| Bit::Q(q, true)).collect(),
            &[not(&machine.pc[0])],
        );
        b
    }
    pub fn measure(&self, plan: &Plan, machine: &machine::Machine) -> Counts {
        let n = u128::from(self.info().iterations);
        let forward = self.prefix.gates as u128 + n * self.iteration.gates as u128;
        let history_wires = self.prefix.alloc as u128 + n * self.iteration.alloc as u128;
        let mut histogram = BTreeMap::new();
        let mut kernels = Kernels::default();
        for (l, weight) in [(&self.prefix, 1), (&self.iteration, n)] {
            for (arity, count) in &l.hist {
                *histogram.entry(*arity).or_default() += 2 * weight * count;
            }
            kernels.constant_multiplications += weight * l.kernels.constant_multiplications;
            kernels.generic_multiplications += weight * l.kernels.generic_multiplications;
            kernels.fixed_shifts += weight * l.kernels.fixed_shifts;
            kernels.variable_shifts += weight * l.kernels.variable_shifts;
            kernels.equalities += weight * l.kernels.equalities;
            kernels.ordered_comparisons += weight * l.kernels.ordered_comparisons;
        }
        for (arity, count) in &self.commit.hist {
            *histogram.entry(*arity).or_default() += count;
        }
        Counts {
            forward,
            commit: self.commit.gates as u128,
            full: 2 * forward + self.commit.gates as u128,
            max_controls: histogram
                .iter()
                .filter(|(_, n)| **n > 0)
                .map(|(k, _)| *k)
                .max()
                .unwrap_or(0),
            histogram,
            qubits: machine.fixed as u128 + history_wires + self.commit.alloc as u128,
            history_wires,
            peak_gate_objects: [self.prefix.gates, self.iteration.gates, self.commit.gates]
                .into_iter()
                .max()
                .unwrap(),
            layer_descriptors: 3,
            source_row_contexts: plan
                .layers()
                .expanded_row_contexts(plan.binding().budget.steps, plan.rows().len()),
            kernels,
        }
    }
    pub fn forward_gates_at(&self, step: u64) -> usize {
        // Macro gates are charged at the original terminal opportunity. This
        // reports a schedule, not a new source-budget unit.
        let info = self.info();
        let first = info.entry_rows + info.exit_rows - 1;
        let prefix = if step == 0 { self.prefix.gates } else { 0 };
        prefix
            + if step >= first
                && (step - first).is_multiple_of(info.continue_rows)
                && (step - first) / info.continue_rows < info.iterations
            {
                self.iteration.gates
            } else {
                0
            }
    }
    pub fn visit(
        &self,
        plan: &Plan,
        source: &source::Source,
        machine: &machine::Machine,
        visitor: &mut impl FnMut(&Flip) -> Result<()>,
    ) -> Result<()> {
        let (b, _) = self.prologue(plan, source, machine);
        emit(&b, &self.prefix, machine.fixed, false, visitor)?;
        drop(b);
        let first = machine.fixed as u128 + self.prefix.alloc as u128;
        for i in 0..self.info().iterations {
            self.visit_iteration(source, machine, first, i, false, visitor)?;
        }
        let last = first + u128::from(self.info().iterations) * self.iteration.alloc as u128;
        let start = usize::try_from(last).map_err(|_| Error("host limit: loop offset".into()))?;
        emit(
            &self.commit(source, machine, start),
            &self.commit,
            start,
            false,
            visitor,
        )?;
        for i in (0..self.info().iterations).rev() {
            self.visit_iteration(source, machine, first, i, true, visitor)?;
        }
        // Drop the initial prologue before replay: peak resident objects is the
        // largest one recipe, not the sum of prologue and iteration objects.
        let (b, _) = self.prologue(plan, source, machine);
        emit(&b, &self.prefix, machine.fixed, true, visitor)
    }
    fn visit_iteration(
        &self,
        source: &source::Source,
        machine: &machine::Machine,
        first: u128,
        i: u64,
        inverse: bool,
        visitor: &mut impl FnMut(&Flip) -> Result<()>,
    ) -> Result<()> {
        let start = usize::try_from(first + u128::from(i) * self.iteration.alloc as u128)
            .map_err(|_| Error("host limit: loop offset".into()))?;
        emit(
            &self.iteration(source, machine, start),
            &self.iteration,
            start,
            inverse,
            visitor,
        )
    }
}
fn emit(
    b: &Build,
    expected: &Layout,
    start: usize,
    inverse: bool,
    visitor: &mut impl FnMut(&Flip) -> Result<()>,
) -> Result<()> {
    assert_eq!(b.next - start, expected.alloc, "loop allocation changed");
    assert_eq!(b.gates.len(), expected.gates, "loop gate count changed");
    if inverse {
        for g in b.gates.iter().rev() {
            visitor(g)?;
        }
    } else {
        for g in &b.gates {
            visitor(g)?;
        }
    }
    Ok(())
}
