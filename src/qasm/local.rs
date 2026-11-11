//! A pre-indexed local circuit layout and lazy primitive expansion.
use super::*;
use std::ops::Range;
pub(super) struct Layout {
    pub map: BTreeMap<String, Range<usize>>,
    pub regions: BTreeMap<String, (u32, u32)>,
    pub own: usize,
}
impl Layout {
    pub fn new(c: &Circuit) -> Result<Self> {
        for m in &c.mem {
            if m.addr_width >= usize::BITS {
                bail!("host limit: QASM region address space");
            }
        }
        let mut map = BTreeMap::new();
        let mut own = 0usize;
        for (name, width) in c.interface_bundles().into_iter().chain(
            c.workspace
                .iter()
                .map(|w| (w.name.clone(), u64::from(w.width))),
        ) {
            let width = usize::try_from(width)
                .map_err(|_| Error("host limit: QASM bundle width".into()))?;
            let end = own
                .checked_add(width)
                .ok_or("host limit: QASM wire count")?;
            if map.insert(name, own..end).is_some() {
                bail!("QASM: duplicate bundle");
            }
            own = end;
        }
        let regions = c
            .mem
            .iter()
            .map(|m| (m.name.clone(), (m.addr_width, m.cell_width)))
            .chain(
                c.local_regions
                    .iter()
                    .map(|r| (r.name.clone(), (r.addr_width, r.cell_width))),
            )
            .collect();
        Ok(Self { map, regions, own })
    }
    pub fn wire(&self, q: &QubitRef) -> Result<usize> {
        let range = self
            .map
            .get(&q.bundle)
            .ok_or_else(|| Error(format!("QASM: unknown bundle {:?}", q.bundle)))?;
        if q.index as usize >= range.len() {
            bail!("QASM: wire outside bundle");
        }
        Ok(range.start + q.index as usize)
    }
    pub fn bundle(&self, name: &str) -> Result<Range<usize>> {
        self.map
            .get(name)
            .cloned()
            .ok_or_else(|| Error(format!("QASM: unknown bundle {name:?}")))
    }
    pub fn controls(&self, controls: &[Ctrl]) -> Result<Vec<(usize, bool)>> {
        controls
            .iter()
            .map(|c| Ok((self.wire(&c.qubit)?, c.positive)))
            .collect()
    }
    pub fn visit(
        &self,
        node: &Node,
        mut visitor: impl FnMut(&FlatGate) -> Result<()>,
    ) -> Result<()> {
        let mut emit = |gate: FlatGate| {
            crate::export::validate_gate(&gate, self.own)?;
            visitor(&gate)
        };
        match node {
            Node::Gate(g) => {
                if g.own_controls > g.controls.len() {
                    bail!("QASM: own controls exceed operands");
                }
                emit(FlatGate {
                    gate: g.gate,
                    controls: self.controls(&g.controls)?,
                    targets: g
                        .targets
                        .iter()
                        .map(|q| self.wire(q))
                        .collect::<Result<_>>()?,
                })
            }
            Node::Copy { src, dst, controls }
            | Node::Uncopy { src, dst, controls }
            | Node::Xor {
                src,
                tgt: dst,
                controls,
            } => self.bundled(src, dst, controls, false, &mut emit),
            Node::Exch { a, b, controls } => self.bundled(a, b, controls, true, &mut emit),
            Node::Load {
                region,
                addr,
                dst,
                controls,
            }
            | Node::Unload {
                region,
                addr,
                dst,
                controls,
            } => self.memory(region, addr, dst, controls, 0, &mut emit),
            Node::Store {
                region,
                addr,
                val,
                controls,
            } => self.memory(region, addr, val, controls, 1, &mut emit),
            Node::ExchM {
                region,
                addr,
                val,
                controls,
            } => self.memory(region, addr, val, controls, 2, &mut emit),
            Node::Call { .. } => bail!("QASM: local visitor cannot expand a call"),
        }
    }
    fn bundled(
        &self,
        a: &str,
        b: &str,
        controls: &[Ctrl],
        exchange: bool,
        emit: &mut impl FnMut(FlatGate) -> Result<()>,
    ) -> Result<()> {
        let (a, b) = (self.bundle(a)?, self.bundle(b)?);
        if a.len() != b.len() {
            bail!("QASM: bundle width mismatch");
        }
        let base = self.controls(controls)?;
        for (source, target) in a.zip(b) {
            let mut cs = base.clone();
            if !exchange {
                cs.insert(0, (source, true));
            }
            emit(FlatGate {
                gate: if exchange {
                    BaseGate::Swap
                } else {
                    BaseGate::X
                },
                controls: cs,
                targets: if exchange {
                    vec![source, target]
                } else {
                    vec![target]
                },
            })?;
        }
        Ok(())
    }
    fn memory(
        &self,
        region: &str,
        addr: &str,
        val: &str,
        controls: &[Ctrl],
        kind: u8,
        emit: &mut impl FnMut(FlatGate) -> Result<()>,
    ) -> Result<()> {
        let (a, w) = self
            .regions
            .get(region)
            .copied()
            .ok_or("QASM: missing region geometry")?;
        let count = 1usize
            .checked_shl(a)
            .ok_or("host limit: QASM region size")?;
        let (cells, address, value) = (self.bundle(region)?, self.bundle(addr)?, self.bundle(val)?);
        if address.len() != a as usize
            || value.len() != w as usize
            || cells.len()
                != count
                    .checked_mul(w as usize)
                    .ok_or("host limit: QASM region width")?
        {
            bail!("QASM: memory geometry mismatch");
        }
        let base = self.controls(controls)?;
        for cell in 0..count {
            let mut equal = address
                .clone()
                .enumerate()
                .map(|(bit, q)| (q, cell >> bit & 1 != 0))
                .collect::<Vec<_>>();
            equal.extend_from_slice(&base);
            for bit in 0..w as usize {
                let (memory, data) = (cells.start + cell * w as usize + bit, value.start + bit);
                let mut cs = equal.clone();
                let targets = match kind {
                    0 => {
                        cs.insert(0, (memory, true));
                        vec![data]
                    }
                    1 => {
                        cs.insert(0, (data, true));
                        vec![memory]
                    }
                    _ => vec![data, memory],
                };
                emit(FlatGate {
                    gate: if kind == 2 {
                        BaseGate::Swap
                    } else {
                        BaseGate::X
                    },
                    controls: cs,
                    targets,
                })?;
            }
        }
        Ok(())
    }
}
