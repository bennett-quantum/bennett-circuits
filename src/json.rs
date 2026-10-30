//! JSON values in the subset the ISA object format uses, with RFC 8785
//! (JCS) canonical serialisation. Numbers are integers only: §9.1 spells
//! integers above 2^53 as decimal strings and every decimal in the
//! schedule/certificate schemas is a string, so a non-integer JSON number
//! in an object metadata line is a loud load error here.

use std::collections::BTreeMap;
use std::fmt;

use crate::{Error, Result, bail};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// Integer, kept as its canonical decimal text.
    Int(String),
    Str(String),
    Array(Vec<Value>),
    /// Keys unique. Serialization sorts by UTF-16 code units (RFC 8785),
    /// including non-ASCII user keys.
    Object(BTreeMap<String, Value>),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_jcs())
    }
}

impl Value {
    pub fn parse(text: &str) -> Result<Value> {
        let mut p = Parser {
            bytes: text.as_bytes(),
            pos: 0,
        };
        let v = p.value()?;
        p.ws();
        if p.pos != p.bytes.len() {
            bail!("JSON: trailing input at byte {}", p.pos);
        }
        Ok(v)
    }

    /// Canonical (JCS) serialisation.
    pub fn to_jcs(&self) -> String {
        let mut out = String::new();
        self.write_jcs(&mut out);
        out
    }

