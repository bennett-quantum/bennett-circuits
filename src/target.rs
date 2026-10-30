//! ISA 0.8 §8 target loading, exact cost expressions and gate realisations.
//! No reference implementation code or private expansion ordinals are used.
use crate::{Error, Result, bail, flatten::FlatGate, ir::BaseGate, json::Value as V};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rational {
    pub numerator: i128,
    pub denominator: i128,
}
impl Rational {
    pub const ZERO: Self = Self::integer(0);
    pub const ONE: Self = Self::integer(1);
    pub const fn integer(n: i128) -> Self {
        Self {
            numerator: n,
            denominator: 1,
        }
    }
    fn new(n: i128, d: i128) -> Result<Self> {
        if d == 0 {
            bail!("§8.1 division by zero");
        }
        if n == i128::MIN || d == i128::MIN {
            bail!("host limit: exact rational magnitude (§8.1)");
        }
        let (mut a, mut b) = (n.abs(), d.abs());
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let g = a.max(1);
        Ok(Self {
            numerator: n / g * d.signum(),
            denominator: d.abs() / g,
        })
    }
    pub fn parse(s: &str) -> Result<Self> {
        let (mant, exp) = s
            .split_once(['e', 'E'])
            .map_or((s, Ok(0)), |(a, b)| (a, b.parse::<i32>()));
        let exp = exp.map_err(|_| Error("§8.1 invalid rational exponent".into()))?;
        let fraction = mant.split_once('.').map_or(0, |(_, s)| s.len());
        let digits = mant.replace('.', "");
        if digits.is_empty()
            || !digits
                .trim_start_matches('-')
                .bytes()
                .all(|b| b.is_ascii_digit())
        {
            bail!("§8.1 invalid decimal {s:?}");
        }
        let n = digits
            .parse::<i128>()
            .map_err(|_| Error("host limit: exact decimal numerator (§8.1)".into()))?;
        let power = i32::try_from(fraction)
            .map_err(|_| Error("host limit: decimal scale".into()))?
            .checked_sub(exp)
            .ok_or("host limit: decimal scale")?;
        let scale = 10i128
            .checked_pow(power.unsigned_abs())
            .ok_or("host limit: exact decimal scale (§8.1)")?;
        if power >= 0 {
            Self::new(n, scale)
        } else {
            Self::new(
                n.checked_mul(scale)
                    .ok_or("host limit: decimal magnitude")?,
                1,
            )
        }
    }
    pub fn checked_add(self, b: Self) -> Result<Self> {
        Self::new(
            self.numerator
                .checked_mul(b.denominator)
                .and_then(|n| {
                    b.numerator
                        .checked_mul(self.denominator)
                        .and_then(|m| n.checked_add(m))
                })
                .ok_or("host limit: rational sum")?,
            self.denominator
                .checked_mul(b.denominator)
                .ok_or("host limit: rational denominator")?,
        )
    }
    pub fn negated(self) -> Result<Self> {
        Self::new(
            self.numerator
                .checked_neg()
                .ok_or("host limit: rational negation")?,
            self.denominator,
        )
    }
    pub fn checked_mul(self, b: Self) -> Result<Self> {
        Self::new(
            self.numerator
                .checked_mul(b.numerator)
                .ok_or("host limit: rational product")?,
            self.denominator
                .checked_mul(b.denominator)
                .ok_or("host limit: rational denominator")?,
        )
    }
    pub fn checked_div(self, b: Self) -> Result<Self> {
        self.checked_mul(Self::new(b.denominator, b.numerator)?)
    }
    pub fn compare(self, b: Self) -> Result<std::cmp::Ordering> {
        Ok(self
            .numerator
            .checked_mul(b.denominator)
            .ok_or("host limit: rational comparison")?
            .cmp(
                &b.numerator
                    .checked_mul(self.denominator)
                    .ok_or("host limit: rational comparison")?,
            ))
    }
    pub fn index(self) -> Result<usize> {
        if self.denominator != 1 {
            bail!("§8.1 non-integer index/count");
        }
        self.numerator
            .try_into()
            .map_err(|_| Error("§8.1 negative or oversized index/count".into()))
    }
    /// Exact internal annotation text; fractions are not mislabelled as §12 decimals.
    pub fn exact_text(self) -> String {
        self.decimal()
            .unwrap_or_else(|_| format!("{}/{}", self.numerator, self.denominator))
    }
    pub fn decimal(self) -> Result<String> {
        let mut d = self.denominator;
        for p in [2, 5] {
            while d % p == 0 {
                d /= p;
            }
        }
        if d != 1 {
            bail!("unimplemented: nonterminating rational resource decimal (milestone 3)");
        }
        let mut n = self.numerator.unsigned_abs();
        let den = self.denominator as u128;
        let mut s = format!("{}{}", if self.numerator < 0 { "-" } else { "" }, n / den);
        n %= den;
        if n != 0 {
            s.push('.');
            while n != 0 {
                n = n.checked_mul(10).ok_or("host limit: decimal rendering")?;
                s.push(char::from(b'0' + (n / den) as u8));
                n %= den;
            }
        }
        Ok(s)
    }
}
pub(crate) fn field<'a>(v: &'a V, k: &str) -> Result<&'a V> {
    v.get(k).ok_or_else(|| Error(format!("§8 missing {k}")))
}
pub(crate) fn array(v: &V) -> Result<&[V]> {
    v.as_array()
        .ok_or_else(|| Error("§8 expected array".into()))
}
pub(crate) fn string(v: &V) -> Result<&str> {
    v.as_str().ok_or_else(|| Error("§8 expected string".into()))
}
fn map(v: &V) -> Result<&BTreeMap<String, V>> {
    v.as_object()
        .ok_or_else(|| Error("§8 expected object".into()))
}
fn keys(v: &V, allowed: &[&str]) -> Result<()> {
    for k in map(v)?.keys() {
        if !allowed.contains(&k.as_str()) {
            if ["topology", "connectivity", "routing"].contains(&k.as_str()) {
                bail!(
                    "unimplemented: §8 topology/routing schema bennett-circuits-b4k (milestone 3)"
                );
            }
            bail!("§8 unknown field {k}");
        }
    }
    Ok(())
}
fn number(v: &V) -> Result<Rational> {
    match v {
        V::Int(s) => Rational::parse(s),
        _ => bail!("§8.1 expected numeric constant"),
    }
}
fn nonnegative(n: Rational) -> Result<Rational> {
    if n.numerator < 0 {
        bail!("§8.1 costs MUST be nonnegative and finite");
    }
    Ok(n)
}
fn check_expr(v: &V, vars: &[&str], tables: &BTreeMap<String, V>) -> Result<()> {
    if matches!(v, V::Int(_)) {
        number(v)?;
        return Ok(());
    }
    if let Some(var) = v.get("var") {
        keys(v, &["var"])?;
        let var = string(var)?;
        if !vars.contains(&var) {
            bail!("§8.1 unbound variable {var}");
        }
        return Ok(());
    }
    if let Some(t) = v.get("table") {
        keys(v, &["table", "index"])?;
        if !tables.contains_key(string(t)?) {
            bail!("§8.1 unknown table");
        }
        let i = string(field(v, "index")?)?;
        if !["k", "eps_decade"].contains(&i) || !vars.contains(&if i == "k" { "k" } else { "eps" })
        {
            bail!("§8.1 unbound table index");
        }
        return Ok(());
    }
    keys(v, &["op", "args"])?;
    let op = string(field(v, "op")?)?;
    let args = array(field(v, "args")?)?;
    let n = match op {
        "+" | "-" | "*" | "/" | "max" | "min" => 2,
        "ceil" => 1,
        "log2inv_eps" => {
            if !vars.contains(&"eps") {
                bail!("§8.1 unbound eps");
            }
            0
        }
        _ => bail!("§8.1 unknown expression op {op}"),
    };
    if args.len() != n {
        bail!("§8.1 expression arity");
    }
    for a in args {
        check_expr(a, vars, tables)?;
    }
    Ok(())
}
pub fn evaluate(
    v: &V,
    vars: &BTreeMap<String, Rational>,
    tables: &BTreeMap<String, V>,
) -> Result<Rational> {
    if matches!(v, V::Int(_)) {
        return number(v);
    }
    if let Some(x) = v.get("var") {
        let name = string(x)?;
        let n = *vars
            .get(name)
            .ok_or_else(|| Error(format!("§8.1 unbound {name}")))?;
        if name == "eps" && (n.numerator <= 0 || n.compare(Rational::ONE)?.is_gt()) {
            bail!("§8.1 eps outside (0,1]");
        }
        return Ok(n);
    }
    if let Some(t) = v.get("table") {
        let name = string(t)?;
        let index = string(field(v, "index")?)?;
        let i = if index == "k" {
            *vars.get("k").ok_or("§8.1 unbound k")?
        } else {
            bail!("unimplemented: §8.1 eps_decade definition bennett-circuits-v44 (milestone 3)");
        };
        return number(
            array(tables.get(name).ok_or("§8.1 unknown table")?)?
                .get(i.index()?)
                .ok_or("§8.1 table index outside array")?,
        );
    }
    let op = string(field(v, "op")?)?;
    let args = array(field(v, "args")?)?;
    let a = args
        .iter()
        .map(|x| evaluate(x, vars, tables))
        .collect::<Result<Vec<_>>>()?;
    Ok(match (op, a.as_slice()) {
        ("+", [x, y]) => x.checked_add(*y)?,
        ("-", [x, y]) => x.checked_add(y.negated()?)?,
        ("*", [x, y]) => x.checked_mul(*y)?,
        ("/", [x, y]) => x.checked_div(*y)?,
        ("min", [x, y]) => {
            if x.compare(*y)?.is_lt() {
                *x
            } else {
                *y
            }
        }
        ("max", [x, y]) => {
            if x.compare(*y)?.is_gt() {
                *x
            } else {
                *y
            }
        }
        ("ceil", [x]) => Rational::integer(
            x.numerator.div_euclid(x.denominator)
                + i128::from(x.numerator.rem_euclid(x.denominator) != 0),
        ),
        ("log2inv_eps", []) => {
            let mut e = *vars.get("eps").ok_or("§8.1 unbound eps")?;
            if e.numerator <= 0 || e.compare(Rational::ONE)?.is_gt() {
                bail!("§8.1 eps outside (0,1]");
            }
            let mut n = 0;
            while e.compare(Rational::ONE)?.is_lt() {
                e = e.checked_mul(Rational::integer(2))?;
                n += 1;
            }
            Rational::integer(n)
        }
        _ => bail!("§8.1 invalid expression/arity"),
    })
}
#[derive(Debug, Clone)]
pub struct Class {
    pub capacity: Option<usize>,
    pub dirty_rate: Rational,
}
#[derive(Debug, Clone)]
pub struct Target {
    pub document: V,
    pub hash: String,
    pub classes: BTreeMap<String, Class>,
    pub resources: BTreeMap<String, V>,
    pub tables: BTreeMap<String, V>,
}
impl Target {
    pub fn parse(text: &str) -> Result<Self> {
        let v = V::parse_target(text)?;
        if v.to_jcs() != text {
            bail!("§8.1 target must be RFC 8785 canonical JSON");
        }
        // JCS number syntax and shortest round-trip decimal spelling.
        fn canonical(v: &V) -> Result<()> {
            match v {
                V::Int(s) => {
                    let f = s
                        .parse::<f64>()
                        .map_err(|_| Error("§8.1 invalid JSON number".into()))?;
                    if !f.is_finite() {
                        bail!("§8.1 nonfinite JSON number");
                    }
                    let c = if f == 0.0 {
                        "0".into()
                    } else if f.abs() >= 1e21 || f.abs() < 1e-6 {
                        let raw = format!("{f:e}");
                        let (m, e) = raw.split_once('e').unwrap();
                        let e = e.parse::<i32>().unwrap();
                        format!("{m}e{}{e}", if e >= 0 { "+" } else { "" })
                    } else {
                        f.to_string()
                    };
                    if *s != c {
                        bail!("§8.1 target number {s} is not canonical JCS ({c})");
                    }
                }
                V::Array(a) => {
                    for x in a {
                        canonical(x)?
                    }
                }
                V::Object(m) => {
                    for x in m.values() {
                        canonical(x)?
                    }
                }
                _ => {}
            }
            Ok(())
        }
        canonical(&v)?;
        keys(
            &v,
            &[
                "ptr_widths_supported",
                "classes",
                "gates",
                "schemes",
                "qram",
                "capabilities",
                "objective",
                "tables",
                "imports",
                "resources",
            ],
        )?;
        let tables = map(field(&v, "tables")?)?.clone();
        for t in tables.values() {
            for n in array(t)? {
                nonnegative(number(n)?)?;
            }
        }
        let resources = map(field(&v, "resources")?)?.clone();
        for (n, r) in &resources {
            keys(r, &["kind", "unit", "rate"])?;
            let kind = string(field(r, "kind")?)?;
            if !["count", "time", "occupancy", "peak"].contains(&kind) {
                bail!("§8.2 invalid resource kind for {n}");
            }
            string(field(r, "unit")?)?;
            if let Some(r) = r.get("rate") {
                for n in map(r)?.values() {
                    nonnegative(number(n)?)?;
                }
            }
        }
        for (n, k) in [
            ("qubit_second", "occupancy"),
            ("time", "time"),
            ("peak_qubits", "peak"),
        ] {
            if string(field(
                resources.get(n).ok_or("§8.2 missing mandatory resource")?,
                "kind",
            )?)? != k
            {
                bail!("§8.2 mandatory resource kind {n}");
            }
        }
        let mut classes = BTreeMap::new();
        for (n, c) in map(field(&v, "classes")?)? {
            keys(c, &["qubit_second", "dirty_rate", "capacity"])?;
            if n == "any" {
                bail!("§8.3 any is not a concrete class");
            }
            nonnegative(number(field(c, "qubit_second")?)?)?;
            let dirty_rate = nonnegative(number(field(c, "dirty_rate")?)?)?;
            let cap = field(c, "capacity")?;
            let capacity = if cap.as_str() == Some("unbounded") {
                None
            } else {
                Some(number(cap)?.index()?)
            };
            classes.insert(
                n.clone(),
                Class {
                    capacity,
                    dirty_rate,
                },
            );
        }
        if classes.is_empty() {
            bail!("§8.3 empty classes");
        }
        let caps = field(&v, "capabilities")?;
        keys(
            caps,
            &[
                "measurement",
                "mbu",
                "max_parallel",
                "max_predicate_depth",
                "classical_step_time",
            ],
        )?;
        if field(caps, "mbu")? != &V::Bool(false) {
            bail!("§6.4/§8.3 mbu MUST be false");
        }
        for key in ["max_parallel", "max_predicate_depth"] {
            let x = field(caps, key)?;
            if x.as_str() != Some("unbounded") {
                number(x)?.index()?;
            }
        }
        nonnegative(number(field(caps, "classical_step_time")?)?)?;
        let m = field(caps, "measurement")?;
        keys(m, &["mid_circuit", "feedforward_latency"])?;
        if !matches!(field(m, "mid_circuit")?, V::Bool(_)) {
            bail!("§8.3 measurement bool");
        }
        nonnegative(number(field(m, "feedforward_latency")?)?)?;
        for width in array(field(&v, "ptr_widths_supported")?)? {
            number(width)?.index()?;
        }
        for g in array(field(&v, "gates")?)? {
            keys(
                g,
                &["op", "classes", "controls", "params", "epsilon", "cost"],
            )?;
            validate_signature(g)?;
            for c in array(field(g, "classes")?)? {
                let c = string(c)?;
                if c != "*" && !classes.contains_key(c) {
                    bail!("§8.3 unknown gate class {c}");
                }
            }
            let exact =
                matches!(field(g,"epsilon")?,V::Int(s) if Rational::parse(s)?==Rational::ZERO);
            let vars = if exact {
                &["k", "c"][..]
            } else {
                &["k", "c", "eps"][..]
            };
            check_expr(field(g, "epsilon")?, vars, &tables)?;
            for (r, e) in map(field(g, "cost")?)? {
                if !resources.contains_key(r) {
                    bail!("§8.2 unknown resource {r}");
                }
                check_expr(e, vars, &tables)?;
            }
            field(field(g, "cost")?, "time")?;
        }
        let mut names = BTreeSet::new();
        for s in array(field(&v, "schemes")?)? {
            keys(s, &["name", "realises", "ancillae", "template"])?;
            let n = string(field(s, "name")?)?;
            if !names.insert(n) || n == "native" {
                bail!("§8.3 duplicate/reserved scheme name");
            }
            keys(field(s, "realises")?, &["op", "controls", "params"])?;
            validate_signature(field(s, "realises")?)?;
            let a = field(s, "ancillae")?;
            keys(a, &["count", "state", "class"])?;
            check_expr(field(a, "count")?, &["k", "c"], &tables)?;
            if !["clean", "dirty"].contains(&string(field(a, "state")?)?) {
                bail!("§8.3 invalid ancilla state");
            }
            if !classes.contains_key(string(field(a, "class")?)?) {
                bail!("§8.3 unknown ancilla class");
            }
            for t in array(field(s, "template")?)? {
                keys(t, &["gate", "wires", "params", "repeat"])?;
                string(field(t, "gate")?)?;
                for w in array(field(t, "wires")?)? {
                    string(w)?;
                }
                if let Some(p) = t.get("params") {
                    for p in array(p)? {
                        check_expr(p, &["theta", "i"], &tables)?;
                    }
                }
                if let Some(r) = t.get("repeat") {
                    keys(r, &["var", "from", "to"])?;
                    if string(field(r, "var")?)? != "i" {
                        bail!("§8.3 repeat variable must be i");
                    }
                    for k in ["from", "to"] {
                        check_expr(field(r, k)?, &["k", "c", "theta"], &tables)?;
                    }
                }
            }
        }
        // These metadata describe costs; no imported quantities are executed here.
        for model in map(field(&v, "imports")?)?.values() {
            keys(model, &["version", "map"])?;
            string(field(model, "version")?)?;
            for entry in map(field(model, "map")?)?.values() {
                let a = array(entry)?;
                if a.len() != 2 || !resources.contains_key(string(&a[0])?) {
                    bail!("§8.3 invalid import mapping");
                }
                nonnegative(number(&a[1])?)?;
            }
        }
        let obj = field(&v, "objective")?;
        keys(obj, &["weighted", "lexicographic"])?;
        if map(obj)?.len() != 1 {
            bail!("§8.3 objective must choose one ordering");
        }
        if let Some(w) = obj.get("weighted") {
            for (n, w) in map(w)? {
                if !resources.contains_key(n) {
                    bail!("§8.3 unknown objective resource");
                }
                nonnegative(number(w)?)?;
            }
        } else {
            for n in array(field(obj, "lexicographic")?)? {
                if !resources.contains_key(string(n)?) {
                    bail!("§8.3 unknown objective resource");
                }
            }
        }
        if !field(&v, "qram")?.is_null() {
            let q = field(&v, "qram")?;
            keys(q, &["load", "store", "exch"])?;
            for k in ["load", "store", "exch"] {
                let e = field(q, k)?;
                keys(e, &["cost"])?;
                for (n, e) in map(field(e, "cost")?)? {
                    if !resources.contains_key(n) {
                        bail!("§8.3 unknown QRAM resource");
                    }
                    check_expr(e, &["a", "w", "c"], &tables)?;
                }
            }
        }
        Ok(Self {
            hash: crate::sha256::sha256_hex(text.as_bytes()),
            document: v,
            classes,
            resources,
            tables,
        })
    }
    pub fn native(
        &self,
        op: &str,
        gate: &FlatGate,
        k: usize,
        classes: &[String],
        eps: Rational,
    ) -> Result<Option<Leaf>> {
        let vars = BTreeMap::from([
            ("k".into(), Rational::integer(k as i128)),
            ("c".into(), Rational::integer(gate.controls.len() as i128)),
            ("eps".into(), eps),
        ]);
        for g in array(field(&self.document, "gates")?)? {
            if !covers(g, op, gate.controls.len(), gate.gate)? {
                continue;
            }
            let pattern = array(field(g, "classes")?)?;
            // Pattern lists targets; controls have the same class constraints via '*' or
            // repeated homogeneous target pattern. Heterogeneous patterns include all operands.
            let operands = gate
                .targets
                .iter()
                .chain(gate.controls.iter().map(|(q, _)| q))
                .collect::<Vec<_>>();
            let pats = pattern.iter().map(string).collect::<Result<Vec<_>>>()?;
            let matched = if pats.len() == gate.targets.len() {
                gate.targets
                    .iter()
                    .zip(&pats)
                    .all(|(q, p)| *p == "*" || classes[*q] == *p)
                    && gate
                        .controls
                        .iter()
                        .all(|(q, _)| pats.contains(&"*") || pats.iter().any(|p| classes[*q] == *p))
            } else if pats.len() == operands.len() {
                operands
                    .iter()
                    .zip(&pats)
                    .all(|(q, p)| *p == "*" || classes[**q] == *p)
            } else {
                false
            };
            if !matched {
                continue;
            }
            let error = nonnegative(evaluate(field(g, "epsilon")?, &vars, &self.tables)?)?;
            if error == Rational::ZERO {
                check_expr(field(g, "epsilon")?, &["k", "c"], &self.tables)?;
                for e in map(field(g, "cost")?)?.values() {
                    check_expr(e, &["k", "c"], &self.tables)?;
                }
            }
            if error.compare(eps)?.is_gt() {
                continue;
            }
            let mut costs = BTreeMap::new();
            for (r, e) in map(field(g, "cost")?)? {
                costs.insert(r.clone(), nonnegative(evaluate(e, &vars, &self.tables)?)?);
            }
            return Ok(Some(Leaf {
                gate: gate.clone(),
                op: op.into(),
                costs,
                error,
                scheme: None,
            }));
        }
        Ok(None)
    }
}
fn validate_signature(v: &V) -> Result<()> {
    let op = string(field(v, "op")?)?;
    if ![
        "x", "cx", "ccx", "mcx", "h", "s", "sdg", "t", "tdg", "cz", "swap", "rx", "ry", "rz",
    ]
    .contains(&op)
    {
        bail!("unimplemented: target opcode {op} (milestone 3)");
    }
    let c = field(v, "controls")?;
    keys(c, &["min", "max"])?;
    let lo = number(field(c, "min")?)?.index()?;
    let hi = field(c, "max")?;
    if hi.as_str() != Some("unbounded") && number(hi)?.index()? < lo {
        bail!("§8.3 reversed controls range");
    }
    let p = v.get("params").map(array).transpose()?.unwrap_or(&[]);
    if ["rx", "ry", "rz"].contains(&op) {
        if p.len() != 1 || p[0].as_str() != Some("theta") {
            bail!("§8.3 parametric gate requires theta");
        }
    } else if !p.is_empty() {
        bail!("§8.3 nonparametric gate params");
    }
    Ok(())
}
fn covers(v: &V, op: &str, c: usize, g: BaseGate) -> Result<bool> {
    let range = field(v, "controls")?;
    let min = number(field(range, "min")?)?.index()?;
    let max = field(range, "max")?;
    let param = matches!(g, BaseGate::Rx(_) | BaseGate::Ry(_) | BaseGate::Rz(_));
    Ok(string(field(v, "op")?)? == op
        && c >= min
        && (max.as_str() == Some("unbounded") || c <= number(max)?.index()?)
        && param
            == v.get("params")
                .is_some_and(|p| p.as_array().is_some_and(|a| !a.is_empty())))
}
#[derive(Debug, Clone, PartialEq)]
pub struct Leaf {
    pub gate: FlatGate,
    pub op: String,
    pub costs: BTreeMap<String, Rational>,
    pub error: Rational,
    pub scheme: Option<String>,
}
pub(crate) fn opcode(g: BaseGate, k: usize) -> String {
    if g == BaseGate::X {
        match k {
            0 => "x",
            1 => "cx",
            2 => "ccx",
            _ => "mcx",
        }
        .into()
    } else {
        crate::ir_codec::gate(g).split('(').next().unwrap().into()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Expansion {
    pub leaves: Vec<Leaf>,
    pub ancillae: Vec<(usize, String, bool)>,
}
impl Target {
    pub(crate) fn expand(
        &self,
        g: &FlatGate,
        k: usize,
        classes: &mut Vec<String>,
        eps: Rational,
    ) -> Result<Expansion> {
        let op = opcode(g.gate, k);
        if let Some(leaf) = self.native(&op, g, k, classes, eps)? {
            return Ok(Expansion {
                leaves: vec![leaf],
                ancillae: vec![],
            });
        }
        let scheme = array(field(&self.document, "schemes")?)?
            .iter()
            .find_map(
                |s| match covers(field(s, "realises").ok()?, &op, g.controls.len(), g.gate) {
                    Ok(true) => Some(Ok(s)),
                    Ok(false) => None,
                    Err(e) => Some(Err(e)),
                },
            )
            .transpose()?
            .ok_or_else(|| {
                Error(format!(
                    "§8.3 no native entry or scheme matches {op}, k={k}, c={} (epsilon={})",
                    g.controls.len(),
                    eps.decimal().unwrap_or_else(|_| "rational".into())
                ))
            })?;
        let name = string(field(scheme, "name")?)?.to_string();
        let mut vars = BTreeMap::from([
            ("k".into(), Rational::integer(k as i128)),
            ("c".into(), Rational::integer(g.controls.len() as i128)),
        ]);
        let theta = match g.gate {
            BaseGate::Rx(t) | BaseGate::Ry(t) | BaseGate::Rz(t) => {
                Some(Rational::parse(&t.to_string())?)
            }
            _ => None,
        };
        if let Some(t) = theta {
            vars.insert("theta".into(), t);
        }
        let a = field(scheme, "ancillae")?;
        let count = evaluate(field(a, "count")?, &vars, &self.tables)?.index()?;
        let class = string(field(a, "class")?)?.to_string();
        let dirty = string(field(a, "state")?)? == "dirty";
        let start = classes.len();
        let end = start
            .checked_add(count)
            .ok_or("host limit: scheme ancilla count")?;
        if count > 1_000_000 {
            bail!("host limit: materialised scheme ancillae > 1000000");
        }
        classes.resize(end, class.clone());
        let ancillae = (start..end).map(|q| (q, class.clone(), dirty)).collect();
        let mut templates = Vec::new();
        for t in array(field(scheme, "template")?)? {
            let range = if let Some(r) = t.get("repeat") {
                evaluate(field(r, "from")?, &vars, &self.tables)?.index()?
                    ..evaluate(field(r, "to")?, &vars, &self.tables)?.index()?
            } else {
                0..1
            };
            if range.start > range.end {
                bail!("§8.3 reversed repeat bounds");
            }
            if range.len() > 1_000_000 {
                bail!("host limit: materialised scheme repeat >1000000");
            }
            for i in range {
                let mut bound = vars.clone();
                if t.get("repeat").is_some() {
                    bound.insert("i".into(), Rational::integer(i as i128));
                }
                let mut wires = Vec::new();
                for w in array(field(t, "wires")?)? {
                    let w = string(w)?;
                    let q = if w == "t" {
                        *g.targets.first().ok_or("§8.3 missing t")?
                    } else if w == "t2" {
                        *g.targets.get(1).ok_or("§8.3 missing t2")?
                    } else {
                        let (kind, sub) =
                            w.split_once('[').ok_or("§8.3 malformed template wire")?;
                        let sub = sub
                            .strip_suffix(']')
                            .ok_or("§8.3 malformed wire subscript")?;
                        let expr = if ["i", "k", "c"].contains(&sub) {
                            V::Object(BTreeMap::from([("var".into(), V::Str(sub.into()))]))
                        } else {
                            V::parse_target(sub)?
                        };
                        check_expr(
                            &expr,
                            &if t.get("repeat").is_some() {
                                vec!["k", "c", "i", "theta"]
                            } else {
                                vec!["k", "c", "theta"]
                            },
                            &self.tables,
                        )?;
                        let j = evaluate(&expr, &bound, &self.tables)?.index()?;
                        match kind {
                            "c" => {
                                g.controls
                                    .get(j)
                                    .ok_or("§8.3 control subscript outside array")?
                                    .0
                            }
                            "a" => {
                                if j >= count {
                                    bail!("§8.3 ancilla subscript outside array");
                                }
                                start + j
                            }
                            _ => bail!("§8.3 invalid template wire"),
                        }
                    };
                    wires.push(q);
                }
                if wires.iter().copied().collect::<BTreeSet<_>>().len() != wires.len() {
                    bail!("§3.3/§8.3 template operands overlap");
                }
                let op = string(field(t, "gate")?)?.to_string();
                let params = t
                    .get("params")
                    .map(array)
                    .transpose()?
                    .unwrap_or(&[])
                    .iter()
                    .map(|e| {
                        let mut v = BTreeMap::new();
                        if let Some(t) = theta {
                            v.insert("theta".into(), t);
                        }
                        if t.get("repeat").is_some() {
                            v.insert("i".into(), Rational::integer(i as i128));
                        }
                        evaluate(e, &v, &self.tables)
                    })
                    .collect::<Result<Vec<_>>>()?;
                let (base, k) = template_gate(&op, &wires, &params)?;
                let targets = wires[k..].to_vec();
                let controls = wires[..k].iter().map(|q| (*q, true)).collect();
                templates.push((
                    op,
                    FlatGate {
                        gate: base,
                        targets,
                        controls,
                    },
                    k,
                ));
            }
        }
        // Negative controls are conjugated around the complete template, not individual leaves.
        let negatives = g
            .controls
            .iter()
            .filter(|(_, p)| !*p)
            .map(|(q, _)| *q)
            .collect::<Vec<_>>();
        for q in &negatives {
            templates.insert(
                0,
                (
                    "x".into(),
                    FlatGate {
                        gate: BaseGate::X,
                        targets: vec![*q],
                        controls: vec![],
                    },
                    0,
                ),
            );
        }
        for q in negatives.iter().rev() {
            templates.push((
                "x".into(),
                FlatGate {
                    gate: BaseGate::X,
                    targets: vec![*q],
                    controls: vec![],
                },
                0,
            ));
        }
        // Default request here is exact. For nonzero accuracy the scheduler must
        // establish the operation set/shares against the bound expansion library.
        let mut leaves = Vec::new();
        for (op, gate, k) in templates {
            let mut leaf=self.native(&op,&gate,k,classes,eps)?.ok_or_else(||Error(format!("§8.3 every template gate MUST match a gates entry; {op} in {name}; schemes do not nest")))?;
            leaf.scheme = Some(name.clone());
            leaves.push(leaf);
        }
        let error = leaves
            .iter()
            .try_fold(Rational::ZERO, |e, l| e.checked_add(l.error))?;
        if error.compare(eps)?.is_gt() {
            bail!("§8.3 scheme leaf errors exceed requested share");
        }
        Ok(Expansion { leaves, ancillae })
    }
}
fn template_gate(op: &str, wires: &[usize], params: &[Rational]) -> Result<(BaseGate, usize)> {
    let arity = if ["cz", "swap"].contains(&op) { 2 } else { 1 };
    let k = match op {
        "cx" => 1,
        "ccx" => 2,
        "mcx" => wires.len().checked_sub(1).ok_or("§8.3 mcx arity")?,
        _ => 0,
    };
    if wires.len() != k + arity {
        bail!("§8.3 template gate operand arity");
    }
    let param = if ["rx", "ry", "rz"].contains(&op) {
        if params.len() != 1 {
            bail!("§8.3 template angle arity");
        }
        let v = params[0].numerator as f64 / params[0].denominator as f64;
        if !v.is_finite() {
            bail!("§8.3 nonfinite angle");
        }
        Some(v)
    } else {
        if !params.is_empty() {
            bail!("§8.3 unexpected template angle");
        }
        None
    };
    let g = match op {
        "x" | "cx" | "ccx" | "mcx" => BaseGate::X,
        "h" => BaseGate::H,
        "s" => BaseGate::S,
        "sdg" => BaseGate::Sdg,
        "t" => BaseGate::T,
        "tdg" => BaseGate::Tdg,
        "cz" => BaseGate::Cz,
        "swap" => BaseGate::Swap,
        "rx" => BaseGate::Rx(param.unwrap()),
        "ry" => BaseGate::Ry(param.unwrap()),
        "rz" => BaseGate::Rz(param.unwrap()),
        _ => bail!("unimplemented: template gate {op} (milestone 3)"),
    };
    Ok((g, k))
}
