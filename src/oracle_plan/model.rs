use super::*;
use crate::{
    Error, bail,
    object::{FnKind, Function, Instr, Operand, Type},
};

pub(super) struct SourceRow {
    pub public: Row,
    pub instruction: Instr,
}
pub(super) struct Model {
    pub rows: Vec<SourceRow>,
    pub locations: Vec<Location>,
    pub initial: Vec<KnownBits>,
    pub entries: BTreeMap<String, usize>,
    pub locals: BTreeMap<String, Vec<usize>>,
    pub parameters: BTreeMap<String, Vec<usize>>,
    pub recursive: bool,
    ids: BTreeMap<(String, String), usize>,
    labels: BTreeMap<(String, String), usize>,
    ptr_width: usize,
}
impl Model {
    pub fn new(obj: &Object, name: &str, budget: Budget, args: &[Argument]) -> Result<Self> {
        obj.check()?;
        let root = obj
            .function(name)
            .ok_or_else(|| Error(format!("plan: unknown @{name}")))?;
        if root.kind != FnKind::Oracle || root.groups[4].len() != args.len() {
            bail!("plan: expected oracle with matching classical arguments")
        }
        validate_budget(obj, budget)?;
        let functions = functions(obj, root)?;
        let recursive = is_recursive(&functions);
        let mut model = Self {
            rows: vec![],
            locations: vec![],
            initial: vec![],
            entries: BTreeMap::new(),
            locals: BTreeMap::new(),
            parameters: BTreeMap::new(),
            recursive,
            ids: BTreeMap::new(),
            labels: BTreeMap::new(),
            ptr_width: obj.header.ptr_width as usize,
        };
        for function in &functions {
            model.index_function(function);
        }
        model.index_inputs(root, args)?;
        Ok(model)
    }
    fn index_function(&mut self, f: &Function) {
        self.entries.insert(f.name.clone(), self.rows.len());
        for block in &f.blocks {
            self.labels
                .insert((f.name.clone(), block.name.clone()), self.rows.len());
            for instruction in &block.instrs {
                self.rows.push(SourceRow {
                    public: Row {
                        pc: self.rows.len(),
                        function: f.name.clone(),
                        block: block.name.clone(),
                        opcode: instruction.opcode.clone(),
                    },
                    instruction: instruction.clone(),
                });
            }
        }
        let mut locals = vec![];
        if f.kind == FnKind::Lproc {
            for p in f.groups[0].iter().chain(&f.groups[1]) {
                locals.push(self.add_location(
                    &f.name,
                    &p.name,
                    p.ty.wire_width().unwrap() as usize,
                    false,
                ));
            }
        }
        for p in &f.regs {
            locals.push(self.add_location(
                &f.name,
                &p.name,
                p.ty.wire_width().unwrap() as usize,
                false,
            ));
        }
        self.parameters.insert(
            f.name.clone(),
            locals.iter().take(f.groups[0].len()).copied().collect(),
        );
        self.locals.insert(f.name.clone(), locals);
    }
    fn add_location(&mut self, function: &str, name: &str, width: usize, unknown: bool) -> usize {
        let id = self.locations.len();
        self.ids.insert((function.into(), name.into()), id);
        self.locations.push(Location {
            function: function.into(),
            name: name.into(),
            width,
        });
        self.initial.push(if unknown {
            KnownBits::unknown(width)
        } else {
            KnownBits::integer("0", width)
        });
        id
    }
    fn index_inputs(&mut self, root: &Function, args: &[Argument]) -> Result<()> {
        for p in &root.groups[0] {
            self.add_location(
                &root.name,
                &p.name,
                p.ty.wire_width().unwrap() as usize,
                true,
            );
        }
        for (p, a) in root.groups[4].iter().zip(args) {
            crate::lower::validate_arg(&p.ty, a)?;
            if let (Type::Ptr, Argument::Integer(s)) = (&p.ty, a) {
                crate::lower::validate_integer(s, self.ptr_width as u32)?;
            }
            let width = match p.ty {
                Type::Int(w) => w as usize,
                Type::Ptr => self.ptr_width,
                _ => continue,
            };
            let id = self.add_location(&root.name, &p.name, width, true);
            if let Argument::Integer(s) = a {
                self.initial[id] = KnownBits::integer(s, width);
            }
        }
        Ok(())
    }
    pub fn id(&self, function: &str, name: &str) -> usize {
        self.ids[&(function.into(), name.into())]
    }
    pub fn read(&self, function: &str, operand: &Operand, values: &[KnownBits]) -> KnownBits {
        match operand {
            Operand::Name(name) => self.ids.get(&(function.into(), name.clone())).map_or_else(
                || KnownBits::unknown(self.ptr_width),
                |id| values[*id].clone(),
            ),
            Operand::Int(text) => KnownBits::integer(text, self.ptr_width),
            _ => KnownBits::unknown(self.ptr_width),
        }
    }
    pub fn operands(&self, row: usize, values: &[KnownBits]) -> Vec<KnownBits> {
        let r = &self.rows[row];
        r.instruction
            .operands
            .iter()
            .flat_map(|op| match op {
                Operand::Name(_) | Operand::Int(_) => {
                    vec![self.read(&r.public.function, op, values)]
                }
                Operand::List(names) => names
                    .iter()
                    .map(|n| self.read(&r.public.function, &Operand::Name(n.clone()), values))
                    .collect(),
                _ => vec![],
            })
            .collect()
    }
    pub fn label(&self, function: &str, operand: &Operand) -> usize {
        let Operand::Label(name, _) = operand else {
            panic!("checked label")
        };
        self.labels[&(function.into(), name.clone())]
    }
    pub fn call_name(&self, row: usize) -> &str {
        let Operand::Sym(name) = &self.rows[row].instruction.operands[0] else {
            panic!("checked callee")
        };
        name
    }
}
fn validate_budget(obj: &Object, budget: Budget) -> Result<()> {
    if obj.layout.is_none() && (budget.heap != 0 || budget.stack != 0 || budget.depth != 0) {
        bail!("plan: memoryless budget limits must be zero (§5.4)")
    }
    if let Some(layout) = &obj.layout {
        let exponent = layout.get("S_cells").and_then(|v| v.as_u64()).unwrap();
        let heap = layout
            .get("heap_limit_max")
            .and_then(|v| v.as_u64())
            .unwrap();
        if (exponent < 64 && budget.stack > (1u64 << exponent)) || budget.heap > heap {
            bail!("plan: invalid budget (§5.4)")
        }
    }
    Ok(())
}
fn references(f: &Function) -> Vec<String> {
    f.blocks
        .iter()
        .flat_map(|b| &b.instrs)
        .filter_map(|i| match (i.opcode.as_str(), i.operands.first()) {
            ("lift.call", Some(Operand::Sym(name))) => Some(name.clone()),
            _ => None,
        })
        .collect()
}
fn functions<'a>(obj: &'a Object, root: &'a Function) -> Result<Vec<&'a Function>> {
    let mut queue = references(root);
    let mut found = BTreeMap::new();
    while let Some(name) = queue.pop() {
        if found.contains_key(&name) {
            continue;
        }
        let f = obj
            .function(&name)
            .ok_or_else(|| Error(format!("plan: unresolved @{name}")))?;
        queue.extend(references(f));
        found.insert(name, f);
    }
    let refs = found
        .iter()
        .map(|(n, f)| (n.clone(), references(f)))
        .collect();
    let defined = found.keys().map(String::as_str).collect();
    let hashes = obj.identity()?;
    let mut keys = BTreeMap::new();
    for group in crate::object::strongly_connected(&refs, &defined) {
        index_group(obj, &hashes, &group, &refs, &mut keys)?;
    }
    let mut procs = found.into_values().collect::<Vec<_>>();
    procs.sort_by_key(|f| keys[&f.name].clone());
    let mut result = vec![root];
    result.extend(procs);
    Ok(result)
}
fn index_group(
    obj: &Object,
    hashes: &BTreeMap<String, crate::object::SymbolHashes>,
    group: &[String],
    refs: &BTreeMap<String, Vec<String>>,
    keys: &mut BTreeMap<String, (String, usize)>,
) -> Result<()> {
    let cyclic = group.len() > 1 || refs[&group[0]].contains(&group[0]);
    let key = if cyclic {
        let mut bytes = vec![];
        for name in group {
            bytes.extend(obj.semantic_envelope(name)?);
        }
        crate::sha256::sha256_hex(&bytes)
    } else {
        hashes[&group[0]]
            .closure_hash
            .clone()
            .ok_or_else(|| Error("plan: unresolved closure".into()))?
    };
    for (index, name) in group.iter().enumerate() {
        keys.insert(name.clone(), (key.clone(), index));
    }
    Ok(())
}
fn is_recursive(functions: &[&Function]) -> bool {
    let refs = functions
        .iter()
        .map(|f| (f.name.clone(), references(f)))
        .collect::<BTreeMap<_, _>>();
    let defined = refs.keys().map(String::as_str).collect();
    crate::object::strongly_connected(&refs, &defined)
        .iter()
        .any(|g| g.len() > 1 || refs[&g[0]].contains(&g[0]))
}
