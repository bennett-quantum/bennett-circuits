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
            "unimplemented: §11 scheduled placement requires the unpublished expansion library b-isa-spec-8r4 (milestone 3)"
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
    let (mut flat, _, allocations) = flatten_allocations(&program)?;
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
