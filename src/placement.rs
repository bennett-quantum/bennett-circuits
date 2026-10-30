//! M3 first slice: deterministic clean-workspace interval placement.
//!
//! Gate order is unchanged. A validated clean wire starts and ends at zero;
//! its first/last gate uses therefore bound a safe live interval. Disjoint
//! intervals may share a slot if their requested allocation classes agree.
//! Interfaces, borrowed wires and local regions keep dedicated slots. This
//! is target-independent placement, not class resolution or §11 scheduling.
use crate::{
    Error, Result, bail,
    flatten::{Flat, flatten_allocations},
    ir::WsKind,
    object::Object,
    oracle::Argument,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRange {
    pub logical: usize,
    pub physical: usize,
    /// Half-open gate-index interval. A gate's controls are uses too.
    pub start: usize,
    pub end: usize,
    pub kind: WsKind,
    /// Requested class; `any` is unresolved until target assignment.
    pub class: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub flat: Flat,
    /// None means an unused clean logical wire requires no physical slot.
    pub logical_to_physical: Vec<Option<usize>>,
    pub lifetimes: Vec<LiveRange>,
}

/// Validate and specialise an action, then place its private clean workspace.
/// Validation is mandatory: reuse relies on the §6 clean-return proof, not a
/// user assertion that arbitrary raw IR is clean. Schedules cannot be ignored.
pub fn place_action(object: &Object, action: &str, args: &[Argument]) -> Result<Placement> {
    if object.schedule.is_some() {
        bail!(
            "E_SCHEDULE_INFEASIBLE: unimplemented: §11 scheduled placement requires the unpublished expansion library b-isa-spec-8r4 (milestone 3)"
        );
    }
    object.check()?;
    let f = object
        .function(action)
        .ok_or_else(|| Error(format!("placement: unknown action @{action}")))?;
    if !f.is_action() {
        bail!("placement: @{action} is not an action (§5.3)");
    }
    let program = crate::lower::lower_program_with_args(object, action, args)?;
    place_validated_program(&program)
}
fn place_validated_program(program: &Program) -> Result<Placement> {
    let (mut flat, _, allocations) = flatten_allocations(program)?;
    let mut uses: Vec<Option<(usize, usize)>> = vec![None; flat.num_qubits];
    for (index, gate) in flat.gates.iter().enumerate() {
        for q in gate
            .targets
            .iter()
            .chain(gate.controls.iter().map(|(q, _)| q))
        {
            let interval = uses
                .get_mut(*q)
                .ok_or_else(|| Error("placement: gate wire outside allocation".into()))?;
            match interval {
                Some((_, end)) => *end = index + 1,
                None => *interval = Some((index, index + 1)),
            }
        }
    }
    let mut mapping = vec![None; flat.num_qubits];
    for q in &flat.interface {
        mapping[*q] = Some(*q);
    }
    let mut next = flat.interface.len();
    let mut lifetimes = Vec::new();
    let mut clean = Vec::new();
    for allocation in allocations {
        for q in allocation.wires {
            if allocation.kind == WsKind::Clean {
                if let Some((start, end)) = uses[q] {
                    clean.push((start, q, end, allocation.class.clone()));
                }
            } else {
                mapping[q] = Some(next);
                lifetimes.push(LiveRange {
                    logical: q,
                    physical: next,
                    start: 0,
                    end: flat.gates.len(),
                    kind: allocation.kind,
                    class: allocation.class.clone(),
                });
                next += 1;
            }
        }
    }
    clean.sort();
    // A physical slot has one requested class for its entire lifetime.
    let mut free: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut active: BTreeMap<(usize, usize), (String, usize)> = BTreeMap::new();
    for (start, q, end, class) in clean {
        while let Some((&key, _)) = active.first_key_value() {
            if key.0 > start {
                break;
            }
            let (released_class, physical) = active.remove(&key).expect("active first key");
            free.entry(released_class).or_default().insert(physical);
        }
        let physical = if let Some(slot) = free.entry(class.clone()).or_default().pop_first() {
            slot
        } else {
            let slot = next;
            next += 1;
            slot
        };
        mapping[q] = Some(physical);
        active.insert((end, q), (class.clone(), physical));
        lifetimes.push(LiveRange {
            logical: q,
            physical,
            start,
            end,
            kind: WsKind::Clean,
            class,
        });
    }
    for gate in &mut flat.gates {
        let map_wire = |q: &mut usize| -> Result<()> {
            *q = mapping
                .get(*q)
                .copied()
                .flatten()
                .ok_or_else(|| Error("placement: used wire has no physical slot".into()))?;
            Ok(())
        };
        for q in &mut gate.targets {
            map_wire(q)?;
        }
        for (q, _) in &mut gate.controls {
            map_wire(q)?;
        }
    }
    flat.num_qubits = next;
    flat.workspace = mapping
        .iter()
        .skip(flat.interface.len())
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    lifetimes.sort_by_key(|l| l.logical);
    Ok(Placement {
        flat,
        logical_to_physical: mapping,
        lifetimes,
    })
}

use crate::{
    ir::Program,
    json::Value,
    target::{Leaf, Rational, Target},
};
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// Internal flattened gate index, never a §11 generated ordinal.
    pub source_gate: usize,
    pub start: Rational,
    pub end: Rational,
    pub leaf: Option<Leaf>,
    pub qram: Option<crate::flatten::MemoryOp>,
    pub costs: BTreeMap<String, Rational>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct TimedWire {
    pub wire: usize,
    pub class: String,
    pub start: Rational,
    pub end: Rational,
    pub dirty: bool,
}
/// Attached source proof material is retained verbatim, not promoted to a new
/// ISA certificate or detached from its original object/closure bindings.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub flat: Flat,
    pub placement: Placement,
    pub source: Program,
    pub request: Value,
    pub object_hash: String,
    pub closure_hash: String,
    pub target_hash: String,
    pub certificate: Option<Value>,
    pub classes: Vec<String>,
    pub events: Vec<Event>,
    pub wires: Vec<TimedWire>,
    /// Count and time resources; occupancy/peak are per class below.
    pub resources: BTreeMap<String, Rational>,
    pub per_class: BTreeMap<String, BTreeMap<String, Rational>>,
}

