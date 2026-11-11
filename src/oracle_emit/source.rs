//! One-time symbol/location indexing; recipes contain no per-gate name lookup.
use super::*;
use crate::object::{FnKind, Instr, Operand};
use std::{ops::Range, sync::Arc};
pub(super) enum Read {
    Location(usize),
    Integer(String),
    List(Vec<usize>),
    Label(usize),
    Symbol(String),
    Other,
}
pub(super) struct Instruction {
    pub op: String,
    pub lhs: Vec<usize>,
    pub operands: Vec<Read>,
    pub locals: Arc<Vec<usize>>,
}
pub(super) struct Source {
    pub name: String,
    pub rows: Vec<Instruction>,
    pub entries: BTreeMap<String, usize>,
    pub parameters: BTreeMap<String, Vec<usize>>,
    pub inputs: BTreeMap<usize, Word>,
    pub interface: Vec<QubitRef>,
    pub bundles: Vec<Bundle>,
    pub target: Range<usize>,
    pub fault: Range<usize>,
    pub ptr_width: usize,
}
pub(super) fn eligible(plan: &Plan) -> bool {
    if plan.is_general() {
        return false;
    }
    plan.nodes().values().all(|node| {
        if node.general_kernel {
            return false;
        }
        let op = plan.rows()[node.key.pc]
            .opcode
            .strip_prefix("lift.")
            .unwrap_or(&plan.rows()[node.key.pc].opcode);
        matches!(
            op,
            "call"
                | "ret"
                | "retval"
                | "qjump"
                | "qbranch"
                | "fault"
                | "faultif"
                | "mov"
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
        ) || op.starts_with("const.i")
            || op.starts_with("zext.i")
            || op.starts_with("sext.i")
            || op.starts_with("trunc.i")
            || op.starts_with("icmp.")
            || (op == "qjumpr" && node.operands[0].as_u128().is_some())
    })
}
impl Source {
    pub fn new(obj: &Object, plan: &Plan) -> Self {
        let root = obj.function(&plan.binding().oracle).unwrap();
        let ids = plan
            .locations()
            .iter()
            .enumerate()
            .map(|(id, l)| ((l.function.clone(), l.name.clone()), id))
            .collect::<BTreeMap<_, _>>();
        let labels = labels(plan);
        let locals = locals(obj, &ids);
        let rows = index_rows(obj, plan, &ids, &labels, &locals);
        let mut source = Self {
            name: root.name.clone(),
            rows,
            entries: BTreeMap::new(),
            parameters: BTreeMap::new(),
            inputs: BTreeMap::new(),
            interface: vec![],
            bundles: vec![],
            target: 0..0,
            fault: 0..0,
            ptr_width: obj.header.ptr_width as usize,
        };
        source.index_interface(root, &ids);
        for row in plan.rows() {
            source.entries.entry(row.function.clone()).or_insert(row.pc);
        }
        for f in &obj.functions {
            if f.kind == FnKind::Lproc {
                source.parameters.insert(
                    f.name.clone(),
                    f.groups[0]
                        .iter()
                        .map(|p| ids[&(f.name.clone(), p.name.clone())])
                        .collect(),
                );
            }
        }
        source
    }
    fn index_interface(
        &mut self,
        root: &crate::object::Function,
        ids: &BTreeMap<(String, String), usize>,
    ) {
        for (group, role) in [(1, Role::Mut), (2, Role::Mut), (0, Role::In)] {
            let start = self.interface.len();
            for p in &root.groups[group] {
                let width = p.ty.wire_width().unwrap();
                let first = self.interface.len();
                self.interface
                    .extend((0..width).map(|i| QubitRef::new(&p.name, i)));
                self.bundles.push(Bundle {
                    name: p.name.clone(),
                    width,
                    role,
                });
                if group == 0 {
                    self.inputs.insert(
                        ids[&(root.name.clone(), p.name.clone())],
                        (first..self.interface.len())
                            .map(|q| Bit::Q(q, true))
                            .collect(),
                    );
                }
            }
            if group == 1 {
                self.target = start..self.interface.len();
            }
            if group == 2 {
                self.fault = start..self.interface.len();
            }
        }
    }
}
fn labels(plan: &Plan) -> BTreeMap<(String, String), usize> {
    let mut out = BTreeMap::new();
    for row in plan.rows() {
        out.entry((row.function.clone(), row.block.clone()))
            .or_insert(row.pc);
    }
    out
}
fn locals(
    obj: &Object,
    ids: &BTreeMap<(String, String), usize>,
) -> BTreeMap<String, Arc<Vec<usize>>> {
    obj.functions
        .iter()
        .map(|f| {
            let mut names = vec![];
            if f.kind == FnKind::Lproc {
                names.extend(
                    f.groups[0]
                        .iter()
                        .chain(&f.groups[1])
                        .map(|p| p.name.as_str()),
                );
            }
            names.extend(f.regs.iter().map(|p| p.name.as_str()));
            let selected = names
                .into_iter()
                .filter_map(|n| ids.get(&(f.name.clone(), n.into())).copied())
                .collect();
            (f.name.clone(), Arc::new(selected))
        })
        .collect()
}
fn index_rows(
    obj: &Object,
    plan: &Plan,
    ids: &BTreeMap<(String, String), usize>,
    labels: &BTreeMap<(String, String), usize>,
    locals: &BTreeMap<String, Arc<Vec<usize>>>,
) -> Vec<Instruction> {
    let mut ordinal = BTreeMap::new();
    plan.rows()
        .iter()
        .map(|row| {
            let block = obj
                .function(&row.function)
                .unwrap()
                .blocks
                .iter()
                .find(|b| b.name == row.block)
                .unwrap();
            let index = ordinal
                .entry((row.function.clone(), row.block.clone()))
                .or_insert(0usize);
            let i = &block.instrs[*index];
            *index += 1;
            instruction(i, &row.function, ids, labels, locals[&row.function].clone())
        })
        .collect()
}
fn instruction(
    i: &Instr,
    f: &str,
    ids: &BTreeMap<(String, String), usize>,
    labels: &BTreeMap<(String, String), usize>,
    locals: Arc<Vec<usize>>,
) -> Instruction {
    let operands = i
        .operands
        .iter()
        .map(|operand| match operand {
            Operand::Name(n) => ids
                .get(&(f.into(), n.clone()))
                .map_or(Read::Other, |id| Read::Location(*id)),
            Operand::List(ns) => Read::List(
                ns.iter()
                    .filter_map(|n| ids.get(&(f.into(), n.clone())).copied())
                    .collect(),
            ),
            Operand::Int(n) => Read::Integer(n.clone()),
            Operand::Label(n, _) => Read::Label(labels[&(f.into(), n.clone())]),
            Operand::Sym(n) => Read::Symbol(n.clone()),
            _ => Read::Other,
        })
        .collect();
    Instruction {
        op: i.opcode.strip_prefix("lift.").unwrap_or(&i.opcode).into(),
        lhs: i.lhs.iter().map(|n| ids[&(f.into(), n.clone())]).collect(),
        operands,
        locals,
    }
}
impl Read {
    pub fn locations(&self) -> Vec<usize> {
        match self {
            Self::Location(id) => vec![*id],
            Self::List(ids) => ids.clone(),
            _ => vec![],
        }
    }
}
