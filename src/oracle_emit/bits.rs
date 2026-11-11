//! Numeric reversible kernels. Every temporary is retained until global replay.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Bit {
    C(bool),
    Q(usize, bool),
}
pub(super) type Word = Vec<Bit>;
pub(super) fn not(bit: &Bit) -> Bit {
    match bit {
        Bit::C(v) => Bit::C(!v),
        Bit::Q(q, p) => Bit::Q(*q, !p),
    }
}
pub(super) fn constant(value: u128, width: usize) -> Word {
    (0..width)
        .map(|i| Bit::C(i < 128 && value >> i & 1 != 0))
        .collect()
}
pub(super) fn resize(word: &Word, width: usize, signed: bool) -> Word {
    let mut out = word.clone();
    out.resize(
        width,
        if signed {
            word.last().cloned().unwrap_or(Bit::C(false))
        } else {
            Bit::C(false)
        },
    );
    out.truncate(width);
    out
}
pub(super) fn literal(word: &Word) -> Option<Vec<bool>> {
    word.iter()
        .map(|b| if let Bit::C(v) = b { Some(*v) } else { None })
        .collect()
}
pub(super) struct Build {
    pub next: usize,
    pub gates: Vec<Flip>,
    pub kernels: Kernels,
}
impl Build {
    pub fn new(next: usize) -> Self {
        Self {
            next,
            gates: vec![],
            kernels: Kernels::default(),
        }
    }
    pub fn alloc(&mut self, width: usize) -> Word {
        let first = self.next;
        self.next = self
            .next
            .checked_add(width)
            .expect("recipe wire address overflow");
        (first..self.next).map(|q| Bit::Q(q, true)).collect()
    }
    pub fn flip(&mut self, target: &Bit, controls: &[Bit]) {
        let Bit::Q(target, true) = target else {
            panic!("recipe target must be a wire")
        };
        let mut normalized = BTreeMap::new();
        for c in controls {
            match c {
                Bit::C(false) => return,
                Bit::C(true) => (),
                Bit::Q(q, p) => {
                    if normalized.insert(*q, *p).is_some_and(|old| old != *p) {
                        return;
                    }
                }
            }
        }
        assert!(!normalized.contains_key(target), "recipe self-controlled X");
        self.gates.push(Flip {
            target: *target,
            controls: normalized.into_iter().collect(),
        });
    }
    pub fn xor(&mut self, a: &Bit, b: &Bit) -> Bit {
        if *a == Bit::C(false) {
            return b.clone();
        }
        if *b == Bit::C(false) {
            return a.clone();
        }
        if *a == Bit::C(true) {
            return not(b);
        }
        if *b == Bit::C(true) {
            return not(a);
        }
        if a == b {
            return Bit::C(false);
        }
        if *a == not(b) {
            return Bit::C(true);
        }
        let out = self.alloc(1)[0].clone();
        self.flip(&out, &[a.clone()]);
        self.flip(&out, &[b.clone()]);
        out
    }
    pub fn and(&mut self, a: &Bit, b: &Bit) -> Bit {
        if *a == Bit::C(false) || *b == Bit::C(false) || *a == not(b) {
            return Bit::C(false);
        }
        if *a == Bit::C(true) {
            return b.clone();
        }
        if *b == Bit::C(true) || a == b {
            return a.clone();
        }
        let out = self.alloc(1)[0].clone();
        self.flip(&out, &[a.clone(), b.clone()]);
        out
    }
    pub fn or(&mut self, a: &Bit, b: &Bit) -> Bit {
        let v = self.and(&not(a), &not(b));
        not(&v)
    }
    pub fn mux(&mut self, condition: &Bit, yes: &Bit, no: &Bit) -> Bit {
        if let Bit::C(v) = condition {
            return if *v { yes.clone() } else { no.clone() };
        }
        if yes == no {
            return yes.clone();
        }
        let difference = self.xor(yes, no);
        let selected = self.and(condition, &difference);
        self.xor(no, &selected)
    }
    pub fn mux_word(&mut self, c: &Bit, a: &Word, b: &Word) -> Word {
        a.iter().zip(b).map(|(a, b)| self.mux(c, a, b)).collect()
    }
    pub fn copy(&mut self, src: &Word, dst: &Word, controls: &[Bit]) {
        assert_eq!(src.len(), dst.len());
        for (s, d) in src.iter().zip(dst) {
            let mut cs = controls.to_vec();
            cs.push(s.clone());
            self.flip(d, &cs);
        }
    }
    pub fn exchange(&mut self, a: &Bit, b: &Bit, controls: &[Bit]) {
        if a == b {
            return;
        }
        for (source, target) in [(a, b), (b, a), (a, b)] {
            let mut cs = controls.to_vec();
            cs.push(source.clone());
            self.flip(target, &cs);
        }
    }
    pub fn eq(&mut self, a: &Word, b: &Word) -> Bit {
        self.kernels.equalities += 1;
        let mut equal = Bit::C(true);
        for (a, b) in a.iter().zip(b) {
            let different = self.xor(a, b);
            equal = self.and(&equal, &not(&different));
        }
        equal
    }
    pub fn equal_flag(&mut self, word: &Word, value: usize) -> Bit {
        let flag = self.alloc(1)[0].clone();
        let controls = word
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if value >> i & 1 != 0 {
                    b.clone()
                } else {
                    not(b)
                }
            })
            .collect::<Vec<_>>();
        self.flip(&flag, &controls);
        flag
    }
    pub fn ult(&mut self, a: &Word, b: &Word) -> Bit {
        self.kernels.ordered_comparisons += 1;
        let mut less = Bit::C(false);
        for (a, b) in a.iter().zip(b) {
            let difference = self.xor(a, b);
            let current = self.and(&not(a), b);
            let previous = self.and(&not(&difference), &less);
            less = self.or(&current, &previous);
        }
        less
    }
    pub fn add(&mut self, a: &Word, b: &Word, subtract: bool) -> Word {
        assert_eq!(a.len(), b.len());
        let mut carry = Bit::C(subtract);
        a.iter()
            .zip(b)
            .map(|(a, b)| {
                let b = if subtract { not(b) } else { b.clone() };
                let x = self.xor(a, &b);
                let sum = self.xor(&x, &carry);
                let ab = self.and(a, &b);
                let xc = self.and(&x, &carry);
                carry = self.or(&ab, &xc);
                sum
            })
            .collect()
    }
    pub fn multiply(&mut self, a: &Word, b: &Word) -> Word {
        if let Some(bits) = literal(b) {
            self.kernels.constant_multiplications += 1;
            let mut value = constant(0, a.len());
            for (shift, _) in bits.iter().enumerate().filter(|(_, bit)| **bit) {
                let row = fixed_shift(a, shift, "shl");
                value = self.add(&value, &row, false);
            }
            return value;
        }
        if literal(a).is_some() {
            return self.multiply(b, a);
        }
        self.kernels.generic_multiplications += 1;
        let mut value = constant(0, a.len());
        for (shift, bit) in b.iter().enumerate() {
            let row = (0..a.len())
                .map(|i| {
                    if i < shift {
                        Bit::C(false)
                    } else {
                        self.and(&a[i - shift], bit)
                    }
                })
                .collect();
            value = self.add(&value, &row, false);
        }
        value
    }
    pub fn shift(&mut self, a: &Word, amount: &Word, op: &str) -> Word {
        if let Some(bits) = literal(amount) {
            self.kernels.fixed_shifts += 1;
            let n = bits.iter().enumerate().try_fold(0usize, |n, (i, b)| {
                if !*b {
                    Some(n)
                } else {
                    1usize.checked_shl(i as u32).and_then(|v| n.checked_add(v))
                }
            });
            return fixed_shift(a, n.unwrap_or(usize::MAX), op);
        }
        self.kernels.variable_shifts += 1;
        let mut value = a.clone();
        for (i, bit) in amount.iter().enumerate() {
            let shift = 1usize.checked_shl(i as u32).unwrap_or(usize::MAX);
            let row = fixed_shift(&value, shift, op);
            value = self.mux_word(bit, &row, &value);
        }
        let large = not(&self.ult(amount, &constant(a.len() as u128, amount.len())));
        self.mux_word(&large, &constant(0, a.len()), &value)
    }
}
fn fixed_shift(a: &Word, shift: usize, op: &str) -> Word {
    if shift >= a.len() {
        return constant(0, a.len());
    }
    (0..a.len())
        .map(|i| {
            if op == "shl" {
                i.checked_sub(shift).map_or(Bit::C(false), |j| a[j].clone())
            } else {
                a.get(i + shift).cloned().unwrap_or_else(|| {
                    if op == "ashr" {
                        a[a.len() - 1].clone()
                    } else {
                        Bit::C(false)
                    }
                })
            }
        })
        .collect()
}