/// Default exact, sequential §8 placement. Supplied schedules require their
/// published expansion library; they are never discarded or reinterpreted.
pub fn place_on_target(
    object: &Object,
    action: &str,
    args: &[Argument],
    target: &Target,
) -> Result<Placed> {
    let placement = place_action(object, action, args)?;
    let source = crate::lower::lower_program_with_args(object, action, args)?;
    let mut placed = place_target_program(object, action, target, source, placement)?;
    placed.request = request(action, args, None);
    Ok(placed)
}
pub fn place_oracle_on_target(
    object: &Object,
    name: &str,
    budget: crate::oracle::Budget,
    args: &[Argument],
    target: &Target,
) -> Result<Placed> {
    if object.schedule.is_some() {
        bail!(
            "E_SCHEDULE_INFEASIBLE: unimplemented: §11 expansion library b-isa-spec-8r4 (milestone 3)"
        );
    }
    object.check()?;
    let source = crate::oracle::specialise(object, name, budget, args)?;
    let placement = place_validated_program(&source)?;
    let mut placed = place_target_program(object, name, target, source, placement)?;
    placed.request = request(name, args, Some(budget));
    Ok(placed)
}
fn place_target_program(
    object: &Object,
    action: &str,
    target: &Target,
    source: Program,
    placement: Placement,
) -> Result<Placed> {
    let report = object.hash()?;
    let (_, _, _, contexts) = crate::flatten::flatten_contexts(&source)?;
    if contexts.own.len() != placement.flat.gates.len() {
        bail!("placement: gate context trace length mismatch");
    }
    let ptr = object.header.ptr_width;
    if !crate::target::array(crate::target::field(
        &target.document,
        "ptr_widths_supported",
    )?)?
    .iter()
    .any(|v| v.as_u64() == Some(u64::from(ptr)))
    {
        bail!("§8.3 unsupported pointer width {ptr}");
    }
    let n = placement.flat.gates.len();
    let mut requests = vec!["any".to_string(); placement.flat.num_qubits];
    let mut intervals = vec![Vec::<(usize, usize, bool)>::new(); requests.len()];
    for q in &placement.flat.interface {
        intervals[*q].push((0, n.max(1), false));
    }
    for l in &placement.lifetimes {
        requests[l.physical] = l.class.clone();
        intervals[l.physical].push((l.start, l.end.max(l.start + 1), l.kind == WsKind::Borrowed));
    }
    let mut classes = vec![String::new(); requests.len()];
    // Default allocation uses the first JCS class with capacity for the whole
    // validated interval. Peak, not the count of distinct historical slots.
    let mut assigned: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for q in 0..requests.len() {
        let mut names = target.classes.keys().collect::<Vec<_>>();
        names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        let mut chosen = None;
        for name in names {
            if requests[q] != "any" && &requests[q] != name {
                continue;
            }
            if target.classes[name].capacity.is_none() {
                chosen = Some(name.clone());
                break;
            }
            let mut spans = assigned.get(name).cloned().unwrap_or_default();
            spans.extend(intervals[q].iter().map(|(s, e, _)| (*s, *e)));
            let peak = peak_indices(&spans);
            if target.classes[name].capacity.is_none_or(|cap| peak <= cap) {
                chosen = Some(name.clone());
                break;
            }
        }
        let name=chosen.ok_or_else(||Error(format!("§8.4 capacity constraint is peak live qubits per class; cannot allocate wire {q} class {}",requests[q])))?;
        assigned
            .entry(name.clone())
            .or_default()
            .extend(intervals[q].iter().map(|(s, e, _)| (*s, *e)));
        classes[q] = name;
    }
    let caps = crate::target::field(&target.document, "capabilities")?;
    if crate::target::field(caps, "max_parallel")?.as_u64() == Some(0) && n != 0 {
        bail!("§8.2 max_parallel=0 forbids gates");
    }
    let mut execution_peaks = assigned
        .iter()
        .map(|(c, spans)| (c.clone(), peak_indices(spans)))
        .collect::<BTreeMap<_, _>>();
    let mut flat = placement.flat.clone();
    flat.gates.clear();
    let mut events = Vec::new();
    let mut clock = Rational::ZERO;
    let mut gate_starts = vec![clock; n];
    let mut gate_ends = vec![clock; n];
    let mut wires = Vec::new();
    let mut i = 0;
    while i < n {
        let g = &placement.flat.gates[i];
        let depth = g
            .controls
            .len()
            .checked_sub(contexts.own[i])
            .ok_or("placement: own controls exceed total")?;
        if crate::target::field(caps, "max_predicate_depth")?
            .as_u64()
            .is_some_and(|max| depth as u64 > max)
        {
            bail!("§8.3 max_predicate_depth exceeded");
        }

        let qram = if !crate::target::field(&target.document, "qram")?.is_null() {
            contexts.memory.iter().find(|m| m.start == i)
        } else {
            None
        };
        if let Some(m) = qram {
            let ctx = BTreeMap::from([
                ("a".into(), Rational::integer(i128::from(m.a))),
                ("w".into(), Rational::integer(i128::from(m.w))),
                ("c".into(), Rational::integer(m.controls as i128)),
            ]);
            let spec =
                crate::target::field(crate::target::field(&target.document, "qram")?, &m.op)?;
            let mut costs = BTreeMap::new();
            for (name, e) in crate::target::field(spec, "cost")?
                .as_object()
                .ok_or("§8.3 QRAM cost map")?
            {
                let cost = crate::target::evaluate(e, &ctx, &target.tables)?;
                if cost.numerator < 0 {
                    bail!("§8.1 costs MUST be nonnegative and finite");
                }
                costs.insert(name.clone(), cost);
            }
            let end = clock.checked_add(*costs.get("time").ok_or("§8.2 QRAM lacks time cost")?)?;
            for j in i..m.end {
                gate_starts[j] = clock;
                gate_ends[j] = end;
            }
            flat.gates
                .extend_from_slice(&placement.flat.gates[i..m.end]);
            events.push(Event {
                source_gate: i,
                start: clock,
                end,
                leaf: None,
                qram: Some(m.clone()),
                costs,
            });
            clock = end;
            i = m.end;
            continue;
        }
        let expanded = target.expand(g, contexts.own[i], &mut classes, Rational::ZERO)?;
        if !expanded.ancillae.is_empty() {
            for (name, class) in &target.classes {
                let base_live = intervals
                    .iter()
                    .enumerate()
                    .filter(|(q, spans)| {
                        classes[*q] == *name
                            && spans.iter().any(|(start, end, _)| *start <= i && i < *end)
                    })
                    .count();
                let ancilla_live = expanded
                    .ancillae
                    .iter()
                    .filter(|(_, c, _)| c == name)
                    .count();
                let peak = execution_peaks.entry(name.clone()).or_default();
                *peak = (*peak).max(base_live + ancilla_live);
                if class
                    .capacity
                    .is_some_and(|cap| base_live + ancilla_live > cap)
                {
                    bail!(
                        "§8.4 capacity constraint is peak live qubits per class: {name} needs {}",
                        base_live + ancilla_live
                    );
                }
            }
        }
        let start = clock;
        gate_starts[i] = clock;
        for leaf in expanded.leaves {
            let duration = *leaf.costs.get("time").ok_or("§8.2 gate lacks time cost")?;
            let end = clock.checked_add(duration)?;
            flat.gates.push(leaf.gate.clone());
            let costs = leaf.costs.clone();
            events.push(Event {
                source_gate: i,
                start: clock,
                end,
                leaf: Some(leaf),
                qram: None,
                costs,
            });
            clock = end;
        }
        for (q, class, dirty) in expanded.ancillae {
            wires.push(TimedWire {
                wire: q,
                class,
                start,
                end: clock,
                dirty,
            });
        }
        gate_ends[i] = clock;
        i += 1;
    }
    // Pack scheme ancillae of the same class/cleanliness across nonoverlapping
    // time intervals. Borrowed source wires are never taken as scratch.
    let base = placement.flat.num_qubits;
    let mut pool: Vec<(String, bool, Rational, usize)> = Vec::new();
    let mut remap = BTreeMap::new();
    for w in &mut wires {
        let slot = if let Some((_, _, end, q)) = pool.iter_mut().find(|(c, d, e, _)| {
            *c == w.class && *d == w.dirty && e.compare(w.start).is_ok_and(|o| !o.is_gt())
        }) {
            *end = w.end;
            *q
        } else {
            let q = base + pool.len();
            pool.push((w.class.clone(), w.dirty, w.end, q));
            q
        };
        remap.insert(w.wire, slot);
        w.wire = slot;
    }
    for g in &mut flat.gates {
        for q in g
            .targets
            .iter_mut()
            .chain(g.controls.iter_mut().map(|(q, _)| q))
        {
            if let Some(p) = remap.get(q) {
                *q = *p;
            }
        }
    }
    for e in &mut events {
        if let Some(leaf) = &mut e.leaf {
            for q in leaf
                .gate
                .targets
                .iter_mut()
                .chain(leaf.gate.controls.iter_mut().map(|(q, _)| q))
            {
                if let Some(p) = remap.get(q) {
                    *q = *p;
                }
            }
        }
    }
    classes.truncate(base);
    classes.extend(pool.iter().map(|(c, _, _, _)| c.clone()));
    flat.num_qubits = classes.len();
    flat.workspace.extend(base..classes.len());
    for (q, spans) in intervals.into_iter().enumerate() {
        for (s, e, dirty) in spans {
            wires.push(TimedWire {
                wire: q,
                class: classes[q].clone(),
                start: if s < n { gate_starts[s] } else { clock },
                end: if e > 0 && n > 0 {
                    gate_ends[e.min(n) - 1]
                } else {
                    clock
                },
                dirty,
            });
        }
    }
    let (resources, mut per_class) = aggregate(target, &events, &wires)?;
    for (name, resource) in &target.resources {
        if crate::target::field(resource, "kind")?.as_str() == Some("peak") {
            let per = per_class
                .get_mut(name)
                .ok_or("placement: missing peak resource")?;
            for (class, peak) in &execution_peaks {
                let value = per.entry(class.clone()).or_insert(Rational::ZERO);
                if value.compare(Rational::integer(*peak as i128))?.is_lt() {
                    *value = Rational::integer(*peak as i128);
                }
            }
        }
    }

    Ok(Placed {
        flat,
        placement,
        source,
        request: Value::Null,
        object_hash: report.object_hash,
        closure_hash: report.symbols[action]
            .closure_hash
            .clone()
            .ok_or("placement: missing closure")?,
        target_hash: target.hash.clone(),
        certificate: object.certificate.clone(),
        classes,
        events,
        wires,
        resources,
        per_class,
    })
}
fn peak_indices(spans: &[(usize, usize)]) -> usize {
    let mut points = Vec::new();
    for (s, e) in spans {
        points.push((*s, 1i64));
        points.push((*e, -1));
    }
    points.sort();
    let (mut live, mut peak) = (0, 0);
    for (_, d) in points {
        live += d;
        peak = peak.max(live);
    }
    peak as usize
}
type ResourceTotals = (
    BTreeMap<String, Rational>,
    BTreeMap<String, BTreeMap<String, Rational>>,
);
fn aggregate(target: &Target, events: &[Event], wires: &[TimedWire]) -> Result<ResourceTotals> {
    let mut values = BTreeMap::new();
    let mut by_class = BTreeMap::new();
    for (name, r) in &target.resources {
        let kind = crate::target::string(crate::target::field(r, "kind")?)?;
        if kind == "count" {
            let value = events.iter().try_fold(Rational::ZERO, |sum, e| {
                sum.checked_add(e.costs.get(name).copied().unwrap_or(Rational::ZERO))
            })?;
            values.insert(name.clone(), value);
        } else if kind == "time" {
            let time = events.iter().try_fold(Rational::ZERO, |t, e| {
                Ok::<_, Error>(if e.end.compare(t)?.is_gt() { e.end } else { t })
            })?;
            values.insert(name.clone(), time);
        } else {
            let mut per = BTreeMap::new();
            for (class, c) in &target.classes {
                let ws = wires
                    .iter()
                    .filter(|w| &w.class == class)
                    .collect::<Vec<_>>();
                let mut points = Vec::new();
                for w in &ws {
                    if w.start.compare(w.end)?.is_lt() {
                        points.push((w.start, 1i64, w.wire));
                        points.push((w.end, -1i64, w.wire));
                    }
                }
                let mut failure = None;
                points.sort_by(|a, b| match a.0.compare(b.0) {
                    Ok(o) => o.then(a.1.cmp(&b.1)),
                    Err(e) => {
                        failure = Some(e);
                        std::cmp::Ordering::Equal
                    }
                });
                if let Some(e) = failure {
                    return Err(e);
                }
                let mut live = BTreeMap::<usize, i64>::new();
                let mut peak = 0;
                for (_, delta, q) in points {
                    let count = live.entry(q).or_default();
                    *count += delta;
                    if *count == 0 {
                        live.remove(&q);
                    }
                    peak = peak.max(live.len());
                }
                if c.capacity.is_some_and(|cap| peak > cap) {
                    bail!(
                        "§8.4 capacity constraint is peak live qubits per class: {class} needs {peak}"
                    );
                }
                let value = if kind == "peak" {
                    Rational::integer(peak as i128)
                } else {
                    let rate = r
                        .get("rate")
                        .and_then(|r| r.get(class))
                        .map(|r| match r {
                            Value::Int(s) => Rational::parse(s),
                            _ => Err(Error("§8.2 rate must be numeric".into())),
                        })
                        .transpose()?
                        .unwrap_or(Rational::ONE);
                    ws.iter().try_fold(Rational::ZERO, |v, w| {
                        v.checked_add(
                            w.end
                                .checked_add(w.start.negated()?)?
                                .checked_mul(if w.dirty { c.dirty_rate } else { rate })?,
                        )
                    })?
                };
                per.insert(class.clone(), value);
            }
            by_class.insert(name.clone(), per);
        }
    }
    Ok((values, by_class))
}
impl Placed {
    pub fn annotations(&self) -> Result<Value> {
        fn s(x: impl ToString) -> Value {
            Value::Str(x.to_string())
        }
        let resources = self
            .resources
            .iter()
            .map(|(n, r)| Ok((n.clone(), s(r.exact_text()))))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let per = self
            .per_class
            .iter()
            .map(|(n, cs)| {
                Ok((
                    n.clone(),
                    Value::Object(
                        cs.iter()
                            .map(|(c, r)| Ok((c.clone(), s(r.exact_text()))))
                            .collect::<Result<_>>()?,
                    ),
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        Ok(Value::Object(BTreeMap::from([
            ("format".into(), s("bennett-circuits-placement-1")),
            ("object_hash".into(), s(&self.object_hash)),
            ("closure_hash".into(), s(&self.closure_hash)),
            ("target_hash".into(), s(&self.target_hash)),
            ("source_ir".into(), crate::ir_codec::encode(&self.source)),
            ("request".into(),self.request.clone()),
            ("obligations".into(),Value::Array(["§5.2/§5.3 full-domain interface equivalence; source proof obligations retained", "§5.6 borrowed-system identity, including external entanglement", "§8.3 scheme correctness is the target author's obligation", "§9.4/§12 frame certificate schema publication: bennett-circuits-4fa", "§12 new certificate generation remains milestone 4"].iter().map(|s|Value::Str((*s).into())).collect())),
            (
                "source_certificate".into(),
                self.certificate.clone().unwrap_or(Value::Null),
            ),
            (
                "proof_status".into(),
                s("source obligations retained; no new §12 certificate (milestone 4)"),
            ),
            (
                "classes".into(),
                Value::Array(self.classes.iter().map(s).collect()),
            ),
            (
                "logical_to_physical".into(),
                Value::Array(
                    self.placement
                        .logical_to_physical
                        .iter()
                        .map(|q| q.map_or(Value::Null, |q| Value::Int(q.to_string())))
                        .collect(),
                ),
            ),
            ("resources".into(), Value::Object(resources)),
            ("resources_per_class".into(), Value::Object(per)),
            (
                "events".into(),
                Value::Array(
                    self.events
                        .iter()
                        .map(|e| {
                            Ok(Value::Object(BTreeMap::from([
                                ("source_gate".into(), Value::Int(e.source_gate.to_string())),
                                ("start".into(), s(e.start.exact_text())),
                                ("end".into(), s(e.end.exact_text())),
                                (
                                    "op".into(),
                                    s(e.leaf.as_ref().map_or_else(
                                        || e.qram.as_ref().expect("event kind").op.as_str(),
                                        |l| l.op.as_str(),
                                    )),
                                ),
                                (
                                    "scheme".into(),
                                    e.leaf
                                        .as_ref()
                                        .and_then(|l| l.scheme.as_ref())
                                        .map_or(Value::Null, s),
                                ),
                            ])))
                        })
                        .collect::<Result<_>>()?,
                ),
            ),
            (
                "wire_lifetimes".into(),
                Value::Array(
                    self.wires
                        .iter()
                        .map(|w| {
                            Ok(Value::Object(BTreeMap::from([
                                ("wire".into(), Value::Int(w.wire.to_string())),
                                ("class".into(), s(&w.class)),
                                ("start".into(), s(w.start.exact_text())),
                                ("end".into(), s(w.end.exact_text())),
                                ("dirty".into(), Value::Bool(w.dirty)),
                            ])))
                        })
                        .collect::<Result<_>>()?,
                ),
            ),
        ])))
    }
}

fn request(name: &str, args: &[Argument], budget: Option<crate::oracle::Budget>) -> Value {
    fn budget_value(b: crate::oracle::Budget) -> Value {
        Value::Array(
            [b.steps, b.heap, b.stack, b.depth]
                .iter()
                .map(|n| Value::Str(n.to_string()))
                .collect(),
        )
    }
    Value::Object(BTreeMap::from([
        ("region".into(), Value::Str(name.into())),
        ("epsilon".into(), Value::Str("0".into())),
        ("budget".into(), budget.map_or(Value::Null, budget_value)),
        (
            "params".into(),
            Value::Array(
                args.iter()
                    .map(|a| {
                        Value::Object(match a {
                            Argument::Integer(n) => {
                                BTreeMap::from([("integer".into(), Value::Str(n.clone()))])
                            }
                            Argument::Float(n) => {
                                BTreeMap::from([("f64".into(), Value::Str(n.to_string()))])
                            }
                            Argument::Token(b) => {
                                BTreeMap::from([("token".into(), budget_value(*b))])
                            }
                        })
                    })
                    .collect(),
            ),
        ),
    ]))
}
