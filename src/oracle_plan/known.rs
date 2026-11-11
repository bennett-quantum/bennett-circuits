//! Finite known-bit lattice. None is an unconstrained quantum input bit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownBits(pub Vec<Option<bool>>);
impl KnownBits {
    pub fn unknown(width: usize) -> Self {
        Self(vec![None; width])
    }
    pub fn integer(text: &str, width: usize) -> Self {
        let mut bits = vec![false; width];
        for digit in text.trim_start_matches('-').bytes() {
            assert!(digit.is_ascii_digit(), "checked integer literal");
            let mut carry = u16::from(digit - b'0');
            for bit in &mut bits {
                let next = u16::from(*bit) * 10 + carry;
                *bit = next & 1 != 0;
                carry = next >> 1;
            }
        }
        if text.starts_with('-') {
            let mut carry = true;
            for bit in &mut bits {
                let next = !*bit ^ carry;
                carry &= !*bit;
                *bit = next;
            }
        }
        Self(bits.into_iter().map(Some).collect())
    }
    pub fn as_u128(&self) -> Option<u128> {
        let mut value = 0;
        for (i, bit) in self.0.iter().enumerate() {
            match (*bit)? {
                true if i >= 128 => return None,
                true => value |= 1 << i,
                false => (),
            }
        }
        Some(value)
    }
    pub fn resize(&self, width: usize, signed: bool) -> Self {
        let fill = if signed {
            self.0.last().copied().unwrap_or(Some(false))
        } else {
            Some(false)
        };
        let mut bits = self.0.clone();
        bits.resize(width, fill);
        bits.truncate(width);
        Self(bits)
    }
    pub(crate) fn join(&mut self, other: &Self) -> bool {
        assert_eq!(self.0.len(), other.0.len());
        let mut changed = false;
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            if *a != *b && a.is_some() {
                *a = None;
                changed = true;
            }
        }
        changed
    }
    pub fn contains(&self, concrete: u128) -> bool {
        self.0
            .iter()
            .enumerate()
            .all(|(i, b)| b.is_none_or(|bit| bit == (i < 128 && concrete >> i & 1 != 0)))
    }
    pub(crate) fn bitwise(&self, other: &Self, op: &str) -> Self {
        assert_eq!(self.0.len(), other.0.len());
        Self(
            self.0
                .iter()
                .zip(&other.0)
                .map(|(a, b)| match (op, a, b) {
                    ("and", Some(false), _) | ("and", _, Some(false)) => Some(false),
                    ("or", Some(true), _) | ("or", _, Some(true)) => Some(true),
                    ("and", Some(a), Some(b)) => Some(*a && *b),
                    ("or", Some(a), Some(b)) => Some(*a || *b),
                    ("xor", Some(a), Some(b)) => Some(*a ^ *b),
                    _ => None,
                })
                .collect(),
        )
    }
    pub(crate) fn add(&self, other: &Self, subtract: bool) -> Self {
        let mut carry = Some(subtract);
        let bits = self
            .0
            .iter()
            .zip(&other.0)
            .map(|(a, b)| {
                let b = b.map(|v| v ^ subtract);
                let values = possibilities(*a, b, carry);
                let sum = consensus(values.iter().map(|(a, b, c)| *a ^ *b ^ *c));
                carry = consensus(
                    values
                        .iter()
                        .map(|(a, b, c)| (*a && *b) || ((*a ^ *b) && *c)),
                );
                sum
            })
            .collect();
        Self(bits)
    }
    pub(crate) fn shift(&self, amount: u128, op: &str) -> Self {
        let width = self.0.len();
        if amount >= width as u128 {
            return Self(vec![Some(false); width]);
        }
        let amount = amount as usize;
        let sign = if op == "ashr" {
            self.0[width - 1]
        } else {
            Some(false)
        };
        Self(
            (0..width)
                .map(|i| {
                    if op == "shl" {
                        i.checked_sub(amount).map_or(Some(false), |j| self.0[j])
                    } else {
                        self.0.get(i + amount).copied().unwrap_or(sign)
                    }
                })
                .collect(),
        )
    }
    pub(crate) fn multiply(&self, other: &Self) -> Self {
        let mut value = Self(vec![Some(false); self.0.len()]);
        for (shift, bit) in other.0.iter().enumerate() {
            let row = Self(
                self.0
                    .iter()
                    .map(|a| match (a, bit) {
                        (Some(false), _) | (_, Some(false)) => Some(false),
                        (Some(a), Some(b)) => Some(*a && *b),
                        _ => None,
                    })
                    .collect(),
            )
            .shift(shift as u128, "shl");
            value = value.add(&row, false);
        }
        value
    }
    pub(crate) fn equal(&self, other: &Self) -> Option<bool> {
        if self
            .0
            .iter()
            .zip(&other.0)
            .any(|(a, b)| a.is_some() && b.is_some() && a != b)
        {
            Some(false)
        } else if self.0.iter().chain(&other.0).all(Option::is_some) {
            Some(true)
        } else {
            None
        }
    }
    pub(crate) fn compare(&self, other: &Self, predicate: &str) -> Self {
        let eq = self.equal(other);
        let lt = self.known_less(other, predicate.starts_with('s'));
        let bit = match predicate {
            "eq" => eq,
            "ne" => eq.map(|v| !v),
            "slt" | "ult" => lt,
            "sle" | "ule" if eq == Some(true) || lt == Some(true) => Some(true),
            "sle" | "ule" if eq == Some(false) && lt == Some(false) => Some(false),
            _ => None,
        };
        Self(vec![bit])
    }
    fn known_less(&self, other: &Self, signed: bool) -> Option<bool> {
        for (i, (a, b)) in self.0.iter().zip(&other.0).enumerate().rev() {
            let flip = signed && i + 1 == self.0.len();
            let (a, b) = ((*a)? ^ flip, (*b)? ^ flip);
            if a != b {
                return Some(!a && b);
            }
        }
        Some(false)
    }
    pub(crate) fn select(condition: Option<bool>, a: &Self, b: &Self) -> Self {
        match condition {
            Some(true) => a.clone(),
            Some(false) => b.clone(),
            None => {
                let mut result = a.clone();
                result.join(b);
                result
            }
        }
    }
}
fn possibilities(a: Option<bool>, b: Option<bool>, c: Option<bool>) -> Vec<(bool, bool, bool)> {
    let mut values = vec![];
    for x in [false, true] {
        for y in [false, true] {
            for z in [false, true] {
                if a.is_none_or(|v| v == x) && b.is_none_or(|v| v == y) && c.is_none_or(|v| v == z)
                {
                    values.push((x, y, z));
                }
            }
        }
    }
    values
}
fn consensus(mut values: impl Iterator<Item = bool>) -> Option<bool> {
    let first = values.next()?;
    values.all(|v| v == first).then_some(first)
}
