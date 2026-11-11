//! Conservative CFG proof. Failure selects the exact row machine.
use super::*;
use crate::oracle_plan::{Check, Event};
#[derive(Clone)]
pub(super) struct Step {
    pub key: RowKey,
    pub branch: Option<bool>,
}
#[derive(Clone)]
pub(super) struct Proof {
    pub prefix: Vec<Vec<Step>>,
    pub entry: usize,
    pub body: Vec<Step>,
    pub continuing: Vec<Step>,
    pub exiting: Vec<Step>,
    pub exit_when: bool,
    pub live: Vec<usize>,
    pub info: LoopInfo,
}
fn root() -> RowKey {
    RowKey {
        pc: 0,
        calls: vec![],
    }
}
fn targets(plan: &Plan, key: &RowKey) -> Vec<RowKey> {
    plan.nodes()[key]
        .successors
        .iter()
        .filter_map(|e| e.target.clone())
        .collect()
}
fn backedges(
    plan: &Plan,
    key: RowKey,
    seen: &mut BTreeSet<RowKey>,
    stack: &mut BTreeSet<RowKey>,
    found: &mut BTreeSet<(RowKey, RowKey)>,
) {
    if !seen.insert(key.clone()) {
        return;
    }
    stack.insert(key.clone());
    for next in targets(plan, &key) {
        if stack.contains(&next) {
            found.insert((key.clone(), next));
        } else {
            backedges(plan, next, seen, stack, found);
        }
    }
    stack.remove(&key);
}
fn pure(i: &source::Instruction) -> bool {
    matches!(
        i.op.as_str(),
        "mov"
            | "parmov"
            | "label"
            | "select"
            | "add"
            | "sub"
            | "mul"
            | "and"
            | "or"
            | "xor"
            | "shl"
            | "lshr"
            | "ashr"
    ) || i.op.starts_with("const.i")
        || i.op.starts_with("zext.i")
        || i.op.starts_with("sext.i")
        || i.op.starts_with("trunc.i")
        || i.op.starts_with("icmp.")
}
fn safe(plan: &Plan, source: &source::Source, key: &RowKey, cycle: bool) -> bool {
    let node = &plan.nodes()[key];
    let i = &source.rows[key.pc];
    if node.source_charge != 1 || node.general_kernel {
        return false;
    }
    if pure(i) || i.op == "qbranch" || i.op == "qjump" || i.op == "qjumpr" {
        return node
            .successors
            .iter()
            .all(|e| e.fault.is_none() && e.target.is_some());
    }
    if cycle {
        return false;
    }
    match i.op.as_str() {
        "call" => matches!(
            node.event,
            Event::Call {
                check: Check::Safe,
                ..
            }
        ),
        "ret" => !key.calls.is_empty() && node.successors.iter().all(|e| e.fault.is_none()),
        "retval" => true,
        "fault" => matches!(i.operands.first(), Some(Read::Integer(_))),
        _ => false,
    }
}
fn next(plan: &Plan, key: &RowKey) -> Option<Option<RowKey>> {
    let edges = &plan.nodes()[key].successors;
    (edges.len() == 1).then(|| edges[0].target.clone())
}
fn branch(source: &source::Source, key: &RowKey, value: bool) -> Option<RowKey> {
    let Read::Label(pc) = source.rows[key.pc].operands[if value { 1 } else { 2 }] else {
        return None;
    };
    Some(RowKey {
        pc,
        calls: key.calls.clone(),
    })
}
/// A straight tail ends either at the header (continue), or at a terminal row.
fn tail(
    plan: &Plan,
    source: &source::Source,
    mut key: RowKey,
    header: &RowKey,
) -> Option<(Vec<Step>, bool)> {
    let mut out = vec![];
    let mut seen = BTreeSet::new();
    while &key != header {
        if !seen.insert(key.clone())
            || !safe(plan, source, &key, false)
            || source.rows[key.pc].op == "qbranch"
            || source.rows[key.pc].op == "call"
        {
            return None;
        }
        out.push(Step {
            key: key.clone(),
            branch: None,
        });
        let Some(k) = next(plan, &key)? else {
            return Some((out, false));
        };
        key = k;
    }
    Some((out, true))
}
fn prefixes(
    plan: &Plan,
    source: &source::Source,
    key: RowKey,
    header: &RowKey,
    path: &mut Vec<Step>,
    out: &mut Vec<Vec<Step>>,
) -> Option<()> {
    // A proof-size bound selects fallback; it never limits executable source.
    if out.len() >= 64 || path.iter().any(|s| s.key == key) {
        return None;
    }
    if &key == header {
        out.push(path.clone());
        return Some(());
    }
    if !safe(plan, source, &key, false) {
        return None;
    }
    if source.rows[key.pc].op == "qbranch" {
        for value in [true, false] {
            let target = branch(source, &key, value)?;
            if !targets(plan, &key).contains(&target) {
                continue;
            }
            path.push(Step {
                key: key.clone(),
                branch: Some(value),
            });
            prefixes(plan, source, target, header, path, out)?;
            path.pop();
        }
    } else {
        path.push(Step {
            key: key.clone(),
            branch: None,
        });
        if let Some(k) = next(plan, &key)? {
            prefixes(plan, source, k, header, path, out)?;
        } else {
            out.push(path.clone());
        }
        path.pop();
    }
    Some(())
}
fn reads_before_writes(
    source: &source::Source,
    path: impl Iterator<Item = Step>,
) -> BTreeSet<usize> {
    let mut defined: BTreeSet<usize> = BTreeSet::new();
    let mut live = BTreeSet::new();
    for step in path {
        let i = &source.rows[step.key.pc];
        live.extend(
            i.operands
                .iter()
                .flat_map(Read::locations)
                .filter(|id| !defined.contains(id)),
        );
        defined.extend(i.lhs.iter().copied());
    }
    live
}
pub(super) fn recognize(plan: &Plan, source: &source::Source) -> Option<Proof> {
    if plan.nodes().len() > 256 {
        return None;
    }
    let mut edges = BTreeSet::new();
    backedges(
        plan,
        root(),
        &mut BTreeSet::new(),
        &mut BTreeSet::new(),
        &mut edges,
    );
    if edges.len() != 1 {
        return None;
    }
    let (_, header) = edges.into_iter().next()?;
    let mut body = vec![];
    let mut key = header.clone();
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(key.clone()) || !safe(plan, source, &key, true) {
            return None;
        }
        body.push(Step {
            key: key.clone(),
            branch: None,
        });
        if source.rows[key.pc].op == "qbranch" {
            break;
        }
        key = next(plan, &key)??;
    }
    if [true, false]
        .into_iter()
        .any(|v| branch(source, &key, v).is_none_or(|k| !targets(plan, &key).contains(&k)))
    {
        return None;
    }
    let (yes, yes_back) = tail(plan, source, branch(source, &key, true)?, &header)?;
    let (no, no_back) = tail(plan, source, branch(source, &key, false)?, &header)?;
    if yes_back == no_back {
        return None;
    }
    let (continuing, exiting, exit_when) = if yes_back {
        (yes, no, false)
    } else {
        (no, yes, true)
    };
    if continuing.iter().any(|s| !safe(plan, source, &s.key, true)) {
        return None;
    }
    let cycle: BTreeSet<_> = body
        .iter()
        .chain(&continuing)
        .map(|s| s.key.clone())
        .collect();
    // Header dominates the whole cycle: no other entry may reach it.
    let mut pending = vec![root()];
    let mut visited = BTreeSet::new();
    while let Some(k) = pending.pop() {
        if k == header || !visited.insert(k.clone()) {
            continue;
        }
        if cycle.contains(&k) {
            return None;
        }
        pending.extend(targets(plan, &k));
    }
    let mut prefix = vec![];
    prefixes(plan, source, root(), &header, &mut vec![], &mut prefix)?;
    let entries: Vec<_> = prefix
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.last().and_then(|s| next(plan, &s.key)).flatten().as_ref() == Some(&header)
                || (p.is_empty() && header == root())
        })
        .map(|(i, _)| i)
        .collect();
    // A branch directly to header has two successors, so use its recorded arm.
    let entries: Vec<_> = prefix
        .iter()
        .enumerate()
        .filter(|(i, p)| {
            entries.contains(i)
                || p.last().is_some_and(|s| {
                    s.branch.and_then(|v| branch(source, &s.key, v)).as_ref() == Some(&header)
                })
        })
        .map(|(i, _)| i)
        .collect();
    if entries.len() != 1 {
        return None;
    }
    let entry = entries[0];
    let written: BTreeSet<_> = body
        .iter()
        .chain(&continuing)
        .flat_map(|s| source.rows[s.key.pc].lhs.iter().copied())
        .collect();
    let mut live = reads_before_writes(source, body.iter().chain(&continuing).cloned());
    live.extend(reads_before_writes(
        source,
        body.iter().chain(&exiting).cloned(),
    ));
    let live: Vec<_> = live.intersection(&written).copied().collect();
    let entry_rows = prefix[entry].len() as u64;
    let continue_rows = (body.len() + continuing.len()) as u64;
    let exit_rows = (body.len() + exiting.len()) as u64;
    let steps = plan.binding().budget.steps;
    let iterations = steps
        .checked_sub(entry_rows + exit_rows)
        .map_or(0, |rest| 1 + rest / continue_rows);
    let info = LoopInfo {
        header: header.pc,
        entry_rows,
        continue_rows,
        exit_rows,
        iterations,
        live_locations: live.clone(),
    };
    Some(Proof {
        prefix,
        entry,
        body,
        continuing,
        exiting,
        exit_when,
        live,
        info,
    })
}
