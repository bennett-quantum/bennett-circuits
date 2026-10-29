//! ISA 0.8 object loader (§16 canonical textual form), signature/schema
//! validation, §9.2 identity, and `check` (parse + validate + verify every
//! stored interface hash — the same verdict and hashes as
//! `bennett-isa-ref check` on the fixture/example corpus; the §6/§5.6/§5.5
//! static disciplines are milestone 2, see docs/PLAN.md).
//!
//! No code is shared with the reference interpreter; everything here is
//! derived from the published spec text and its §16.3 vectors.

use std::collections::{BTreeMap, BTreeSet};

use crate::json::Value;
use crate::sha256::sha256_hex;
use crate::{Error, Result, bail};

pub const ISA_VERSION: &str = "0.8";

const SECTION_ORDER: [&str; 9] = [
    "symbols",
    "imports",
    "functions",
    "data",
    "devices",
    "layout",
    "schedule",
    "certificate",
    "coverage",
];

type ClosureSet = (Vec<String>, Vec<(String, Value)>, Vec<(String, Value)>);

#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub isa: String,
    pub object: String,
    pub launch: String,
    pub sched: String,
    pub cert: String,
    pub coverage: String,
    pub target: String,
    pub ptr_width: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    pub index: String,
    pub name: String,
    pub kind: String,
    pub signature: Value,
    pub stored_interface_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub name: String,
    pub signature: Value,
    pub pinned_closure_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int(u32),
    Ptr,
    F64,
    Token,
    Wire(u32),
    Reg(u32),
    Region(u32, u32),
    QReg(u32),
}

impl Type {
    /// Width in wires for `wire`/`reg` types.
    pub fn wire_width(&self) -> Option<u32> {
        match self {
            Type::Wire(n) | Type::Reg(n) => Some(*n),
            _ => None,
        }
    }

