use super::*;
impl Layers {
    pub(crate) fn descriptors(&self) -> (&[BTreeSet<RowKey>], &[BTreeSet<RowKey>]) {
        (&self.prefix, &self.cycle)
    }

    pub fn descriptor_count(&self) -> usize {
        self.prefix.len() + self.cycle.len()
    }
    pub fn at(&self, step: u64) -> &BTreeSet<RowKey> {
        if step < self.prefix.len() as u64 {
            return &self.prefix[step as usize];
        }
        let index = (step - self.prefix.len() as u64) % self.cycle.len() as u64;
        &self.cycle[index as usize]
    }
    pub fn expanded_row_contexts(&self, steps: u64, halt: usize) -> u128 {
        let count =
            |layer: &BTreeSet<RowKey>| layer.iter().filter(|k| k.pc != halt).count() as u128;
        let prefix = steps.min(self.prefix.len() as u64) as usize;
        let mut total: u128 = self.prefix[..prefix].iter().map(count).sum();
        let rest = steps - prefix as u64;
        total +=
            u128::from(rest / self.cycle.len() as u64) * self.cycle.iter().map(count).sum::<u128>();
        total += self.cycle[..(rest % self.cycle.len() as u64) as usize]
            .iter()
            .map(count)
            .sum::<u128>();
        total
    }
}
pub(super) fn build(nodes: &BTreeMap<RowKey, Node>, halt: usize) -> Layers {
    let mut seen = BTreeMap::new();
    let mut layers = vec![];
    let mut current = BTreeSet::from([RowKey {
        pc: 0,
        calls: vec![],
    }]);
    let limit = nodes.len().saturating_mul(4).max(64);
    loop {
        if let Some(start) = seen.get(&current).copied() {
            let cycle = layers.split_off(start);
            return Layers {
                prefix: layers,
                cycle,
                widened: false,
            };
        }
        if layers.len() == limit {
            let all = nodes
                .keys()
                .cloned()
                .chain([RowKey {
                    pc: halt,
                    calls: vec![],
                }])
                .collect();
            return Layers {
                prefix: layers,
                cycle: vec![all],
                widened: true,
            };
        }
        seen.insert(current.clone(), layers.len());
        layers.push(current.clone());
        current = next_layer(nodes, &current, halt);
    }
}
fn next_layer(
    nodes: &BTreeMap<RowKey, Node>,
    current: &BTreeSet<RowKey>,
    halt: usize,
) -> BTreeSet<RowKey> {
    let mut next = BTreeSet::new();
    for key in current {
        if key.pc == halt {
            next.insert(key.clone());
            continue;
        }
        for edge in nodes[key].successors.iter() {
            next.insert(edge.target.clone().unwrap_or(RowKey {
                pc: halt,
                calls: vec![],
            }));
        }
    }
    next
}

pub(super) fn general(halt: usize) -> Layers {
    let entry = BTreeSet::from([RowKey {
        pc: 0,
        calls: vec![],
    }]);
    let all = (0..=halt).map(|pc| RowKey { pc, calls: vec![] }).collect();
    Layers {
        prefix: vec![entry],
        cycle: vec![all],
        widened: true,
    }
}