    fn write_jcs(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(true) => out.push_str("true"),
            Value::Bool(false) => out.push_str("false"),
            Value::Int(n) => out.push_str(n),
            Value::Str(s) => write_jcs_string(s, out),
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_jcs(out);
                }
                out.push(']');
            }
            Value::Object(map) => {
                out.push('{');
                let mut entries = map.iter().collect::<Vec<_>>();
                entries.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
                for (i, (k, v)) in entries.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_jcs_string(k, out);
                    out.push(':');
                    v.write_jcs(out);
                }
                out.push('}');
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(map) => map.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Int(n) => n.parse().ok(),
            _ => None,
        }
    }

    pub fn as_i128(&self) -> Option<i128> {
        match self {
            Value::Int(n) => n.parse().ok(),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

fn write_jcs_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{09}' => out.push_str("\\t"),
            '\u{0a}' => out.push_str("\\n"),
            '\u{0c}' => out.push_str("\\f"),
            '\u{0d}' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.pos < self.bytes.len()
            && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\n' | b'\r')
        {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Result<u8> {
        self.bytes
            .get(self.pos)
            .copied()
            .ok_or_else(|| Error("JSON: unexpected end of input".into()))
    }

    fn value(&mut self) -> Result<Value> {
        self.ws();
        match self.peek()? {
            b'n' => self.lit("null", Value::Null),
            b't' => self.lit("true", Value::Bool(true)),
            b'f' => self.lit("false", Value::Bool(false)),
            b'"' => Ok(Value::Str(self.string()?)),
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek()? == b']' {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                loop {
                    items.push(self.value()?);
                    self.ws();
                    match self.peek()? {
                        b',' => self.pos += 1,
                        b']' => {
                            self.pos += 1;
                            return Ok(Value::Array(items));
                        }
                        c => bail!(
                            "JSON: expected ',' or ']' at byte {}, got {:?}",
                            self.pos,
                            c as char
                        ),
                    }
                }
            }
            b'{' => {
                self.pos += 1;
                let mut map = BTreeMap::new();
                self.ws();
                if self.peek()? == b'}' {
                    self.pos += 1;
                    return Ok(Value::Object(map));
                }
                loop {
                    self.ws();
                    let key = self.string()?;
                    self.ws();
                    if self.peek()? != b':' {
                        bail!("JSON: expected ':' at byte {}", self.pos);
                    }
                    self.pos += 1;
                    let v = self.value()?;
                    if map.insert(key.clone(), v).is_some() {
                        bail!("JSON: duplicate object key {key:?}");
                    }
                    self.ws();
                    match self.peek()? {
                        b',' => self.pos += 1,
                        b'}' => {
                            self.pos += 1;
                            return Ok(Value::Object(map));
                        }
                        c => bail!(
                            "JSON: expected ',' or '}}' at byte {}, got {:?}",
                            self.pos,
                            c as char
                        ),
                    }
                }
            }
            b'-' | b'0'..=b'9' => self.number(),
            c => bail!(
                "JSON: unexpected character {:?} at byte {}",
                c as char,
                self.pos
            ),
        }
    }

    fn lit(&mut self, word: &str, v: Value) -> Result<Value> {
        if self.bytes.len() - self.pos >= word.len()
            && &self.bytes[self.pos..self.pos + word.len()] == word.as_bytes()
        {
            self.pos += word.len();
            Ok(v)
        } else {
            bail!("JSON: bad literal at byte {}", self.pos)
        }
    }

    fn number(&mut self) -> Result<Value> {
        let start = self.pos;
        if self.peek()? == b'-' {
            self.pos += 1;
        }
        match self.peek()? {
            b'0' => self.pos += 1,
            b'1'..=b'9' => {
                while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
            }
            _ => bail!("JSON: bad number at byte {start}"),
        }
        if self.pos < self.bytes.len() && matches!(self.bytes[self.pos], b'.' | b'e' | b'E') {
            bail!(
                "JSON: non-integer number at byte {start} is outside the object-format subset (decimals are strings, §9.1)"
            );
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .expect("digits are utf8")
            .to_string();
        // Canonicalise -0 to 0 (JCS).
        Ok(Value::Int(if text == "-0" { "0".into() } else { text }))
    }

    fn string(&mut self) -> Result<String> {
        if self.peek()? != b'"' {
            bail!("JSON: expected string at byte {}", self.pos);
        }
        self.pos += 1;
        let mut out = String::new();
        loop {
            let b = self.peek()?;
            self.pos += 1;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = self.peek()?;
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let cp = self.hex4()?;
                            if (0xd800..0xdc00).contains(&cp) {
                                // High surrogate: a low one must follow.
                                if self.bytes.get(self.pos) == Some(&b'\\')
                                    && self.bytes.get(self.pos + 1) == Some(&b'u')
                                {
                                    self.pos += 2;
                                    let lo = self.hex4()?;
                                    if !(0xdc00..0xe000).contains(&lo) {
                                        bail!("JSON: bad low surrogate");
                                    }
                                    let c = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
                                    out.push(char::from_u32(c).ok_or("JSON: bad code point")?);
                                } else {
                                    bail!("JSON: lone high surrogate");
                                }
                            } else if (0xdc00..0xe000).contains(&cp) {
                                bail!("JSON: lone low surrogate");
                            } else {
                                out.push(char::from_u32(cp).ok_or("JSON: bad code point")?);
                            }
                        }
                        _ => bail!("JSON: bad escape at byte {}", self.pos - 1),
                    }
                }
                0x00..=0x1f => bail!("JSON: control character in string"),
                _ => {
                    // UTF-8 passthrough: copy the full sequence.
                    let start = self.pos - 1;
                    let len = if b < 0x80 {
                        1
                    } else if b >= 0xf0 {
                        4
                    } else if b >= 0xe0 {
                        3
                    } else {
                        2
                    };
                    let end = start + len;
                    if end > self.bytes.len() {
                        bail!("JSON: truncated UTF-8 in string");
                    }
                    let s = std::str::from_utf8(&self.bytes[start..end])
                        .map_err(|_| Error("JSON: invalid UTF-8 in string".into()))?;
                    out.push_str(s);
                    self.pos = end;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32> {
        if self.pos + 4 > self.bytes.len() {
            bail!("JSON: truncated \\u escape");
        }
        let s = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
            .map_err(|_| Error("JSON: bad \\u escape".into()))?;
        let v = u32::from_str_radix(s, 16).map_err(|_| Error("JSON: bad \\u escape".into()))?;
        self.pos += 4;
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jcs_orders_utf16_keys_and_normalises_negative_zero() {
        let v = Value::parse("{\"\\ue000\":-0,\"\\ud800\\udc00\":1}").unwrap();
        assert_eq!(v.to_jcs(), "{\"\u{10000}\":1,\"\u{e000}\":0}");
    }

    #[test]
    fn jcs_orders_keys_and_roundtrips() {
        let v = Value::parse("{\"b\":1,\"a\":[true,null,\"x\"]}").unwrap();
        assert_eq!(v.to_jcs(), "{\"a\":[true,null,\"x\"],\"b\":1}");
        let sig = "{\"abi\":\"0.8\",\"args\":[],\"fault\":4,\"in\":[8],\"kind\":\"oracle\",\"mem\":[],\"tgt\":8,\"tier\":\"q\"}";
        assert_eq!(Value::parse(sig).unwrap().to_jcs(), sig);
    }

    #[test]
    fn rejects_floats_and_dupes() {
        assert!(Value::parse("{\"a\":1.5}").is_err());
        assert!(Value::parse("{\"a\":1,\"a\":2}").is_err());
        assert!(Value::parse("[1,]").is_err());
        assert!(Value::parse("{} x").is_err());
    }
}