    /// The §9.1 signature spelling of a classical type.
    pub fn signature_string(&self) -> Result<String> {
        Ok(match self {
            Type::Int(n) => format!("i{n}"),
            Type::Ptr => "ptr".into(),
            Type::F64 => "f64".into(),
            Type::Token => "token".into(),
            Type::QReg(n) => format!("qreg[{n}]"),
            t => bail!("type {t:?} has no classical signature spelling"),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub role: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegDecl {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Name(String),
    Indexed(String, u32),
    Sym(String),
    Label(String, Option<Vec<String>>),
    /// Integer literal, exact text (widths up to 2^257 appear).
    Int(String),
    /// Float literal, exact text.
    Float(String),
    Str(String),
    List(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Instr {
    pub lhs: Vec<String>,
    pub opcode: String,
    pub operands: Vec<Operand>,
    /// Nested `{ ... }` groups (one for `qloop`, up to two for `qif`).
    pub children: Vec<Vec<Instr>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub name: String,
    pub params: Vec<Param>,
    pub instrs: Vec<Instr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FnKind {
    Cc,
    R,
    Oracle,
    Action,
    PermAction,
    Lproc,
}

impl FnKind {
    pub fn symbol_kind(self) -> &'static str {
        match self {
            FnKind::Cc => "cc",
            FnKind::R => "r",
            FnKind::Oracle => "oracle",
            FnKind::Action => "action",
            FnKind::PermAction => "perm_action",
            FnKind::Lproc => "lproc",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub kind: FnKind,
    pub name: String,
    /// The five header parameter groups (§16.2).
    pub groups: [Vec<Param>; 5],
    pub regs: Vec<RegDecl>,
    pub bounds: Vec<u64>,
    pub blocks: Vec<Block>,
    /// Header line through last instruction line, LF-joined, no trailing LF.
    pub canonical_bytes: String,
}

impl Function {
    pub fn is_action(&self) -> bool {
        matches!(self.kind, FnKind::Action | FnKind::PermAction)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataEntry {
    pub name: String,
    pub hex: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Object {
    text: String,
    pub header: Header,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
    pub functions: Vec<Function>,
    pub data: Vec<DataEntry>,
    pub devices: Option<Value>,
    pub layout: Option<Value>,
    pub schedule: Option<Value>,
    pub certificate: Option<Value>,
    pub coverage: Option<Value>,
    /// (section name, first line index incl. preceding blank, end excl.)
    section_spans: Vec<(String, usize, usize)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolHashes {
    pub interface_hash: String,
    pub closure_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckReport {
    pub object_hash: String,
    pub status: String,
    pub symbols: BTreeMap<String, SymbolHashes>,
    pub unresolved_imports: Vec<String>,
}

impl Object {
    pub fn function(&self, name: &str) -> Option<&Function> {
        self.functions.iter().find(|f| f.name == name)
    }

    pub fn symbol(&self, name: &str) -> Option<&Symbol> {
        self.symbols.iter().find(|s| s.name == name)
    }

    pub fn data_entry(&self, name: &str) -> Option<&DataEntry> {
        self.data.iter().find(|d| d.name == name)
    }

    /// The §16.3-style examples and fixtures are LF text; anything else is
    /// rejected before structure parsing.
    pub fn parse(text: &str) -> Result<Object> {
        if text.contains('\r') {
            bail!("object: CR found; canonical form is LF-only (§16.1)");
        }
        let mut lines: Vec<&str> = text.split('\n').collect();
        if lines.last() == Some(&"") {
            lines.pop();
        }
        if lines.is_empty() {
            bail!("object: empty input");
        }
        for (i, line) in lines.iter().enumerate() {
            if line.ends_with(' ') || line.ends_with('\t') {
                bail!("object: trailing whitespace on line {} (§16.1)", i + 1);
            }
        }
        let header = parse_header(lines[0])?;

        // Split into sections: exactly one blank line before each
        // `section` line, none elsewhere (§16.1).
        let mut spans: Vec<(String, usize, usize)> = Vec::new();
        let mut bodies: Vec<(String, Vec<String>)> = Vec::new();
        let mut i = 1usize;
        let mut last_order: Option<usize> = None;
        while i < lines.len() {
            if !lines[i].is_empty() {
                bail!(
                    "object: expected blank line before section at line {}, got {:?}",
                    i + 1,
                    lines[i]
                );
            }
            let span_start = i;
            i += 1;
            let Some(name) = lines.get(i).and_then(|l| l.strip_prefix("section ")) else {
                bail!(
                    "object: expected 'section <name>' at line {} (§16.1)",
                    i + 1
                );
            };
            let name = name.to_string();
            let Some(order) = SECTION_ORDER.iter().position(|s| *s == name) else {
                bail!("object: unknown section {name:?} (§9.1)");
            };
            if last_order.is_some_and(|o| order <= o) {
                bail!("object: section {name:?} out of order or repeated (§9.1)");
            }
            last_order = Some(order);
            i += 1;
            let mut body = Vec::new();
            while i < lines.len() && !lines[i].is_empty() {
                body.push(lines[i].to_string());
                i += 1;
            }
            spans.push((name.clone(), span_start, i));
            bodies.push((name, body));
        }

        let mut symbols = Vec::new();
        let mut imports = Vec::new();
        let mut functions = Vec::new();
        let mut data = Vec::new();
        let mut devices = None;
        let mut layout = None;
        let mut schedule = None;
        let mut certificate = None;
        let mut coverage = None;
        for (name, body) in &bodies {
            match name.as_str() {
                "symbols" => symbols = parse_symbols(body)?,
                "imports" => imports = parse_imports(body)?,
                "functions" => functions = parse_functions(body)?,
                "data" => data = parse_data(body)?,
                "devices" => devices = Some(parse_json_section("devices", body)?),
                "layout" => layout = Some(parse_json_section("layout", body)?),
                "schedule" => schedule = Some(parse_json_section("schedule", body)?),
                "certificate" => certificate = Some(parse_json_section("certificate", body)?),
                "coverage" => coverage = Some(parse_json_section("coverage", body)?),
                _ => unreachable!("section order checked"),
            }
        }
        Ok(Object {
            text: text.to_string(),
            header,
            symbols,
            imports,
            functions,
            data,
            devices,
            layout,
            schedule,
            certificate,
            coverage,
            section_spans: spans,
        })
    }
}

fn parse_header(line: &str) -> Result<Header> {
    let toks: Vec<&str> = line.split(' ').collect();
    if toks.len() != 9 || toks[0] != "bennett-object" {
        bail!("object: malformed header {line:?} (§16.2)");
    }
    let field = |i: usize, key: &str| -> Result<String> {
        toks[i]
            .strip_prefix(&format!("{key}="))
            .map(str::to_string)
            .ok_or_else(|| Error(format!("object: header field {i} is not {key}= (§16.2)")))
    };
    let h = Header {
        isa: field(1, "isa")?,
        object: field(2, "object")?,
        launch: field(3, "launch")?,
        sched: field(4, "sched")?,
        cert: field(5, "cert")?,
        coverage: field(6, "coverage")?,
        target: field(7, "target")?,
        ptr_width: field(8, "ptr_width")?
            .parse()
            .map_err(|_| Error("object: bad ptr_width in header".into()))?,
    };
    for (k, v) in [
        ("isa", &h.isa),
        ("object", &h.object),
        ("launch", &h.launch),
        ("sched", &h.sched),
        ("cert", &h.cert),
        ("coverage", &h.coverage),
        ("target", &h.target),
    ] {
        if v != ISA_VERSION {
            bail!(
                "object: unsupported {k}={v}; this implementation is ISA {ISA_VERSION} (0.9 is drafted, not accepted; §9.1)"
            );
        }
    }
    Ok(h)
}

fn parse_json_section(name: &str, body: &[String]) -> Result<Value> {
    if body.len() != 1 {
        bail!(
            "object: section {name} must be exactly one JCS line (§9.1), got {} lines",
            body.len()
        );
    }
    let v = Value::parse(&body[0])?;
    if !matches!(v, Value::Object(_)) {
        bail!("object: section {name} is not a JSON object");
    }
    if v.to_jcs() != body[0] {
        bail!("object: section {name} line is not RFC 8785 canonical (§16.1)");
    }
    Ok(v)
}

fn expect_keys(label: &str, v: &Value, allowed: &[&str], required: &[&str]) -> Result<()> {
    let Some(map) = v.as_object() else {
        bail!("object: {label} is not an object");
    };
    for k in map.keys() {
        if !allowed.contains(&k.as_str()) {
            bail!("object: {label} has unknown field {k:?} (§9.1: unknown fields are errors)");
        }
    }
    for k in required {
        if !map.contains_key(*k) {
            bail!("object: {label} is missing field {k:?}");
        }
    }
    Ok(())
}

fn parse_symbols(body: &[String]) -> Result<Vec<Symbol>> {
    let v = parse_json_section("symbols", body)?;
    let map = v.as_object().expect("object checked");
    let mut out = Vec::new();
    let mut names = BTreeSet::new();
    for (index, entry) in map {
        if index.parse::<u64>().is_err() {
            bail!("object: symbol index {index:?} is not a decimal integer (§9.1)");
        }
        expect_keys(
            &format!("symbol {index}"),
            entry,
            &["interface_hash", "kind", "name", "signature"],
            &["interface_hash", "kind", "name", "signature"],
        )?;
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error(format!("object: symbol {index} name is not a string")))?
            .to_string();
        if !names.insert(name.clone()) {
            bail!("object: duplicate symbol name {name:?}");
        }
        let kind = entry
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| Error(format!("object: symbol {name} kind is not a string")))?
            .to_string();
        if ![
            "cc",
            "r",
            "oracle",
            "action",
            "perm_action",
            "lproc",
            "data",
        ]
        .contains(&kind.as_str())
        {
            bail!("object: symbol {name} has unknown kind {kind:?} (§9.1)");
        }
        let hash = entry
            .get("interface_hash")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error(format!(
                    "object: symbol {name} interface_hash is not a string"
                ))
            })?
            .to_string();
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            bail!("object: symbol {name} interface_hash is not 64 lowercase hex");
        }
        out.push(Symbol {
            index: index.clone(),
            name,
            kind,
            signature: entry.get("signature").expect("checked").clone(),
            stored_interface_hash: hash,
        });
    }
    out.sort_by_key(|s| s.index.parse::<u64>().expect("checked"));
    Ok(out)
}

fn parse_imports(body: &[String]) -> Result<Vec<Import>> {
    let v = parse_json_section("imports", body)?;
    let map = v.as_object().expect("object checked");
    let mut out = Vec::new();
    for (name, entry) in map {
        expect_keys(
            &format!("import {name}"),
            entry,
            &["pinned_closure_hash", "signature"],
            &["signature"],
        )?;
        out.push(Import {
            name: name.clone(),
            signature: entry.get("signature").expect("checked").clone(),
            pinned_closure_hash: entry
                .get("pinned_closure_hash")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    Ok(out)
}

fn parse_data(body: &[String]) -> Result<Vec<DataEntry>> {
    let mut out = Vec::new();
    for line in body {
        let Some((name, hex)) = line.split_once(' ') else {
            bail!("object: malformed data line {line:?} (§9.1 NAME SP HEX)");
        };
        if hex.len() % 2 != 0
            || !hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            bail!("object: data {name:?} payload is not lowercase even-length hex (§16.1)");
        }
        let bytes = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex checked"))
            .collect();
        out.push(DataEntry {
            name: name.to_string(),
            hex: hex.to_string(),
            bytes,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Function parsing (§16.2)
// ---------------------------------------------------------------------------

fn is_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.'
}

/// Tokenise one instruction/header line: top-level spaces separate tokens,
/// but parenthesised groups and quoted strings (either of which may contain
/// spaces) stay whole.
fn tokenise(line: &str) -> Result<Vec<String>> {
    let bytes = line.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b' ' {
            if i == 0 || bytes[i - 1] == b' ' {
                bail!("object: tokens require exactly one space (§16.1)");
            }
            i += 1;
            continue;
        }
        let start = i;
        let mut depth = 0i32;
        while i < bytes.len() {
            match bytes[i] {
                b' ' if depth == 0 => break,
                b'"' => {
                    i += 1;
                    loop {
                        match bytes.get(i) {
                            None => bail!("object: unterminated string in {line:?}"),
                            Some(b'\\') => i += 2,
                            Some(b'"') => {
                                i += 1;
                                break;
                            }
                            Some(_) => i += 1,
                        }
                    }
                }
                b'(' => {
                    depth += 1;
                    i += 1;
                }
                b')' => {
                    depth -= 1;
                    if depth < 0 {
                        bail!("object: unbalanced ')' in {line:?}");
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        }
        if depth != 0 {
            bail!("object: unbalanced '(' in {line:?}");
        }
        toks.push(line[start..i].to_string());
    }
    Ok(toks)
}

/// Split on `sep` at bracket depth zero (`region[4,8]` keeps its comma).
fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            _ => {}
        }
        if c == sep && depth == 0 {
            parts.push(cur.trim().to_string());
            cur = String::new();
        } else {
            cur.push(c);
        }
    }
    if !cur.trim().is_empty() || !parts.is_empty() {
        parts.push(cur.trim().to_string());
    }
    parts
}

fn parse_type(text: &str) -> Result<Type> {
    let t = text.trim();
    let bracket = |prefix: &str| -> Option<&str> {
        t.strip_prefix(prefix)?.strip_prefix('[')?.strip_suffix(']')
    };
    Ok(match t {
        "ptr" => Type::Ptr,
        "f64" => Type::F64,
        "token" => Type::Token,
        "wire" => Type::Wire(1),
        _ if bracket("wire").is_some() => Type::Wire(parse_u32(bracket("wire").unwrap(), t)?),
        _ if bracket("reg").is_some() => Type::Reg(parse_u32(bracket("reg").unwrap(), t)?),
        _ if bracket("qreg").is_some() => Type::QReg(parse_u32(bracket("qreg").unwrap(), t)?),
        _ if bracket("region").is_some() => {
            let inner = bracket("region").unwrap();
            let (a, w) = inner
                .split_once(',')
                .ok_or_else(|| Error(format!("object: bad region type {t:?}")))?;
            Type::Region(parse_u32(a, t)?, parse_u32(w, t)?)
        }
        _ if t.starts_with('i') && t[1..].chars().all(|c| c.is_ascii_digit()) && t.len() > 1 => {
            Type::Int(parse_u32(&t[1..], t)?)
        }
        _ => bail!("object: unknown type {t:?} (§16.2)"),
    })
}

fn parse_u32(text: &str, ctx: &str) -> Result<u32> {
    text.trim()
        .parse()
        .map_err(|_| Error(format!("object: bad integer in {ctx:?}")))
}

fn parse_param(text: &str) -> Result<Param> {
    let text = text.trim();
    let Some((name, rest)) = text.split_once(':') else {
        bail!("object: malformed parameter {text:?}");
    };
    let mut parts = rest.splitn(2, ' ');
    let ty = parse_type(parts.next().unwrap())?;
    let role = parts.next().map(|r| r.trim().to_string());
    if let Some(r) = &role
        && r != "in"
        && r != "mut"
    {
        bail!("object: parameter {text:?} has unknown role {r:?}");
    }
    Ok(Param {
        name: name.trim().to_string(),
        ty,
        role,
    })
}

fn parse_plist(token: &str) -> Result<Vec<Param>> {
    let inner = token
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| Error(format!("object: expected parameter list, got {token:?}")))?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    split_top(inner, ',')
        .iter()
        .map(|p| parse_param(p))
        .collect()
}

fn parse_operand(token: &str) -> Result<Operand> {
    if let Some(rest) = token.strip_prefix('"') {
        let Some(inner) = rest.strip_suffix('"') else {
            bail!("object: malformed string operand {token:?}");
        };
        let mut s = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('n') => s.push('\n'),
                    other => bail!("object: bad string escape {other:?} (§16.1)"),
                }
            } else {
                s.push(c);
            }
        }
        return Ok(Operand::Str(s));
    }
    if token.starts_with('(') {
        let inner = token
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .ok_or_else(|| Error(format!("object: malformed list operand {token:?}")))?;
        if inner.trim().is_empty() {
            return Ok(Operand::List(Vec::new()));
        }
        return Ok(Operand::List(split_top(inner, ',')));
    }
    if let Some(name) = token.strip_prefix('@') {
        return Ok(Operand::Sym(name.to_string()));
    }
    if let Some(rest) = token.strip_prefix('^') {
        return match rest.split_once('(') {
            None => Ok(Operand::Label(rest.to_string(), None)),
            Some((name, args)) => {
                let args = args
                    .strip_suffix(')')
                    .ok_or_else(|| Error(format!("object: malformed label operand {token:?}")))?;
                let list = if args.trim().is_empty() {
                    Vec::new()
                } else {
                    split_top(args, ',')
                };
                Ok(Operand::Label(name.to_string(), Some(list)))
            }
        };
    }
    if let Some(open) = token.find('[')
        && token.ends_with(']')
    {
        let idx: u32 = token[open + 1..token.len() - 1]
            .parse()
            .map_err(|_| Error(format!("object: bad indexed operand {token:?}")))?;
        return Ok(Operand::Indexed(token[..open].to_string(), idx));
    }
    let bytes = token.as_bytes();
    if is_int_text(token) {
        return Ok(Operand::Int(token.to_string()));
    }
    if (bytes[0].is_ascii_digit() || bytes[0] == b'-') && token.parse::<f64>().is_ok() {
        return Ok(Operand::Float(token.to_string()));
    }
    Ok(Operand::Name(token.to_string()))
}

fn is_int_text(token: &str) -> bool {
    let t = token.strip_prefix('-').unwrap_or(token);
    !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) && (t.len() == 1 || !t.starts_with('0'))
}

fn function_kind(line: &str) -> Option<(FnKind, usize)> {
    // (kind, number of leading tokens before @name)
    for (prefix, kind, ntok) in [
        ("fn cc @", FnKind::Cc, 2usize),
        ("fn r @", FnKind::R, 2),
        ("oracle @", FnKind::Oracle, 1),
        ("perm_action @", FnKind::PermAction, 1),
        ("action @", FnKind::Action, 1),
        ("lproc @", FnKind::Lproc, 1),
    ] {
        if line.starts_with(prefix) {
            return Some((kind, ntok));
        }
    }
    None
}

fn parse_functions(body: &[String]) -> Result<Vec<Function>> {
    let mut functions = Vec::new();
    let mut i = 0usize;
    while i < body.len() {
        let start = i;
        let Some((kind, ntok)) = function_kind(&body[i]) else {
            bail!(
                "object: expected function header at functions line {}: {:?}",
                i + 1,
                body[i]
            );
        };
        let toks = tokenise(&body[i])?;
        if toks.len() != ntok + 1 + 5 {
            bail!(
                "object: function header {:?} does not have five parameter groups (§16.2)",
                body[i]
            );
        }
        let name = toks[ntok]
            .strip_prefix('@')
            .ok_or_else(|| Error(format!("object: bad function name in {:?}", body[i])))?
            .to_string();
        let mut groups: [Vec<Param>; 5] = Default::default();
        for (g, tok) in toks[ntok + 1..].iter().enumerate() {
            groups[g] = parse_plist(tok)?;
        }
        i += 1;

        let mut regs = Vec::new();
        let mut bounds = Vec::new();
        if i < body.len() && body[i].starts_with("regs ") {
            (regs, bounds) = parse_regs(&body[i])?;
            i += 1;
        }

        let mut blocks = Vec::new();
        while i < body.len() && body[i].starts_with('^') {
            let (block, next) = parse_block(body, i)?;
            blocks.push(block);
            i = next;
        }
        if blocks.is_empty() {
            bail!("object: function @{name} has no blocks (§16.2 function := fhead regs? block+)");
        }
        let canonical_bytes = body[start..i].join("\n");
        functions.push(Function {
            kind,
            name,
            groups,
            regs,
            bounds,
            blocks,
            canonical_bytes,
        });
    }
    Ok(functions)
}

fn parse_regs(line: &str) -> Result<(Vec<RegDecl>, Vec<u64>)> {
    let rest = line.strip_prefix("regs ").expect("checked");
    let Some(open) = rest.find('{') else {
        bail!("object: malformed regs line {line:?}");
    };
    let Some(close) = rest[open..].find('}').map(|c| open + c) else {
        bail!("object: malformed regs line {line:?}");
    };
    let inner = &rest[open + 1..close];
    let mut regs = Vec::new();
    if !inner.trim().is_empty() {
        for entry in split_top(inner, ',') {
            let Some((name, ty)) = entry.split_once(':') else {
                bail!("object: malformed regs entry {entry:?} in {line:?}");
            };
            regs.push(RegDecl {
                name: name.trim().to_string(),
                ty: parse_type(ty)?,
            });
        }
    }
    let mut bounds = Vec::new();
    let tail = rest[close + 1..].trim();
    if !tail.is_empty() {
        let Some(b) = tail
            .strip_prefix("bounds {")
            .and_then(|s| s.strip_suffix('}'))
        else {
            bail!("object: malformed bounds in regs line {line:?}");
        };
        if !b.trim().is_empty() {
            for n in b.split(',') {
                bounds.push(
                    n.trim()
                        .parse()
                        .map_err(|_| Error(format!("object: bad bound in {line:?}")))?,
                );
            }
        }
    }
    Ok((regs, bounds))
}

fn parse_block(body: &[String], start: usize) -> Result<(Block, usize)> {
    let header = &body[start];
    let Some(h) = header.strip_prefix('^').and_then(|s| s.strip_suffix(':')) else {
        bail!("object: malformed block header {header:?}");
    };
    let Some((name, params)) = h.split_once('(') else {
        bail!("object: malformed block header {header:?}");
    };
    let params = parse_plist(&format!("({params}"))?;
    let (instrs, next) = parse_instrs(body, start + 1, 0)?;
    if instrs.is_empty() {
        bail!("object: block ^{name} has no instructions (§16.2)");
    }
    Ok((
        Block {
            name: name.to_string(),
            params,
            instrs,
        },
        next,
    ))
}

/// Parse instructions until a block/function boundary (depth 0) or a
/// closing `}` (depth > 0, which the caller consumes).
fn parse_instrs(body: &[String], mut i: usize, depth: usize) -> Result<(Vec<Instr>, usize)> {
    let mut instrs = Vec::new();
    while i < body.len() {
        let line = &body[i];
        if depth > 0 && (line == "}" || line == "} {") {
            return Ok((instrs, i));
        }
        if depth == 0 && (line.starts_with('^') || function_kind(line).is_some()) {
            return Ok((instrs, i));
        }
        let mut toks = tokenise(line)?;
        if toks.is_empty() {
            bail!("object: empty instruction line");
        }
        let opens = toks.last().map(String::as_str) == Some("{");
        if opens {
            toks.pop();
        }
        let mut instr = parse_instr_line(line, &toks)?;
        i += 1;
        if opens {
            loop {
                let (child, next) = parse_instrs(body, i, depth + 1)?;
                i = next;
                instr.children.push(child);
                match body.get(i).map(String::as_str) {
                    Some("}") => {
                        i += 1;
                        break;
                    }
                    Some("} {") => {
                        i += 1;
                        if instr.children.len() >= 2 {
                            bail!("object: instruction in {line:?} has more than two brace groups");
                        }
                    }
                    _ => bail!("object: unterminated brace group after {line:?}"),
                }
            }
        }
        instrs.push(instr);
    }
    if depth > 0 {
        bail!("object: unterminated brace group at end of functions section");
    }
    Ok((instrs, i))
}

fn parse_instr_line(line: &str, toks: &[String]) -> Result<Instr> {
    let (lhs, opcode, rest) = if toks.len() >= 3 && toks[1] == "=" {
        let lhs = if toks[0].starts_with('(') {
            split_top(&toks[0][1..toks[0].len() - 1], ',')
        } else {
            vec![toks[0].clone()]
        };
        (lhs, toks[2].clone(), &toks[3..])
    } else if toks.len() >= 2 && toks[0] == "=" {
        bail!("object: malformed instruction {line:?}");
    } else {
        (Vec::new(), toks[0].clone(), &toks[1..])
    };
    let operands = rest
        .iter()
        .map(|t| parse_operand(t))
        .collect::<Result<Vec<_>>>()?;
    Ok(Instr {
        lhs,
        opcode,
        operands,
        children: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Signatures (§9.1) derived from headers, and validation
// ---------------------------------------------------------------------------

fn jobj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn jint(n: u64) -> Value {
    Value::Int(n.to_string())
}

fn widths(params: &[Param], ctx: &str) -> Result<Vec<Value>> {
    params
        .iter()
        .map(|p| {
            p.ty.wire_width()
                .map(|w| jint(u64::from(w)))
                .ok_or_else(|| {
                    Error(format!(
                        "object: {ctx} parameter {:?} is not a wire/reg bundle",
                        p.name
                    ))
                })
        })
        .collect()
}

fn mem_sig(params: &[Param]) -> Result<Vec<Value>> {
    params
        .iter()
        .map(|p| match p.ty {
            Type::Region(a, w) => {
                let role = p.role.clone().ok_or_else(|| {
                    Error(format!(
                        "object: region parameter {:?} has no role (§5.1)",
                        p.name
                    ))
                })?;
                Ok(jobj(vec![
                    ("A", jint(u64::from(a))),
                    ("W", jint(u64::from(w))),
                    ("role", Value::Str(role)),
                ]))
            }
            _ => bail!("object: mem parameter {:?} is not a region", p.name),
        })
        .collect()
}

fn type_strings(params: &[Param]) -> Result<Vec<Value>> {
    params
        .iter()
        .map(|p| Ok(Value::Str(p.ty.signature_string()?)))
        .collect()
}

fn one_width(params: &[Param], ctx: &str) -> Result<u64> {
    if params.len() != 1 {
        bail!("object: {ctx} group must hold exactly one bundle (§16.2)");
    }
    params[0]
        .ty
        .wire_width()
        .map(u64::from)
        .ok_or_else(|| Error(format!("object: {ctx} is not a wire/reg bundle")))
}

fn expect_empty(params: &[Param], ctx: &str) -> Result<()> {
    if !params.is_empty() {
        bail!("object: {ctx} group must be empty (§16.2)");
    }
    Ok(())
}

/// The §9.1 signature a function header declares.
pub fn derived_signature(f: &Function) -> Result<Value> {
    let [g0, g1, g2, g3, g4] = &f.groups;
    Ok(match f.kind {
        FnKind::Oracle => jobj(vec![
            ("abi", Value::Str(ISA_VERSION.into())),
            ("args", Value::Array(type_strings(g4)?)),
            ("fault", jint(one_width(g2, "oracle fault")?)),
            ("in", Value::Array(widths(g0, "oracle in")?)),
            ("kind", Value::Str("oracle".into())),
            ("mem", Value::Array(mem_sig(g3)?)),
            ("tgt", jint(one_width(g1, "oracle tgt")?)),
            ("tier", Value::Str("q".into())),
        ]),
        FnKind::Action | FnKind::PermAction => {
            expect_empty(g4, "action fifth")?;
            jobj(vec![
                ("abi", Value::Str(ISA_VERSION.into())),
                ("args", Value::Array(type_strings(g3)?)),
                (
                    "bounds",
                    Value::Array(f.bounds.iter().map(|b| jint(*b)).collect()),
                ),
                ("in", Value::Array(widths(g1, "action in")?)),
                ("kind", Value::Str(f.kind.symbol_kind().into())),
                ("mem", Value::Array(mem_sig(g2)?)),
                ("mut", Value::Array(widths(g0, "action mut")?)),
                ("tier", Value::Str("q".into())),
            ])
        }
        FnKind::Lproc => {
            expect_empty(g2, "lproc third")?;
            expect_empty(g3, "lproc fourth")?;
            expect_empty(g4, "lproc fifth")?;
            jobj(vec![
                ("abi", Value::Str(ISA_VERSION.into())),
                ("kind", Value::Str("lproc".into())),
                ("params", Value::Array(widths(g0, "lproc params")?)),
                ("result", jint(one_width(g1, "lproc result")?)),
                ("tier", Value::Str("q".into())),
            ])
        }
        FnKind::Cc | FnKind::R => {
            expect_empty(g2, "fn third")?;
            expect_empty(g3, "fn fourth")?;
            expect_empty(g4, "fn fifth")?;
            jobj(vec![
                ("abi", Value::Str(ISA_VERSION.into())),
                ("kind", Value::Str(f.kind.symbol_kind().into())),
                ("params", Value::Array(type_strings(g0)?)),
                ("results", Value::Array(type_strings(g1)?)),
                ("tier", Value::Str(f.kind.symbol_kind().into())),
            ])
        }
    })
}

fn data_signature(entry: &DataEntry) -> Value {
    jobj(vec![
        ("abi", Value::Str(ISA_VERSION.into())),
        ("kind", Value::Str("data".into())),
        ("size", jint(entry.bytes.len() as u64)),
        ("tier", Value::Str("-".into())),
    ])
}

/// Trip counts of every `qloop` in text order, outer before inner (§5.3).
/// `None` entries are non-literal counts, which milestone 1 cannot decide
/// (lowering rejects them as milestone-2 specialisation).
fn body_bounds(instrs: &[Instr], out: &mut Vec<Option<u64>>) {
    for instr in instrs {
        if instr.opcode == "qloop" {
            match instr.operands.first() {
                Some(Operand::Int(n)) => out.push(n.parse().ok()),
                _ => out.push(None),
            }
        }
        for child in &instr.children {
            body_bounds(child, out);
        }
    }
}

impl Object {
    fn validate(&self) -> Result<()> {
        // Imports must not shadow definitions.
        for imp in &self.imports {
            if self.symbol(&imp.name).is_some() {
                bail!("object: import {:?} duplicates a defined symbol", imp.name);
            }
        }
        // Symbol/function/data correspondence + derived signatures.
        let mut fn_names = BTreeSet::new();
        for f in &self.functions {
            if !fn_names.insert(f.name.clone()) {
                bail!("object: duplicate function @{}", f.name);
            }
            let Some(sym) = self.symbol(&f.name) else {
                bail!("object: function @{} has no symbol entry (§9.1)", f.name);
            };
            if sym.kind != f.kind.symbol_kind() {
                bail!(
                    "object: @{} symbol kind {:?} != function kind {:?}",
                    f.name,
                    sym.kind,
                    f.kind.symbol_kind()
                );
            }
            let derived = derived_signature(f)?;
            if derived != sym.signature {
                bail!(
                    "object: @{} stored signature {} != header signature {} (§9.1)",
                    f.name,
                    sym.signature.to_jcs(),
                    derived.to_jcs()
                );
            }
            if f.is_action() {
                let mut found = Vec::new();
                for b in &f.blocks {
                    body_bounds(&b.instrs, &mut found);
                }
                let declared: Vec<Option<u64>> = f.bounds.iter().map(|b| Some(*b)).collect();
                if found.len() != declared.len()
                    || found
                        .iter()
                        .zip(&declared)
                        .any(|(a, b)| a.is_some() && a != b)
                {
                    bail!(
                        "object: @{} bounds list {:?} does not equal its qloop trip counts {:?} (§5.3)",
                        f.name,
                        f.bounds,
                        found
                    );
                }
            }
        }
        for sym in &self.symbols {
            if sym.kind == "data" {
                let Some(entry) = self.data_entry(&sym.name) else {
                    bail!("object: data symbol {:?} has no data line (§9.1)", sym.name);
                };
                if data_signature(entry) != sym.signature {
                    bail!(
                        "object: data {:?} stored signature does not match its size",
                        sym.name
                    );
                }
            } else if !fn_names.contains(&sym.name) {
                bail!(
                    "object: symbol {:?} ({}) has no function (§9.1)",
                    sym.name,
                    sym.kind
                );
            }
        }
        for entry in &self.data {
            match self.symbol(&entry.name) {
                Some(s) if s.kind == "data" => {}
                _ => bail!(
                    "object: data line {:?} has no data symbol (§9.1)",
                    entry.name
                ),
            }
        }
        // Layout: ptr_width agreement; globals are data symbols (§9.2).
        if let Some(layout) = &self.layout {
            expect_keys(
                "layout",
                layout,
                &[
                    "A_tab",
                    "H",
                    "S_cells",
                    "block_image",
                    "globals",
                    "heap_base",
                    "heap_limit_max",
                    "interface_regions",
                    "null_size",
                    "ptr_width",
                ],
                &[],
            )?;
            if let Some(pw) = layout.get("ptr_width")
                && pw.as_u64() != Some(u64::from(self.header.ptr_width))
            {
                bail!("object: layout ptr_width disagrees with header ptr_width");
            }
            if let Some(globals) = layout.get("globals").and_then(Value::as_object) {
                for (g, gv) in globals {
                    expect_keys(
                        &format!("layout.globals.{g}"),
                        gv,
                        &["base", "image", "size"],
                        &["base", "image", "size"],
                    )?;
                    if gv.get("image").and_then(Value::as_str) != Some(g.as_str()) {
                        bail!("object: global {g:?} image is not its own name (§9.2)");
                    }
                    let Some(entry) = self.data_entry(g) else {
                        bail!("object: global {g:?} has no data symbol (§9.2)");
                    };
                    if gv.get("size").and_then(Value::as_u64) != Some(entry.bytes.len() as u64) {
                        bail!("object: global {g:?} size disagrees with its data entry");
                    }
                }
            }
        }
        if let Some(devices) = &self.devices {
            expect_keys("devices", devices, &["models", "services"], &[])?;
        }
        // Every @reference resolves to a definition or an import.
        for f in &self.functions {
            for r in self.references(f) {
                if self.symbol(&r).is_none() && !self.imports.iter().any(|i| i.name == r) {
                    bail!("object: @{} references unknown symbol @{r}", f.name);
                }
            }
        }
        Ok(())
    }

    /// `@name` references of a function in first-reference order (§9.2 part
    /// 6), excluding its header definition and `lift.addr` naming one of the
    /// containing oracle's own interface regions.
    fn references(&self, f: &Function) -> Vec<String> {
        let (refs, _) = self.collect_refs(f);
        refs
    }

    fn collect_refs(&self, f: &Function) -> (Vec<String>, Vec<String>) {
        let mut refs: Vec<String> = Vec::new();
        let mut addr_refs: Vec<String> = Vec::new();
        let region_names: BTreeSet<&str> = if f.kind == FnKind::Oracle {
            f.groups[3].iter().map(|p| p.name.as_str()).collect()
        } else {
            BTreeSet::new()
        };
        fn walk(
            instrs: &[Instr],
            region_names: &BTreeSet<&str>,
            refs: &mut Vec<String>,
            addr_refs: &mut Vec<String>,
        ) {
            for instr in instrs {
                for op in &instr.operands {
                    if let Operand::Sym(name) = op {
                        let is_region =
                            instr.opcode == "lift.addr" && region_names.contains(name.as_str());
                        if !is_region {
                            if !refs.contains(name) {
                                refs.push(name.clone());
                            }
                            if instr.opcode == "lift.addr" && !addr_refs.contains(name) {
                                addr_refs.push(name.clone());
                            }
                        }
                    }
                }
                for child in &instr.children {
                    walk(child, region_names, refs, addr_refs);
                }
            }
        }
        for b in &f.blocks {
            walk(&b.instrs, &region_names, &mut refs, &mut addr_refs);
        }
        (refs, addr_refs)
    }
}

// ---------------------------------------------------------------------------
// Identity (§9.2) and check
// ---------------------------------------------------------------------------

enum RefHash {
    Hash(String),
    SccMember(usize),
    Missing,
}

fn layout_leaves(layout: &Value) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let Some(map) = layout.as_object() else {
        return out;
    };
    for (k, v) in map {
        match k.as_str() {
            "globals" => {
                if let Some(g) = v.as_object() {
                    for (name, gv) in g {
                        out.push((format!("globals.{name}"), gv.clone()));
                    }
                }
            }
            "interface_regions" => {
                if let Some(o) = v.as_object() {
                    for (oracle, regions) in o {
                        if let Some(r) = regions.as_object() {
                            for (region, rv) in r {
                                out.push((
                                    format!("interface_regions.{oracle}.{region}"),
                                    rv.clone(),
                                ));
                            }
                        }
                    }
                }
            }
            _ => out.push((k.clone(), v.clone())),
        }
    }
    out
}

impl Object {
    fn closure_data_and_leaves(&self, sym: &Symbol) -> Result<ClosureSet> {
        // (data names, layout leaves, device entries) for envelope 5a/5b/5c.
        let mut data_names: Vec<String> = Vec::new();
        let mut leaves: Vec<(String, Value)> = Vec::new();
        let mut device_entries: Vec<(String, Value)> = Vec::new();
        let take_leaf = |path: &str,
                         v: &Value,
                         leaves: &mut Vec<(String, Value)>,
                         data_names: &mut Vec<String>| {
            leaves.push((path.to_string(), v.clone()));
            if let Some(g) = path.strip_prefix("globals.")
                && let Some(image) = v.get("image").and_then(Value::as_str)
            {
                let _ = g;
                if !data_names.contains(&image.to_string()) {
                    data_names.push(image.to_string());
                }
            }
            if path == "block_image"
                && let Some(image) = v.as_str()
                && !data_names.contains(&image.to_string())
            {
                data_names.push(image.to_string());
            }
        };
        if let Some(layout) = &self.layout {
            let all = layout_leaves(layout);
            match sym.kind.as_str() {
                "oracle" => {
                    for (path, v) in &all {
                        if let Some(rest) = path.strip_prefix("interface_regions.") {
                            let owner = rest.split('.').next().unwrap_or("");
                            if owner != sym.name {
                                continue;
                            }
                        }
                        take_leaf(path, v, &mut leaves, &mut data_names);
                    }
                }
                "lproc" => {
                    if let Some(f) = self.function(&sym.name) {
                        let (_, addr_refs) = self.collect_refs(f);
                        for (path, v) in &all {
                            if let Some(g) = path.strip_prefix("globals.")
                                && addr_refs.iter().any(|r| r == g)
                            {
                                take_leaf(path, v, &mut leaves, &mut data_names);
                            }
                        }
                    }
                }
                "cc" | "r" => {
                    for (path, v) in &all {
                        take_leaf(path, v, &mut leaves, &mut data_names);
                    }
                }
                _ => {}
            }
        }
        if matches!(sym.kind.as_str(), "cc" | "r")
            && let Some(f) = self.function(&sym.name)
        {
            let mut used: Vec<(String, bool)> = Vec::new(); // (name, is_service)
            fn walk(instrs: &[Instr], used: &mut Vec<(String, bool)>) {
                for instr in instrs {
                    match instr.opcode.as_str() {
                        "effect" => {
                            if let Some(Operand::Name(devop)) = instr.operands.first() {
                                let dev = devop.split('.').next().unwrap_or(devop).to_string();
                                if !used.contains(&(dev.clone(), false)) {
                                    used.push((dev, false));
                                }
                            }
                        }
                        "hostcall" => {
                            if let Some(Operand::Name(svc)) = instr.operands.first()
                                && !used.contains(&(svc.clone(), true))
                            {
                                used.push((svc.clone(), true));
                            }
                        }
                        _ => {}
                    }
                    for child in &instr.children {
                        walk(child, used);
                    }
                }
            }
            for b in &f.blocks {
                walk(&b.instrs, &mut used);
            }
            for (name, is_service) in &used {
                // An undeclared device is legal (its effect faults
                // FAULT_EFFECT_UNMODELLED, §17 item 3); it simply has no
                // devices entry to bind into the envelope.
                if let Some(entry) = self
                    .devices
                    .as_ref()
                    .and_then(|d| d.get(if *is_service { "services" } else { "models" }))
                    .and_then(|m| m.get(name))
                    .cloned()
                {
                    device_entries.push((name.clone(), entry));
                }
            }
            device_entries.sort_by(|a, b| a.0.cmp(&b.0));
        }
        data_names.sort();
        leaves.sort_by(|a, b| a.0.cmp(&b.0));
        // Every named image must exist as a data entry.
        for name in &data_names {
            if self.data_entry(name).is_none() {
                bail!(
                    "object: closure of {:?} names missing data image {name:?}",
                    sym.name
                );
            }
        }
        Ok((data_names, leaves, device_entries))
    }

    fn build_envelope(&self, sym: &Symbol, resolve: &dyn Fn(&str) -> RefHash) -> Result<Vec<u8>> {
        let mut out = String::new();
        out.push_str(&format!("isa={}\n", self.header.isa));
        out.push_str(&format!("ptr_width={}\n", self.header.ptr_width));
        out.push_str(&sym.signature.to_jcs());
        out.push('\n');
        if sym.kind == "data" {
            let entry = self
                .data_entry(&sym.name)
                .ok_or_else(|| Error(format!("object: data {:?} missing", sym.name)))?;
            out.push_str(&format!("{} {}\n", entry.name, entry.hex));
            return Ok(out.into_bytes());
        }
        let f = self
            .function(&sym.name)
            .ok_or_else(|| Error(format!("object: function @{} missing", sym.name)))?;
        let refs = self.references(f);
        out.push_str(&replace_refs(&f.canonical_bytes, &f.name, &refs, resolve));
        out.push('\n');
        let (data_names, leaves, device_entries) = self.closure_data_and_leaves(sym)?;
        for name in &data_names {
            let entry = self.data_entry(name).expect("checked");
            out.push_str(&format!("data {} {}\n", entry.name, entry.hex));
        }
        for (path, v) in &leaves {
            out.push_str(&format!("layout {path} {}\n", v.to_jcs()));
        }
        for (name, v) in &device_entries {
            out.push_str(&format!("device {name} {}\n", v.to_jcs()));
        }
        for r in &refs {
            match resolve(r) {
                RefHash::Hash(h) => {
                    out.push_str(&h);
                    out.push('\n');
                }
                RefHash::SccMember(_) => {} // intra-SCC: no part-6 line
                RefHash::Missing => bail!(
                    "object: unresolved hash for @{r} while building @{} envelope",
                    sym.name
                ),
            }
        }
        Ok(out.into_bytes())
    }

    /// One identity pass. `closure=false` computes interface hashes
    /// (imports by pseudo-hash); `closure=true` computes closure hashes
    /// (`None` where the closure holds an unpinned import).
    fn compute_hashes(&self, closure: bool) -> Result<BTreeMap<String, Option<String>>> {
        let refs_of: BTreeMap<String, Vec<String>> = self
            .symbols
            .iter()
            .map(|s| {
                let refs = if s.kind == "data" {
                    Vec::new()
                } else {
                    self.function(&s.name)
                        .map(|f| self.references(f))
                        .unwrap_or_default()
                };
                (s.name.clone(), refs)
            })
            .collect();
        let defined: BTreeSet<&str> = self.symbols.iter().map(|s| s.name.as_str()).collect();
        let sccs = strongly_connected(&refs_of, &defined);

        let import_hash = |name: &str| -> Option<String> {
            let imp = self.imports.iter().find(|i| i.name == name)?;
            if closure {
                imp.pinned_closure_hash.clone()
            } else {
                let mut bytes = name.as_bytes().to_vec();
                bytes.push(0);
                bytes.extend_from_slice(imp.signature.to_jcs().as_bytes());
                Some(sha256_hex(&bytes))
            }
        };

        let mut done: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut remaining: Vec<&Vec<String>> = sccs.iter().collect();
        while !remaining.is_empty() {
            let mut progress = false;
            let mut next = Vec::new();
            for scc in remaining {
                let ready = scc.iter().all(|m| {
                    refs_of[m].iter().all(|r| {
                        !defined.contains(r.as_str()) || scc.contains(r) || done.contains_key(r)
                    })
                });
                if !ready {
                    next.push(scc);
                    continue;
                }
                progress = true;
                // The whole SCC is None iff some needed external hash is.
                let missing = scc.iter().any(|m| {
                    refs_of[m].iter().any(|r| {
                        if scc.contains(r) {
                            false
                        } else if defined.contains(r.as_str()) {
                            done.get(r).is_some_and(|h| h.is_none())
                        } else {
                            import_hash(r).is_none()
                        }
                    })
                });
                if missing {
                    for m in scc.iter() {
                        done.insert(m.clone(), None);
                    }
                    continue;
                }
                let mut envelopes = Vec::new();
                for m in scc.iter() {
                    let sym = self.symbol(m).expect("defined");
                    let scc_pos: BTreeMap<&str, usize> = scc
                        .iter()
                        .enumerate()
                        .map(|(i, n)| (n.as_str(), i))
                        .collect();
                    let resolve = |r: &str| -> RefHash {
                        if let Some(i) = scc_pos.get(r) {
                            RefHash::SccMember(*i)
                        } else if defined.contains(r) {
                            match done.get(r) {
                                Some(Some(h)) => RefHash::Hash(h.clone()),
                                _ => RefHash::Missing,
                            }
                        } else {
                            match import_hash(r) {
                                Some(h) => RefHash::Hash(h),
                                None => RefHash::Missing,
                            }
                        }
                    };
                    envelopes.push(self.build_envelope(sym, &resolve)?);
                }
                let cyclic = scc.len() > 1 || refs_of[&scc[0]].contains(&scc[0]);
                if !cyclic {
                    done.insert(scc[0].clone(), Some(sha256_hex(&envelopes[0])));
                } else {
                    let mut cat = Vec::new();
                    for e in &envelopes {
                        cat.extend_from_slice(e);
                    }
                    let scc_hash = sha256_hex(&cat);
                    for (i, m) in scc.iter().enumerate() {
                        done.insert(
                            m.clone(),
                            Some(sha256_hex(format!("{scc_hash}:{i}").as_bytes())),
                        );
                    }
                }
            }
            if !progress {
                bail!("object: symbol reference graph has an unresolvable cycle (internal error)");
            }
            remaining = next;
        }
        Ok(done)
    }

    pub(crate) fn identity(&self) -> Result<BTreeMap<String, SymbolHashes>> {
        let interface = self.compute_hashes(false)?;
        let closure = self.compute_hashes(true)?;
        let mut out = BTreeMap::new();
        for s in &self.symbols {
            let ih = interface
                .get(&s.name)
                .cloned()
                .flatten()
                .ok_or_else(|| Error(format!("object: no interface hash for {:?}", s.name)))?;
            out.insert(
                s.name.clone(),
                SymbolHashes {
                    interface_hash: ih,
                    closure_hash: closure.get(&s.name).cloned().flatten(),
                },
            );
        }
        Ok(out)
    }

    /// The semantic envelope used for one symbol's interface hash (§9.2),
    /// after reference replacement. Exposed for tests and debugging.
    pub fn semantic_envelope(&self, name: &str) -> Result<Vec<u8>> {
        self.validate()?;
        let hashes = self.identity()?;
        let sym = self
            .symbol(name)
            .ok_or_else(|| Error(format!("object: unknown symbol {name:?}")))?;
        // Rebuild with interface hashes; intra-SCC refs need SCC indices.
        let refs_of: BTreeMap<String, Vec<String>> = self
            .symbols
            .iter()
            .map(|s| {
                (
                    s.name.clone(),
                    if s.kind == "data" {
                        Vec::new()
                    } else {
                        self.function(&s.name)
                            .map(|f| self.references(f))
                            .unwrap_or_default()
                    },
                )
            })
            .collect();
        let defined: BTreeSet<&str> = self.symbols.iter().map(|s| s.name.as_str()).collect();
        let sccs = strongly_connected(&refs_of, &defined);
        let scc = sccs
            .iter()
            .find(|c| c.iter().any(|m| m == name))
            .cloned()
            .unwrap_or_else(|| vec![name.to_string()]);
        let scc_pos: BTreeMap<&str, usize> = scc
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();
        let resolve = |r: &str| -> RefHash {
            if let Some(i) = scc_pos.get(r) {
                RefHash::SccMember(*i)
            } else if let Some(h) = hashes.get(r) {
                RefHash::Hash(h.interface_hash.clone())
            } else if let Some(imp) = self.imports.iter().find(|i| i.name == r) {
                let mut bytes = r.as_bytes().to_vec();
                bytes.push(0);
                bytes.extend_from_slice(imp.signature.to_jcs().as_bytes());
                RefHash::Hash(sha256_hex(&bytes))
            } else {
                RefHash::Missing
            }
        };
        self.build_envelope(sym, &resolve)
    }

    pub fn object_hash(&self) -> String {
        let removable = ["schedule", "certificate", "coverage"];
        if !self
            .section_spans
            .iter()
            .any(|(n, _, _)| removable.contains(&n.as_str()))
        {
            return sha256_hex(self.text.as_bytes());
        }
        let mut lines: Vec<&str> = self.text.split('\n').collect();
        if lines.last() == Some(&"") {
            lines.pop();
        }
        let mut kept: Vec<&str> = Vec::new();
        let mut idx = 0usize;
        let mut spans: Vec<&(String, usize, usize)> = self.section_spans.iter().collect();
        spans.sort_by_key(|s| s.1);
        for (name, start, end) in spans {
            if !removable.contains(&name.as_str()) {
                continue;
            }
            kept.extend_from_slice(&lines[idx..*start]);
            idx = *end;
        }
        kept.extend_from_slice(&lines[idx..]);
        let mut text = kept.join("\n");
        if self.text.ends_with('\n') {
            text.push('\n');
        }
        sha256_hex(text.as_bytes())
    }

    /// Parse-level validation plus identity: verifies every stored
    /// interface hash. Mirrors the reference `check` report shape.
    pub fn check(&self) -> Result<CheckReport> {
        self.validate()?;
        crate::checker::validate(self)?;
        let symbols = self.identity()?;
        for s in &self.symbols {
            let computed = &symbols[&s.name].interface_hash;
            if *computed != s.stored_interface_hash {
                bail!(
                    "check: symbol {:?} stored interface_hash {} != computed {} (§9.2)",
                    s.name,
                    s.stored_interface_hash,
                    computed
                );
            }
        }
        let unresolved_imports: Vec<String> = self.imports.iter().map(|i| i.name.clone()).collect();
        Ok(CheckReport {
            object_hash: self.object_hash(),
            status: if unresolved_imports.is_empty() {
                "ok"
            } else {
                "unlinked"
            }
            .into(),
            symbols,
            unresolved_imports,
        })
    }

    /// Identities without comparing stored hashes (the reference `hash`).
    pub fn hash(&self) -> Result<CheckReport> {
        self.validate()?;
        crate::checker::validate(self)?;
        let symbols = self.identity()?;
        let unresolved_imports: Vec<String> = self.imports.iter().map(|i| i.name.clone()).collect();
        Ok(CheckReport {
            object_hash: self.object_hash(),
            status: if unresolved_imports.is_empty() {
                "ok"
            } else {
                "unlinked"
            }
            .into(),
            symbols,
            unresolved_imports,
        })
    }
}

/// Replace `@ref` tokens in canonical function bytes per §9.2. The
/// definition token in the header line is left alone; names outside the
/// reference set (interface regions) are left alone.
fn replace_refs(
    canonical: &str,
    def_name: &str,
    refs: &[String],
    resolve: &dyn Fn(&str) -> RefHash,
) -> String {
    let bytes = canonical.as_bytes();
    let mut out = String::with_capacity(canonical.len() + 64);
    let mut i = 0usize;
    let mut definition_done = false;
    while i < bytes.len() {
        if bytes[i] == b'@' && i + 1 < bytes.len() && is_name_start(bytes[i + 1]) {
            let mut j = i + 1;
            while j < bytes.len() && is_name_char(bytes[j]) {
                j += 1;
            }
            let name = &canonical[i + 1..j];
            if !definition_done && name == def_name {
                definition_done = true;
                out.push_str(&canonical[i..j]);
            } else if refs.iter().any(|r| r == name) {
                match resolve(name) {
                    RefHash::Hash(h) => {
                        out.push('@');
                        out.push_str(&h);
                    }
                    RefHash::SccMember(idx) => out.push_str(&format!("@${idx}")),
                    RefHash::Missing => out.push_str(&canonical[i..j]),
                }
            } else {
                out.push_str(&canonical[i..j]);
            }
            i = j;
        } else {
            // Copy one UTF-8 char.
            let ch_len = canonical[i..]
                .chars()
                .next()
                .expect("valid utf8")
                .len_utf8();
            out.push_str(&canonical[i..i + ch_len]);
            i += ch_len;
        }
    }
    out
}

/// Tarjan SCC over defined symbols; members of each SCC sorted by name
/// bytewise (§9.2). Iteration uses sorted names for determinism.
pub(crate) fn strongly_connected(
    refs_of: &BTreeMap<String, Vec<String>>,
    defined: &BTreeSet<&str>,
) -> Vec<Vec<String>> {
    struct State<'a> {
        refs_of: &'a BTreeMap<String, Vec<String>>,
        defined: &'a BTreeSet<&'a str>,
        index: BTreeMap<String, usize>,
        low: BTreeMap<String, usize>,
        on_stack: BTreeSet<String>,
        stack: Vec<String>,
        next: usize,
        out: Vec<Vec<String>>,
    }
    fn visit(st: &mut State<'_>, v: &str) {
        st.index.insert(v.to_string(), st.next);
        st.low.insert(v.to_string(), st.next);
        st.next += 1;
        st.stack.push(v.to_string());
        st.on_stack.insert(v.to_string());
        let succs: Vec<String> = st.refs_of[v]
            .iter()
            .filter(|r| st.defined.contains(r.as_str()))
            .cloned()
            .collect();
        for w in succs {
            if !st.index.contains_key(&w) {
                visit(st, &w);
                let lw = st.low[&w];
                let lv = st.low[v];
                st.low.insert(v.to_string(), lv.min(lw));
            } else if st.on_stack.contains(&w) {
                let iw = st.index[&w];
                let lv = st.low[v];
                st.low.insert(v.to_string(), lv.min(iw));
            }
        }
        if st.low[v] == st.index[v] {
            let mut scc = Vec::new();
            loop {
                let w = st.stack.pop().expect("stack nonempty");
                st.on_stack.remove(&w);
                let done = w == v;
                scc.push(w);
                if done {
                    break;
                }
            }
            scc.sort();
            st.out.push(scc);
        }
    }
    let mut st = State {
        refs_of,
        defined,
        index: BTreeMap::new(),
        low: BTreeMap::new(),
        on_stack: BTreeSet::new(),
        stack: Vec::new(),
        next: 0,
        out: Vec::new(),
    };
    let names: Vec<String> = refs_of.keys().cloned().collect();
    for v in &names {
        if !st.index.contains_key(v) {
            visit(&mut st, v);
        }
    }
    st.out
}
