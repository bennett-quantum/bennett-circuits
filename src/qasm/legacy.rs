//! Lossless legacy transport without enclosing-circuit clones or flat arrays.
use super::*;
use local::Layout;
use std::io::Write;
struct Index<'a> {
    program: &'a Program,
    ids: BTreeMap<String, usize>,
    layouts: Vec<Layout>,
    order: Vec<usize>,
    widths: Vec<usize>,
}
impl<'a> Index<'a> {
    fn new(program: &'a Program) -> Result<Self> {
        let mut ids = BTreeMap::new();
        let mut layouts = vec![];
        for (i, c) in program.circuits.iter().enumerate() {
            if ids.insert(c.name.clone(), i).is_some() {
                bail!("QASM: duplicate circuit");
            }
            layouts.push(Layout::new(c)?);
        }
        if !ids.contains_key(&program.top) {
            bail!("QASM: missing top circuit");
        }
        let mut index = Self {
            program,
            ids,
            layouts,
            order: vec![],
            widths: vec![0; program.circuits.len()],
        };
        let mut active = BTreeSet::new();
        let mut seen = BTreeSet::new();
        for id in 0..program.circuits.len() {
            index.order(id, &mut active, &mut seen)?;
        }
        for &id in &index.order {
            let mut width = index.layouts[id].own;
            for node in &program.circuits[id].body {
                if let Node::Call {
                    callee,
                    controls,
                    args,
                    ..
                } = node
                {
                    let target = index.ids[callee];
                    index.call_wires(id, target, controls, args)?;
                    width = width
                        .checked_add(
                            index.widths[target]
                                - program.circuits[target].interface_wire_count() as usize,
                        )
                        .ok_or("host limit: QASM call workspace")?;
                } else {
                    index.layouts[id].visit(node, |_| Ok(()))?;
                }
            }
            index.widths[id] = width.max(1);
        }
        Ok(index)
    }
    fn order(
        &mut self,
        id: usize,
        active: &mut BTreeSet<usize>,
        seen: &mut BTreeSet<usize>,
    ) -> Result<()> {
        if active.contains(&id) {
            bail!("QASM: call cycle");
        }
        if seen.contains(&id) {
            return Ok(());
        }
        active.insert(id);
        for node in &self.program.circuits[id].body {
            if let Node::Call { callee, .. } = node {
                let child = *self.ids.get(callee).ok_or("QASM: unknown callee")?;
                self.order(child, active, seen)?;
            }
        }
        active.remove(&id);
        seen.insert(id);
        self.order.push(id);
        Ok(())
    }
    fn call_wires(
        &self,
        id: usize,
        target: usize,
        controls: &[Ctrl],
        args: &[String],
    ) -> Result<(Vec<(usize, bool)>, Vec<usize>)> {
        let formals = self.program.circuits[target].interface_bundles();
        if args.len() != formals.len() {
            bail!("QASM: call argument arity");
        }
        let mut wires = vec![];
        for (arg, (_, width)) in args.iter().zip(&formals) {
            let range = self.layouts[id].bundle(arg)?;
            if range.len() as u64 != *width {
                bail!("QASM: call width mismatch");
            }
            wires.extend(range);
        }
        Ok((self.layouts[id].controls(controls)?, wires))
    }
    fn circuit(&self, id: usize, writer: &mut dyn Write, stats: &mut WriteStats) -> Result<()> {
        let c = &self.program.circuits[id];
        let width = self.widths[id];
        put(writer, &format!("gate {} ", ident(id)))?;
        arguments(writer, 0..width, false)?;
        put(writer, " {\n")?;
        let mut next = self.layouts[id].own;
        for node in &c.body {
            if let Node::Call {
                callee,
                adjoint,
                controls,
                args,
            } = node
            {
                let target = self.ids[callee];
                let (cs, mut wires) = self.call_wires(id, target, controls, args)?;
                let extra = self.widths[target] - wires.len();
                wires.extend(next..next + extra);
                next += extra;
                put(writer, &invocation(&ident(target), &cs, &wires, *adjoint))?;
                stats.writer_visits += 1;
            } else {
                self.layouts[id].visit(node, |gate| {
                    put(
                        writer,
                        &invocation(
                            &ir_codec::gate(gate.gate),
                            &gate.controls,
                            &gate.targets,
                            false,
                        ),
                    )?;
                    stats.writer_visits += 1;
                    Ok(())
                })?;
            }
        }
        put(writer, "}\n")
    }
}
/// Write the lossless hierarchical format. Validation precedes the first byte.
pub fn write<W: Write + ?Sized>(program: &Program, writer: &mut W) -> Result<WriteStats> {
    let index = Index::new(program)?;
    let mut counted = crate::export::CountingWrite::new(writer);
    let mut stats = WriteStats::default();
    stats.resident_descriptors =
        index.layouts.iter().map(|l| l.map.len()).sum::<usize>() + index.ids.len();
    put(
        &mut counted,
        "OPENQASM 3.0;\ninclude \"stdgates.inc\";\n// bc: ",
    )?;
    ir_codec::write(program, &mut counted)?;
    put(&mut counted, "\n")?;
    for id in &index.order {
        index.circuit(*id, &mut counted, &mut stats)?;
    }
    let top = index.ids[&program.top];
    let width = index.widths[top];
    put(&mut counted, &format!("qubit[{width}] q;\n{} ", ident(top)))?;
    arguments(&mut counted, 0..width, true)?;
    put(
        &mut counted,
        &format!(
            ";\n// interface: {}\n",
            program.circuits[top].interface_wire_count()
        ),
    )?;
    counted.flush().map_err(io_error)?;
    stats.bytes = counted.bytes;
    stats.peak_gate_objects = usize::from(stats.writer_visits > 0);
    Ok(stats)
}
fn arguments(writer: &mut dyn Write, wires: impl Iterator<Item = usize>, top: bool) -> Result<()> {
    for (i, q) in wires.enumerate() {
        if i != 0 {
            put(writer, ", ")?;
        }
        put(
            writer,
            &if top {
                format!("q[{q}]")
            } else {
                format!("v{q}")
            },
        )?;
    }
    Ok(())
}
