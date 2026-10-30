//! ISA 0.9-draft §12.1–§12.3 CLAIM-REPLAY. Full positional tuples and
//! per-claim invocation bindings require explicit draft opt-in (ratified A3).
//! Reference relations only: no claim of scheduled/target equivalence.
use crate::{
    Error, Result, bail,
    flatten::{Flat, flatten_allocations},
    ir::{BaseGate, Program, WsKind},
    json::Value as V,
    object::{FnKind, Function, Object, Type},
    oracle::{Argument, Budget},
};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) fn obj<const N: usize>(p: [(&str, V); N]) -> V {
    V::Object(p.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn strv(s: impl ToString) -> V {
    V::Str(s.to_string())
}
fn arr(v: Vec<V>) -> V {
    V::Array(v)
}
fn num(n: usize) -> V {
    V::Int(n.to_string())
}
fn get<'a>(v: &'a V, k: &str) -> Result<&'a V> {
    v.get(k)
        .ok_or_else(|| Error(format!("certificate: missing {k} (§12.3)")))
}
fn array(v: &V) -> Result<&[V]> {
    v.as_array()
        .ok_or("certificate: array (§12)")
        .map_err(Error::from)
}
fn text(v: &V) -> Result<&str> {
    v.as_str()
        .ok_or("certificate: string (§12)")
        .map_err(Error::from)
}
fn budget_json(b: Budget) -> V {
    arr([b.steps, b.heap, b.stack, b.depth]
        .into_iter()
        .map(strv)
        .collect())
}
fn budget(v: &V) -> Result<Budget> {
    let a = array(v)?;
    if a.len() != 4 {
        bail!("certificate: four explicit budget components (§12.3)")
    }
    let n = a
        .iter()
        .map(|v| {
            canonical(v)?
                .parse::<u64>()
                .map_err(|_| Error("host limit: budget component (§5.4)".into()))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Budget {
        steps: n[0],
        heap: n[1],
        stack: n[2],
        depth: n[3],
    })
}
fn canonical(v: &V) -> Result<&str> {
    let s = text(v)?;
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || s.len() > 1 && s.starts_with('0') {
        bail!("certificate: canonical unsigned decimal string required (§12.1)")
    }
    Ok(s)
}
pub fn invocation(args: &[Argument], b: Budget) -> V {
    obj([
        (
            "args",
            arr(args
                .iter()
                .map(|a| match a {
                    Argument::Integer(n) => strv(n),
                    Argument::Float(f) => strv(format!("0x{:016x}", f.to_bits())),
                    Argument::Token(b) => budget_json(*b),
                })
                .collect()),
        ),
        ("budget", budget_json(b)),
    ])
}
fn resolve_invocation(_o: &Object, f: &Function, v: &V) -> Result<(Vec<Argument>, Budget)> {
    let m = v.as_object().ok_or("certificate: invocation record")?;
    if m.len() != 2 {
        bail!("certificate: invocation unknown fields (§12.3)")
    }
    let b = budget(get(v, "budget")?)?;
    let a = array(get(v, "args")?)?;
    let ps = &f.groups[if f.kind == FnKind::Oracle { 4 } else { 3 }];
    if a.len() != ps.len() {
        bail!("certificate: invocation argument arity (§12.3)")
    }
    let args = a
        .iter()
        .zip(ps)
        .map(|(a, p)| {
            let value = match p.ty {
                Type::Token => Argument::Token(budget(a)?),
                Type::F64 => {
                    let s = text(a)?;
                    if s.len() != 18
                        || !s.starts_with("0x")
                        || !s[2..]
                            .bytes()
                            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                    {
                        bail!("certificate: binary64 hex encoding (§12.3)")
                    }
                    Argument::Float(f64::from_bits(
                        u64::from_str_radix(&s[2..], 16)
                            .map_err(|_| Error("certificate: binary64".into()))?,
                    ))
                }
                _ => Argument::Integer(canonical(a)?.into()),
            };
            crate::lower::validate_arg(&p.ty, &value)
        })
        .collect::<Result<Vec<_>>>()?;

    Ok((args, b))
}
fn program(o: &Object, f: &Function, args: &[Argument], b: Budget) -> Result<Program> {
    if o.schedule.is_some() {
        bail!(
            "E_SCHEDULE_INFEASIBLE: certificate realised operation needs expansion library b-isa-spec-8r4"
        )
    }
    if f.kind == FnKind::Oracle {
        crate::oracle::specialise(o, &f.name, b, args)
    } else {
        crate::lower::lower_program_with_args(o, &f.name, args)
    }
}
fn uniform_adjoint(o: &Object, root: &Function) -> bool {
    fn inspect<'a>(is: &'a [crate::object::Instr], out: &mut Vec<&'a crate::object::Instr>) {
        for i in is {
            out.push(i);
            for b in &i.children {
                inspect(b, out)
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(f) = pending.pop() {
        if !seen.insert(&f.name) {
            continue;
        }
        let mut rows = vec![];
        for b in &f.blocks {
            inspect(&b.instrs, &mut rows)
        }
        for i in rows {
            if i.opcode == "qinplace" {
                return false;
            }
            for a in &i.operands {
                if let crate::object::Operand::Sym(n) = a
                    && let Some(callee) = o.function(n)
                {
                    pending.push(callee)
                }
            }
        }
    }
    true
}
fn nested_inverse_owner(o: &Object, root: &Function) -> Option<String> {
    fn rows<'a>(is: &'a [crate::object::Instr], out: &mut Vec<&'a crate::object::Instr>) {
        for i in is {
            out.push(i);
            for child in &i.children {
                rows(child, out)
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(f) = pending.pop() {
        if !seen.insert(&f.name) {
            continue;
        }
        let mut body = vec![];
        for b in &f.blocks {
            rows(&b.instrs, &mut body)
        }
        for i in body {
            if f.name != root.name && i.opcode == "qinplace" {
                return Some(f.name.clone());
            }
            for a in &i.operands {
                if let crate::object::Operand::Sym(n) = a
                    && let Some(cf) = o.function(n)
                {
                    pending.push(cf)
                }
            }
        }
    }
    None
}
fn direct_inverse_pairs(f: &Function, args: &[Argument]) -> Result<Vec<(String, String, Budget)>> {
    fn visit(
        is: &[crate::object::Instr],
        env: &BTreeMap<String, Argument>,
        out: &mut Vec<(String, String, Budget)>,
    ) -> Result<()> {
        for i in is {
            if i.opcode == "qinplace" {
                let [
                    crate::object::Operand::Sym(g),
                    crate::object::Operand::Sym(gi),
                    crate::object::Operand::Name(token),
                    _,
                ] = i.operands.as_slice()
                else {
                    bail!("certificate: qinplace schema (§4.5)")
                };
                let Some(Argument::Token(b)) = env.get(token) else {
                    bail!("certificate: unresolved qinplace token; bennett-circuits-4pf (§12.3)")
                };
                let value = (g.clone(), gi.clone(), *b);
                if !out.contains(&value) {
                    out.push(value)
                }
            }
            if i.opcode == "qloop"
                && i.operands.first() == Some(&crate::object::Operand::Int("0".into()))
            {
                continue;
            }
            for body in &i.children {
                visit(body, env, out)?;
            }
        }
        Ok(())
    }
    let mut pairs = vec![];
    let env = f.groups[if f.kind == FnKind::Oracle { 4 } else { 3 }]
        .iter()
        .zip(args)
        .map(|(p, a)| (p.name.clone(), a.clone()))
        .collect();
    for block in &f.blocks {
        visit(&block.instrs, &env, &mut pairs)?
    }
    Ok(pairs)
}
fn compatible(a: &Function, b: &Function) -> Result<bool> {
    if (a.kind == FnKind::Oracle) != (b.kind == FnKind::Oracle)
        || (!a.is_action() && a.kind != FnKind::Oracle)
        || (!b.is_action() && b.kind != FnKind::Oracle)
    {
        return Ok(false);
    }
    let normalize = |f: &Function| -> Result<V> {
        let mut v = crate::object::derived_signature(f)?;
        if f.is_action() {
            let V::Object(m) = &mut v else { unreachable!() };
            m.insert("kind".into(), strv("action"));
            m.remove("bounds");
        }
        Ok(v)
    };
    Ok(normalize(a)? == normalize(b)?)
}
struct Space {
    flat: Flat,
    bundles: Vec<(String, Vec<usize>)>,
    borrowed: Vec<(String, Vec<usize>)>,
    regions: Vec<(String, Vec<Vec<usize>>)>,
    order: Vec<usize>,
    readonly: Vec<usize>,
}
impl Space {
    fn new(p: &Program, f: &Function) -> Result<Self> {
        let c = p.top_circuit()?;
        let (flat, _, alloc) = flatten_allocations(p)?;
        let mut offset = 0;
        let mut wiremap = BTreeMap::new();
        for (n, w) in c.interface_bundles() {
            wiremap.insert(n.clone(), (offset..offset + w as usize).collect::<Vec<_>>());
            offset += w as usize;
        }
        let groups = if f.kind == FnKind::Oracle {
            vec![0, 1, 2]
        } else {
            vec![0, 1]
        };
        let bundles = groups
            .iter()
            .flat_map(|g| &f.groups[*g])
            .map(|p| (p.name.clone(), wiremap[&p.name].clone()))
            .collect::<Vec<_>>();
        let regions = c
            .mem
            .iter()
            .map(|m| {
                (
                    m.name.clone(),
                    wiremap[&m.name]
                        .chunks(m.cell_width as usize)
                        .map(<[usize]>::to_vec)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let names = c
            .workspace
            .iter()
            .filter(|w| w.kind == WsKind::Borrowed)
            .map(|w| w.name.clone())
            .collect::<Vec<_>>();
        let dirty = alloc
            .iter()
            .filter(|a| a.kind == WsKind::Borrowed)
            .collect::<Vec<_>>();
        if names.len() != dirty.len() {
            bail!("unimplemented: nested borrow domain identities; bennett-circuits-4pf (§12.2)")
        }
        let borrowed = names
            .into_iter()
            .zip(dirty.iter().map(|a| a.wires.clone()))
            .collect::<Vec<_>>();
        let components = bundles
            .iter()
            .chain(&borrowed)
            .map(|(_, ws)| ws.clone())
            .chain(regions.iter().flat_map(|(_, cells)| cells.clone()))
            .collect::<Vec<_>>();
        let order = components.into_iter().rev().flatten().collect();
        let readonly = c
            .in_bundles
            .iter()
            .flat_map(|b| wiremap[&b.name].clone())
            .chain(
                c.mem
                    .iter()
                    .filter(|m| m.role == crate::ir::Role::In)
                    .flat_map(|m| wiremap[&m.name].clone()),
            )
            .collect();
        Ok(Self {
            readonly,
            flat,
            bundles,
            borrowed,
            regions,
            order,
        })
    }
    fn domain(&self) -> V {
        let word = |(n, ws): &(String, Vec<usize>)| {
            obj([
                ("name", strv(n)),
                ("width", num(ws.len())),
                ("lo", strv("0")),
                ("hi", strv(crate::metadata::max_word(ws.len() as u32))),
            ])
        };
        obj([
            ("bundles", arr(self.bundles.iter().map(word).collect())),
            ("borrowed", arr(self.borrowed.iter().map(word).collect())),
            (
                "regions",
                arr(self
                    .regions
                    .iter()
                    .map(|(n, _)| obj([("name", strv(n)), ("cells", strv("all"))]))
                    .collect()),
            ),
        ])
    }
    fn tuple(&self, bits: &[bool]) -> V {
        let word =
            |ws: &Vec<usize>| strv(decimal(&ws.iter().map(|q| bits[*q]).collect::<Vec<_>>()));
        obj([
            (
                "bundles",
                arr(self.bundles.iter().map(|(_, ws)| word(ws)).collect()),
            ),
            (
                "borrowed",
                arr(self.borrowed.iter().map(|(_, ws)| word(ws)).collect()),
            ),
            (
                "regions",
                arr(self
                    .regions
                    .iter()
                    .map(|(_, cells)| arr(cells.iter().map(word).collect()))
                    .collect()),
            ),
        ])
    }
    fn execute(&self, input: &[bool]) -> Result<Vec<bool>> {
        let mut bits = input.to_vec();
        for g in &self.flat.gates {
            if !matches!(g.gate, BaseGate::X | BaseGate::Swap) {
                bail!(
                    "E_SCHEDULE_INFEASIBLE: non-permutation table assertion needs numerical/channel proof and expansion library b-isa-spec-8r4 (§12.1)"
                )
            }
            if g.controls.iter().all(|(q, p)| bits[*q] == *p) {
                match g.gate {
                    BaseGate::X => bits[g.targets[0]] ^= true,
                    BaseGate::Swap => bits.swap(g.targets[0], g.targets[1]),
                    _ => unreachable!(),
                }
            }
        }
        if self.readonly.iter().any(|q| bits[*q] != input[*q]) {
            bail!("certificate: input/readonly region not preserved (§12.1)")
        }
        let borrowed = self
            .borrowed
            .iter()
            .flat_map(|(_, ws)| ws)
            .copied()
            .collect::<BTreeSet<_>>();
        if self.flat.workspace.iter().any(|q| {
            if borrowed.contains(q) {
                bits[*q] != input[*q]
            } else {
                bits[*q]
            }
        }) {
            bail!("certificate: workspace/borrow not restored (§12.1)")
        }
        if !borrowed.is_empty() {
            let mut zero = input.to_vec();
            for q in &borrowed {
                zero[*q] = false;
            }
            // Independently prove full current-program P tensor I_B, including arbitrary
            // non-borrowed inputs; exact X/SWAP implies exact +1 basis phase.
            for g in &self.flat.gates {
                if g.controls.iter().all(|(q, p)| zero[*q] == *p) {
                    match g.gate {
                        BaseGate::X => zero[g.targets[0]] ^= true,
                        BaseGate::Swap => zero.swap(g.targets[0], g.targets[1]),
                        _ => unreachable!(),
                    }
                }
            }
            if self.flat.interface.iter().any(|q| zero[*q] != bits[*q]) {
                bail!("certificate: output depends on dirty borrow (§5.6)")
            }
        }
        Ok(bits)
    }
    fn each(&self, mut visit: impl FnMut(&[bool]) -> Result<()>) -> Result<()> {
        let mut bits = vec![false; self.flat.num_qubits];
        loop {
            visit(&bits)?;
            let mut carry = true;
            for q in &self.order {
                if carry {
                    bits[*q] = !bits[*q];
                    carry = !bits[*q];
                }
            }
            if carry {
                break;
            }
        }
        Ok(())
    }
    fn fault_rate(&self, f: &Function) -> Result<V> {
        let mut counts = BTreeMap::<String, String>::new();
        if f.kind == FnKind::Oracle {
            let fault = &self.bundles[f.groups[0].len() + 1].1;
            self.each(|input| {
                let output = self.execute(input)?;
                let code = decimal(
                    &fault
                        .iter()
                        .map(|q| input[*q] ^ output[*q])
                        .collect::<Vec<_>>(),
                );
                if code != "0" {
                    let value = counts.entry(code).or_insert_with(|| "0".into());
                    let mut digits = value.bytes().rev().collect::<Vec<_>>();
                    let mut carry = true;
                    for d in &mut digits {
                        if carry {
                            if *d == b'9' {
                                *d = b'0';
                            } else {
                                *d += 1;
                                carry = false;
                            }
                        }
                    }
                    if carry {
                        digits.push(b'1');
                    }
                    digits.reverse();
                    *value = String::from_utf8(digits).expect("decimal digits");
                }
                Ok(())
            })?;
        }
        let number = |text: String| {
            if text
                .parse::<u64>()
                .is_ok_and(|n| n <= 9_007_199_254_740_991)
            {
                V::Int(text)
            } else {
                strv(text)
            }
        };
        Ok(obj([
            ("domain", self.domain()),
            (
                "faults_by_code",
                V::Object(counts.into_iter().map(|(k, v)| (k, number(v))).collect()),
            ),
            ("undefined_excluded", num(0)),
            ("layout_dependent_excluded", num(0)),
        ]))
    }
    fn rows(&self) -> Result<Vec<V>> {
        let mut rows = vec![];
        self.each(|bits| {
            rows.push(obj([
                ("before", self.tuple(bits)),
                ("after", self.tuple(&self.execute(bits)?)),
            ]));
            Ok(())
        })?;
        Ok(rows)
    }
}
fn decimal(bits: &[bool]) -> String {
    let mut ds = vec![0u8];
    for b in bits.iter().rev() {
        let mut c = u8::from(*b);
        for d in &mut ds {
            let v = *d * 2 + c;
            *d = v % 10;
            c = v / 10;
        }
        if c > 0 {
            ds.push(c)
        }
    }
    ds.iter().rev().map(|d| (b'0' + d) as char).collect()
}
/// Caller-supplied publication bindings; no synthetic target or dialect id.
pub struct Context<'a> {
    pub target: &'a crate::target::Target,
    pub dialect_mapping_version: &'a str,
}
/// Generate reference-only §12 claims with complete full-domain tuple tables.
/// ISA 0.9-draft K13 encodings are never emitted without explicit opt-in.
pub fn generate(
    o: &Object,
    name: &str,
    args: &[Argument],
    b: Option<Budget>,
    draft: bool,
    context: &Context<'_>,
) -> Result<V> {
    generate_relation(o, name, args, b, draft, context, None)
}
/// Generate the compact §12.1 program relation. Replay still quantifies the
/// complete domain; no sample or finite-corpus verdict replaces the proof.
pub fn generate_program(
    o: &Object,
    name: &str,
    args: &[Argument],
    b: Option<Budget>,
    draft: bool,
    context: &Context<'_>,
    expected: &str,
) -> Result<V> {
    generate_relation(o, name, args, b, draft, context, Some(expected))
}
fn generate_relation(
    o: &Object,
    name: &str,
    args: &[Argument],
    b: Option<Budget>,
    draft: bool,
    context: &Context<'_>,
    expected_program: Option<&str>,
) -> Result<V> {
    if !draft {
        bail!("certificate: explicit --isa 0.9-draft required (§12 K13)")
    }
    if !o.imports.is_empty() {
        bail!("certificate: obligations pending; link before proving (§9.3; bennett-circuits-z9r)")
    }
    let f = o.function(name).ok_or("certificate: unknown subject")?;
    if let Some(owner) = nested_inverse_owner(o, f) {
        bail!(
            "unimplemented: nested qinplace owner @{owner} certificate/context composition; bennett-circuits-4pf (ISA question bennett-circuits-9su; §12.3)"
        )
    }

    let b = match (f.kind, b) {
        (FnKind::Oracle, None) => bail!("certificate: explicit oracle budget required (§12.3)"),
        (_, Some(b)) => b,
        _ => Budget {
            steps: 0,
            heap: 0,
            stack: 0,
            depth: 0,
        },
    };
    let inv = invocation(args, b);
    resolve_invocation(o, f, &inv)?;
    let p = program(o, f, args, b)?;
    let space = Space::new(&p, f)?;
    let identities = o.identity()?;
    let h = identities[name]
        .closure_hash
        .as_ref()
        .ok_or("certificate: unresolved closure")?;
    let subject = obj([
        ("closure_hash", strv(h)),
        ("variant", strv("forward")),
        ("inherited_controls", num(0)),
    ]);
    let table = obj([
        ("kind", strv("exhaustive")),
        ("domain", space.domain()),
        (
            "expected",
            if let Some(hash) = expected_program {
                obj([("program", strv(hash))])
            } else {
                obj([("table", arr(space.rows()?))])
            },
        ),
        ("subject", subject),
        ("invocation", inv.clone()),
    ]);
    let adjoint = obj([
        ("kind", strv("structural")),
        ("rule", strv("adjoint_table")),
        ("subclaims", arr(vec![])),
        ("witness", obj([])),
        (
            "invocation",
            if uniform_adjoint(o, f) {
                obj([("args", strv("all")), ("budget", strv("all"))])
            } else {
                inv.clone()
            },
        ),
    ]);
    if context.dialect_mapping_version.is_empty() {
        bail!("certificate: explicit dialect_mapping_version required (§12.3)")
    }
    let mut claims = vec![table, adjoint];
    for witness in crate::translation::witnesses(o, f)? {
        claims.push(obj([
            ("kind", strv("structural")),
            ("rule", strv("translation_lemma")),
            ("subclaims", arr(vec![])),
            ("witness", witness),
            (
                "invocation",
                obj([("args", strv("all")), ("budget", strv("all"))]),
            ),
        ]));
    }
    let mut cert = obj([
        (
            "versions",
            obj([
                ("isa", strv(&o.header.isa)),
                ("object", strv(&o.header.object)),
                ("launch", strv(&o.header.launch)),
                ("sched", strv(&o.header.sched)),
                ("cert", strv("0.9")),
                ("coverage", strv(&o.header.coverage)),
                ("target", strv(&o.header.target)),
            ]),
        ),
        (
            "bindings",
            obj([
                ("object_hash", strv(o.object_hash())),
                ("interface_hash", strv(&identities[name].interface_hash)),
                ("closure_hash", strv(h)),
                (
                    "reference_program_hash",
                    strv(&identities[name].interface_hash),
                ),
                (
                    "dialect_mapping_version",
                    strv(context.dialect_mapping_version),
                ),
                ("target_hash", strv(&context.target.hash)),
            ]),
        ),
        (
            "producer",
            obj([
                ("name", strv("bennett-circuits")),
                ("version", strv(crate::version())),
                (
                    "materialisation",
                    obj([
                        ("llvm_version", strv("none")),
                        ("pipeline", arr(vec![])),
                        ("normaliser_version", strv("history-1")),
                    ]),
                ),
            ]),
        ),
        (
            "regions",
            obj_dynamic(
                h,
                obj([
                    ("kind", strv(f.kind.symbol_kind())),
                    ("signature", crate::object::derived_signature(f)?),
                    (
                        "bounds",
                        arr(f.bounds.iter().map(|n| V::Int(n.to_string())).collect()),
                    ),
                    ("resources", obj([])),
                    ("gates_used", obj([])),
                    ("cleanup", obj([])),
                    ("error", obj([])),
                    ("definedness_assumptions", arr(vec![])),
                    ("claims", arr(claims)),
                    (
                        "verification",
                        obj([
                            ("oracle", num(0)),
                            (
                                "borrow_frames",
                                if space.borrowed.is_empty() {
                                    arr(vec![])
                                } else {
                                    arr(vec![num(0)])
                                },
                            ),
                            ("inverse_pairs", arr(vec![])),
                            ("unitary", arr(vec![])),
                            ("channel", arr(vec![])),
                            ("fault_rate", space.fault_rate(f)?),
                        ]),
                    ),
                ]),
            ),
        ),
        (
            "composition",
            obj([
                ("method", strv("sum_over_multiplicities")),
                ("multiplicities", obj([])),
                ("epsilon_total", obj([("diamond", strv("0"))])),
            ]),
        ),
    ]);
    let mut bound = o.clone();
    bound.draft_encodings = true;
    bound.certificate = Some(cert.clone());
    crate::checker::validate(&bound)?;
    verify(&bound)?;
    let placed = if f.kind == FnKind::Oracle {
        crate::placement::place_oracle_on_target(&bound, name, b, args, context.target)?
    } else {
        crate::placement::place_on_target(&bound, name, args, context.target)?
    };
    let frame = crate::frame::placed(&placed, &bound, true)?;
    let V::Object(top) = &mut cert else {
        unreachable!()
    };
    let V::Object(regions) = top.get_mut("regions").unwrap() else {
        unreachable!()
    };
    let V::Object(region) = regions.get_mut(h).unwrap() else {
        unreachable!()
    };
    region.insert("frame".into(), frame.to_json());
    region.insert(
        "resources".into(),
        V::Object(
            placed
                .resources
                .iter()
                .map(|(k, v)| Ok((k.clone(), strv(v.decimal()?))))
                .collect::<Result<_>>()?,
        ),
    );
    let mut gates = BTreeMap::<String, usize>::new();
    for e in &placed.events {
        if let Some(l) = &e.leaf {
            *gates.entry(l.op.clone()).or_default() += 1;
        }
    }
    region.insert(
        "gates_used".into(),
        V::Object(
            gates
                .into_iter()
                .map(|(k, n)| (k, obj([("count", num(n))])))
                .collect(),
        ),
    );
    for (g, gi, budget) in direct_inverse_pairs(f, args)? {
        add_inverse_pair(o, &mut cert, name, &g, &gi, budget)?;
    }
    bound.certificate = Some(cert.clone());
    crate::checker::validate(&bound)?;
    verify(&bound)?;
    Ok(cert)
}
fn obj_dynamic(k: &str, v: V) -> V {
    V::Object(BTreeMap::from([(k.into(), v)]))
}
/// Add a bound unary ISA 0.8 inverse-pair proof, checking both orientations
/// and fault freedom over the full input domain before emitting the entry.
pub fn add_inverse_pair(
    o: &Object,
    certificate: &mut V,
    owner: &str,
    g: &str,
    ginv: &str,
    b: Budget,
) -> Result<()> {
    let proof = crate::proof::inverse_pair(o, g, ginv, b)?;
    let ids = o.identity()?;
    let owner_hash = ids[owner]
        .closure_hash
        .as_ref()
        .ok_or("inverse pair: unresolved owner")?;
    let V::Object(cert) = certificate else {
        bail!("inverse pair: certificate record")
    };
    let V::Object(regions) = cert.get_mut("regions").ok_or("inverse pair: regions")? else {
        bail!("inverse pair: regions record")
    };
    let V::Object(region) = regions
        .get_mut(owner_hash)
        .ok_or("inverse pair: owner region")?
    else {
        bail!("inverse pair: region record")
    };
    let V::Array(claims) = region.get_mut("claims").ok_or("inverse pair: claims")? else {
        bail!("inverse pair: claims array")
    };
    let index = claims.len();
    let f = o.function(g).ok_or("inverse pair: source")?;
    let domain = obj([
        (
            "bundles",
            arr(vec![obj([
                ("name", strv(&f.groups[0][0].name)),
                ("width", num(proof.width as usize)),
                ("lo", strv("0")),
                ("hi", strv(crate::metadata::max_word(proof.width))),
            ])]),
        ),
        ("borrowed", arr(vec![])),
        ("regions", arr(vec![])),
    ]);
    claims.push(obj([
        ("kind", strv("exhaustive")),
        ("expected", strv("inverse_pair")),
        ("domain", domain),
    ]));
    let V::Object(verification) = region
        .get_mut("verification")
        .ok_or("inverse pair: verification")?
    else {
        bail!("inverse pair: verification record")
    };
    let entries = verification
        .entry("inverse_pairs".into())
        .or_insert_with(|| arr(vec![]));
    let V::Array(entries) = entries else {
        bail!("inverse pair: entries array")
    };
    entries.push(obj([
        ("g", strv(proof.g_closure_hash)),
        ("ginv", strv(proof.ginv_closure_hash)),
        ("claim", num(index)),
        ("budget", budget_json(b)),
    ]));
    let mut bound = o.clone();
    bound.draft_encodings = true;
    bound.certificate = Some(certificate.clone());
    crate::checker::validate(&bound)?;
    verify(&bound)
}
fn pair_entry(region: &V, index: usize) -> Result<&V> {
    let entries = array(get(get(region, "verification")?, "inverse_pairs")?)?;
    let found = entries
        .iter()
        .filter(|e| e.get("claim").and_then(V::as_u64) == Some(index as u64))
        .collect::<Vec<_>>();
    if found.len() != 1 {
        bail!("certificate: inverse claim requires exactly one bound entry (§12.3)")
    }
    Ok(found[0])
}
fn claim_dag(regions: &BTreeMap<String, V>) -> Result<()> {
    fn visit(
        key: (String, usize),
        regions: &BTreeMap<String, V>,
        state: &mut BTreeMap<(String, usize), u8>,
    ) -> Result<()> {
        match state.get(&key) {
            Some(1) => bail!("certificate: cyclic linked claim DAG (§12.3)"),
            Some(2) => return Ok(()),
            _ => {}
        }
        state.insert(key.clone(), 1);
        let region = regions
            .get(&key.0)
            .ok_or("certificate: library closure unresolved (§12.3)")?;
        let claim = array(get(region, "claims")?)?
            .get(key.1)
            .ok_or("certificate: library claim index outside array")?;
        if let Some(subclaims) = claim.get("subclaims") {
            for i in array(subclaims)? {
                visit(
                    (
                        key.0.clone(),
                        i.as_u64().ok_or("certificate: subclaim index")? as usize,
                    ),
                    regions,
                    state,
                )?;
            }
        }
        if claim.get("rule").and_then(V::as_str) == Some("inverse_pair_library") {
            let w = get(claim, "witness")?;
            let hash = text(get(w, "library_closure_hash")?)?.to_string();
            let i = get(w, "claim_index")?
                .as_u64()
                .ok_or("certificate: library index")? as usize;
            visit((hash.clone(), i), regions, state)?;
            let target = &regions[&hash];
            let target_claim = &array(get(target, "claims")?)?[i];
            if !(target_claim.get("expected").and_then(V::as_str) == Some("inverse_pair")
                || target_claim.get("rule").and_then(V::as_str) == Some("inverse_pair_library"))
            {
                bail!("certificate: library result is not an inverse-pair proof (§12.1)")
            }
            let a = pair_entry(region, key.1)?;
            let b = pair_entry(target, i)?;
            let same = (get(a, "g")? == get(b, "g")? && get(a, "ginv")? == get(b, "ginv")?)
                || (get(a, "g")? == get(b, "ginv")? && get(a, "ginv")? == get(b, "g")?);
            let ab = budget(get(a, "budget")?)?;
            let bb = budget(get(b, "budget")?)?;
            if !same
                || ab.steps < bb.steps
                || ab.heap < bb.heap
                || ab.stack < bb.stack
                || ab.depth < bb.depth
                || a.get("arg_tokens") != b.get("arg_tokens")
            {
                bail!("certificate: library inverse pair binding/coverage mismatch (§12.3)")
            }
        }
        state.insert(key, 2);
        Ok(())
    }
    let mut state = BTreeMap::new();
    for (h, r) in regions {
        for i in 0..array(get(r, "claims")?)?.len() {
            visit((h.clone(), i), regions, &mut state)?;
        }
    }
    Ok(())
}
/// Validate every draft claim, not just the indices referenced by verification.
pub(crate) fn verify(o: &Object) -> Result<()> {
    if !o.draft_encodings || !o.imports.is_empty() {
        return Ok(());
    }
    let Some(cert) = &o.certificate else {
        return Ok(());
    };
    let versions = get(cert, "versions")?;
    for (key, expected) in [
        ("isa", &o.header.isa),
        ("object", &o.header.object),
        ("launch", &o.header.launch),
        ("sched", &o.header.sched),
        ("coverage", &o.header.coverage),
        ("target", &o.header.target),
    ] {
        if let Some(actual) = versions.get(key)
            && actual.as_str() != Some(expected)
        {
            bail!("certificate: {key} version conflicts with the bound source header (§12.3)")
        }
        if versions.get("cert").and_then(V::as_str) == Some("0.9") && versions.get(key).is_none() {
            bail!("certificate: missing {key} version (§12.3)")
        }
    }
    let ids = o.identity()?;
    let bindings = get(cert, "bindings")?;
    if get(bindings, "object_hash")?.as_str() != Some(&o.object_hash()) {
        bail!("certificate: object binding mismatch (§12.3)")
    }
    let root = text(get(bindings, "closure_hash")?)?;
    let (root_name, root_hash) = ids
        .iter()
        .find(|(_, h)| h.closure_hash.as_deref() == Some(root))
        .ok_or("certificate: root closure unresolved")?;
    let root_fn = o
        .function(root_name)
        .filter(|f| f.is_action() || f.kind == FnKind::Oracle)
        .ok_or("certificate: root is not a quantum region (§12.3)")?;
    if root_fn.kind == FnKind::Oracle
        && get(bindings, "reference_program_hash")?.as_str() != Some(&root_hash.interface_hash)
    {
        bail!("certificate: oracle reference_program_hash binding differs (§9.2)")
    }
    if get(bindings, "interface_hash")?.as_str() != Some(&root_hash.interface_hash) {
        bail!("certificate: root interface binding mismatch (§12.3)")
    }
    let regions = get(cert, "regions")?
        .as_object()
        .ok_or("certificate: regions")?;
    claim_dag(regions)?;
    for (hash, region) in regions {
        let f = ids
            .iter()
            .find(|(_, h)| h.closure_hash.as_deref() == Some(hash))
            .and_then(|(n, _)| o.function(n))
            .ok_or("certificate: region closure unresolved (§12.3)")?;
        if let Some(frame) = region.get("frame") {
            crate::frame::Frame::parse(frame, true)?.validate_slots(f)?;
        }
        for (claim_index, claim) in array(get(region, "claims")?)?.iter().enumerate() {
            match text(get(claim, "kind")?)? {
                "exhaustive" => {
                    if claim.get("expected").and_then(V::as_str) == Some("inverse_pair") {
                        let entry = pair_entry(region, claim_index)?;
                        if let Some(tokens) = entry.get("arg_tokens")
                            && tokens.as_object().is_none_or(|m| !m.is_empty())
                        {
                            bail!(
                                "certificate: unary 0.8 inverse claim has no token arguments (§12.3)"
                            )
                        }

                        let lookup = |key: &str| -> Result<&str> {
                            let hash = text(get(entry, key)?)?;
                            ids.iter()
                                .find(|(_, id)| id.closure_hash.as_deref() == Some(hash))
                                .map(|(n, _)| n.as_str())
                                .ok_or_else(|| {
                                    Error("certificate: inverse closure unresolved".into())
                                })
                        };
                        let g = lookup("g")?;
                        let gi = lookup("ginv")?;
                        let b = budget(get(entry, "budget")?)?;
                        let proof = crate::proof::inverse_pair(o, g, gi, b)?;
                        let source = o.function(g).unwrap();
                        let expected_domain = obj([
                            (
                                "bundles",
                                arr(vec![obj([
                                    ("name", strv(&source.groups[0][0].name)),
                                    ("width", num(proof.width as usize)),
                                    ("lo", strv("0")),
                                    ("hi", strv(crate::metadata::max_word(proof.width))),
                                ])]),
                            ),
                            ("borrowed", arr(vec![])),
                            ("regions", arr(vec![])),
                        ]);
                        if get(claim, "domain")? != &expected_domain {
                            bail!("certificate: inverse pair requires full D (§12.1)")
                        }
                        continue;
                    }
                    if let Some(owner) = nested_inverse_owner(o, f) {
                        bail!(
                            "unimplemented: nested qinplace owner @{owner} proof/context replay; bennett-circuits-4pf (§12.3)"
                        )
                    }
                    let inv = get(claim, "invocation")?;
                    let (args, b) = resolve_invocation(o, f, inv)?;
                    let subject = get(claim, "subject")?;
                    if get(subject, "closure_hash")?.as_str() != Some(hash)
                        || get(subject, "variant")?.as_str() != Some("forward")
                        || get(subject, "inherited_controls")?.as_u64() != Some(0)
                    {
                        bail!(
                            "certificate: subject binding unsupported/mismatch; bennett-circuits-4pf (§12.1)"
                        )
                    }
                    if subject.get("schedule_hash").is_some() {
                        bail!(
                            "E_SCHEDULE_INFEASIBLE: scheduled claim requires expansion library b-isa-spec-8r4"
                        )
                    }
                    let space = Space::new(&program(o, f, &args, b)?, f)?;
                    if get(claim, "domain")? != &space.domain() {
                        bail!(
                            "certificate: domain must equal full D; subset evidence replay pending bennett-circuits-4pf (§12.2)"
                        )
                    }
                    let expected = get(claim, "expected")?;
                    if let Some(rows) = expected.get("table") {
                        let rows = array(rows)?;
                        let mut index = 0;
                        space.each(|bits|{let row=rows.get(index).ok_or("certificate: missing table row (§12.1)")?;let actual=obj([("before",space.tuple(bits)),("after",space.tuple(&space.execute(bits)?))]);if row!=&actual {bail!("certificate: full tuple mismatch/noncanonical row at {index} (§12.1)")}index+=1;Ok(())})?;
                        if index != rows.len() {
                            bail!("certificate: excess/duplicate table row (§12.1)")
                        }
                    } else if let Some(hash) = expected.get("program") {
                        let hash = text(hash)?;
                        let name = ids
                            .iter()
                            .find(|(_, h)| h.closure_hash.as_deref() == Some(hash))
                            .map(|(n, _)| n)
                            .ok_or(
                                "certificate: expected closure unresolved; bennett-circuits-4pf",
                            )?;
                        let other = o
                            .function(name)
                            .ok_or("certificate: expected closure is not callable")?;
                        if !compatible(f, other)? {
                            bail!("certificate: expected program signature incompatible (§12.1)")
                        }
                        let rhs = Space::new(&program(o, other, &args, b)?, other)?;
                        if space.order.len() != rhs.order.len() {
                            bail!("certificate: expected program domain incompatible (§12.1)")
                        }
                        space.each(|bits| {
                            let mut rbits = vec![false; rhs.flat.num_qubits];
                            for (a, b) in space.order.iter().zip(&rhs.order) {
                                rbits[*b] = bits[*a];
                            }
                            if space.tuple(&space.execute(bits)?)
                                != rhs.tuple(&rhs.execute(&rbits)?)
                            {
                                bail!("certificate: expected program tuple mismatch (§12.1)")
                            }
                            Ok(())
                        })?;
                    } else {
                        bail!(
                            "E_SCHEDULE_INFEASIBLE: expected relation requires expansion library b-isa-spec-8r4 (§12.1)"
                        )
                    }
                }
                "structural" => {
                    let invocation = get(claim, "invocation")?;
                    if invocation != &obj([("args", strv("all")), ("budget", strv("all"))]) {
                        resolve_invocation(o, f, invocation)?;
                    }
                    match text(get(claim, "rule")?)? {
                        "adjoint_table" => {
                            if let Some(owner) = nested_inverse_owner(o, f) {
                                bail!(
                                    "unimplemented: nested qinplace owner @{owner} proof/context replay; bennett-circuits-4pf (§12.3)"
                                )
                            }
                            if !uniform_adjoint(o, f) {
                                if invocation
                                    == &obj([("args", strv("all")), ("budget", strv("all"))])
                                {
                                    bail!(
                                        "unimplemented: parameter-uniform qinplace adjoint side conditions; bennett-circuits-4pf (§12.3)"
                                    )
                                }
                                let (args, b) = resolve_invocation(o, f, invocation)?;
                                program(o, f, &args, b)?;
                            }

                            if get(claim, "witness")? != &obj([]) {
                                bail!("certificate: adjoint witness must be empty (§12.1)")
                            }
                        }
                        "translation_lemma" => {
                            let witness = get(claim, "witness")?;
                            if !crate::translation::witnesses(o, f)?.contains(witness) {
                                bail!(
                                    "certificate: translation source points/wires/side conditions differ (§5.6; §12.1)"
                                )
                            }
                        }
                        "inverse_pair_library" => {}
                        _ => bail!(
                            "unimplemented: structural witness replay; bennett-circuits-4pf (§12.1)"
                        ),
                    }
                }
                "numerical" | "channel" => bail!(
                    "E_SCHEDULE_INFEASIBLE: numerical/channel claim requires expansion library b-isa-spec-8r4 (§12.1)"
                ),
                _ => bail!(
                    "unimplemented: sampled replay remains evidence; bennett-circuits-4pf (§12.1)"
                ),
            }
        }
        let verification = get(region, "verification")?;
        let claims = array(get(region, "claims")?)?;
        if let Some(index) = verification.get("oracle") {
            let claim = claims
                .get(index.as_u64().ok_or("certificate: oracle claim index")? as usize)
                .ok_or("certificate: oracle claim index outside array")?;
            if claim.get("kind").and_then(V::as_str) != Some("exhaustive")
                || claim.get("expected").and_then(V::as_str) == Some("inverse_pair")
            {
                bail!(
                    "certificate: verification.oracle does not name a full relation proof (§12.3)"
                )
            }
            let (args, b) = resolve_invocation(o, f, get(claim, "invocation")?)?;
            let expected_rate = Space::new(&program(o, f, &args, b)?, f)?.fault_rate(f)?;
            if get(verification, "fault_rate")? != &expected_rate {
                bail!("certificate: computed full-domain fault histogram differs (§12.3)")
            }
        }
        if let Some(entries) = verification.get("inverse_pairs") {
            for entry in array(entries)? {
                let index = get(entry, "claim")?
                    .as_u64()
                    .ok_or("certificate: inverse claim index")?
                    as usize;
                let claim = claims
                    .get(index)
                    .ok_or("certificate: inverse claim index outside array")?;
                if claim.get("expected").and_then(V::as_str) != Some("inverse_pair")
                    && claim.get("rule").and_then(V::as_str) != Some("inverse_pair_library")
                {
                    bail!("certificate: inverse entry does not name an inverse-pair proof (§12.3)")
                }
            }
        }
    }
    Ok(())
}
