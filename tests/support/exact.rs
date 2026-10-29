//! Independent, std-only exact amplitudes. Dyadic arbitrary integers,
//! Q(sqrt(2),i) coefficients, and formal exp(i*d) for dyadic radians d.
//! No pruning or floating-point zero tests. Only display uses binary64.
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct D {
    neg: bool,
    words: Vec<u32>,
    exp: i32,
}
impl D {
    pub fn int(n: i32) -> Self {
        Self {
            neg: n < 0,
            words: if n == 0 {
                vec![]
            } else {
                vec![n.unsigned_abs()]
            },
            exp: 0,
        }
        .norm()
    }
    fn norm(mut self) -> Self {
        while self.words.last() == Some(&0) {
            self.words.pop();
        }
        if self.words.is_empty() {
            self.neg = false;
            self.exp = 0;
            return self;
        }
        while self.words[0] == 0 {
            self.words.remove(0);
            self.exp += 32;
        }
        let n = self.words[0].trailing_zeros();
        if n > 0 {
            for i in 0..self.words.len() {
                self.words[i] = (self.words[i] >> n)
                    | self
                        .words
                        .get(i + 1)
                        .copied()
                        .unwrap_or(0)
                        .wrapping_shl(32 - n);
            }
            self.exp += n as i32;
        }
        while self.words.last() == Some(&0) {
            self.words.pop();
        }
        self
    }
    pub fn from_f64(x: f64) -> Self {
        assert!(x.is_finite());
        let bits = x.to_bits();
        let e = ((bits >> 52) & 2047) as i32;
        let mant = (bits & ((1u64 << 52) - 1)) | if e == 0 { 0 } else { 1u64 << 52 };
        Self {
            neg: bits >> 63 != 0,
            words: vec![mant as u32, (mant >> 32) as u32],
            exp: if e == 0 { -1074 } else { e - 1023 - 52 },
        }
        .norm()
    }
    fn zero(&self) -> bool {
        self.words.is_empty()
    }
    pub fn neg(&self) -> Self {
        let mut n = self.clone();
        if !n.zero() {
            n.neg = !n.neg;
        }
        n
    }
    pub fn half(&self) -> Self {
        let mut n = self.clone();
        if !n.zero() {
            n.exp -= 1;
        }
        n
    }
    fn aligned(&self, exp: i32) -> Vec<u32> {
        if self.zero() {
            return vec![];
        }
        let shift = (self.exp - exp) as usize;
        let mut out = vec![0; shift / 32];
        let mut carry = 0u64;
        for w in &self.words {
            let v = ((*w as u64) << (shift % 32)) | carry;
            out.push(v as u32);
            carry = v >> 32;
        }
        if carry > 0 {
            out.push(carry as u32);
        }
        out
    }
    pub fn add(&self, b: &Self) -> Self {
        let exp = self.exp.min(b.exp);
        let mut a = self.aligned(exp);
        let mut bwords = b.aligned(exp);
        let neg;
        if self.neg == b.neg {
            neg = self.neg;
            let len = a.len().max(bwords.len());
            a.resize(len, 0);
            let mut carry = 0u64;
            for (i, w) in a.iter_mut().enumerate() {
                let v = *w as u64 + bwords.get(i).copied().unwrap_or(0) as u64 + carry;
                *w = v as u32;
                carry = v >> 32;
            }
            if carry > 0 {
                a.push(carry as u32);
            }
        } else {
            let cmp = a
                .len()
                .cmp(&bwords.len())
                .then_with(|| a.iter().rev().cmp(bwords.iter().rev()));
            neg = if cmp.is_lt() {
                std::mem::swap(&mut a, &mut bwords);
                b.neg
            } else {
                self.neg
            };
            let mut borrow = 0i64;
            for (i, w) in a.iter_mut().enumerate() {
                let v = *w as i64 - bwords.get(i).copied().unwrap_or(0) as i64 - borrow;
                *w = v as u32;
                borrow = i64::from(v < 0);
            }
            assert_eq!(borrow, 0);
        }
        Self { neg, words: a, exp }.norm()
    }
    pub fn mul(&self, b: &Self) -> Self {
        let mut words = vec![0; self.words.len() + b.words.len()];
        for (i, a) in self.words.iter().enumerate() {
            let mut carry = 0u64;
            for (j, b) in b.words.iter().enumerate() {
                let v = (*a as u64) * (*b as u64) + words[i + j] as u64 + carry;
                words[i + j] = v as u32;
                carry = v >> 32;
            }
            words[i + b.words.len()] = carry as u32;
        }
        Self {
            neg: self.neg ^ b.neg,
            words,
            exp: self.exp + b.exp,
        }
        .norm()
    }
    fn value(&self) -> f64 {
        if self.zero() {
            return 0.0;
        }
        let top = self.words.len() - 1;
        let mut v = 0.0;
        for (offset, w) in self.words.iter().rev().take(3).enumerate() {
            v += f64::from(*w) * 2.0f64.powi(-32 * offset as i32);
        }
        let mut e = self.exp + 32 * top as i32;
        while e < -512 {
            v *= 2.0f64.powi(-512);
            e += 512;
        }
        while e > 512 {
            v *= 2.0f64.powi(512);
            e -= 512;
        }
        v *= 2.0f64.powi(e);
        if self.neg { -v } else { v }
    }
}
// Basis: 1, sqrt(2), i, i sqrt(2).
#[derive(Clone, Debug, PartialEq, Eq)]
struct C([D; 4]);
impl C {
    fn scalar(n: i32) -> Self {
        Self([D::int(n), D::int(0), D::int(0), D::int(0)])
    }
    fn zero(&self) -> bool {
        self.0.iter().all(D::zero)
    }
    fn add(&self, b: &Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i].add(&b.0[i])))
    }
    fn mul(&self, b: &Self) -> Self {
        let mut c = Self::scalar(0);
        for i in 0..4 {
            for j in 0..4 {
                let mut v = self.0[i].mul(&b.0[j]);
                if i & 1 != 0 && j & 1 != 0 {
                    v = v.mul(&D::int(2));
                }
                if i & 2 != 0 && j & 2 != 0 {
                    v = v.neg();
                }
                let k = i ^ j;
                c.0[k] = c.0[k].add(&v);
            }
        }
        c
    }
    fn value(&self) -> (f64, f64) {
        (
            self.0[0].value() + std::f64::consts::SQRT_2 * self.0[1].value(),
            self.0[2].value() + std::f64::consts::SQRT_2 * self.0[3].value(),
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Amp(BTreeMap<D, C>);
impl Amp {
    pub fn one() -> Self {
        Self::constant(1, 0, 0)
    }
    pub fn zero() -> Self {
        Self(BTreeMap::new())
    }
    // Coefficient n*2^-denom in one of C's four basis components.
    pub fn constant(n: i32, denom: u32, component: usize) -> Self {
        let mut c = C::scalar(0);
        let mut d = D::int(n);
        if !d.zero() {
            d.exp -= denom as i32;
        }
        c.0[component] = d;
        if c.zero() {
            Self::zero()
        } else {
            Self(BTreeMap::from([(D::int(0), c)]))
        }
    }
    pub fn phase(d: D) -> Self {
        Self(BTreeMap::from([(d, C::scalar(1))]))
    }
    pub fn is_zero(&self) -> bool {
        self.0.is_empty()
    }
    pub fn add(&self, b: &Self) -> Self {
        let mut n = self.0.clone();
        for (d, c) in &b.0 {
            let v = n.get(d).unwrap_or(&C::scalar(0)).add(c);
            if v.zero() {
                n.remove(d);
            } else {
                n.insert(d.clone(), v);
            }
        }
        Self(n)
    }
    pub fn mul(&self, b: &Self) -> Self {
        let mut n = Self::zero();
        for (d, c) in &self.0 {
            for (e, b) in &b.0 {
                let v = c.mul(b);
                if !v.zero() {
                    n = n.add(&Self(BTreeMap::from([(d.add(e), v)])));
                }
            }
        }
        n
    }
    pub fn value(&self) -> (f64, f64) {
        let (mut re, mut im) = (0.0, 0.0);
        for (d, c) in &self.0 {
            let angle = d.value();
            assert!(
                angle.is_finite(),
                "display angle exceeds binary64 host limit"
            );
            let (s, t) = angle.sin_cos();
            let (a, b) = c.value();
            re += a * t - b * s;
            im += a * s + b * t;
        }
        (re, im)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dyad_arithmetic() {
        for a in [-9., -0.5, 0., 0.25, 8., 1e-300] {
            for b in [-1., 0., 2., 0.5] {
                let x = D::from_f64(a);
                let y = D::from_f64(b);
                assert_eq!(x.add(&y).value(), a + b);
                assert_eq!(x.mul(&y).value(), a * b);
                assert!(x.add(&x.neg()).zero());
            }
        }
    }
    #[test]
    fn sqrt2_i_algebra() {
        let x = Amp::constant(1, 1, 1);
        assert_eq!(x.mul(&x), Amp::constant(1, 1, 0));
        let i = Amp::constant(1, 0, 2);
        assert_eq!(i.mul(&i), Amp::constant(-1, 0, 0));
    }
}
