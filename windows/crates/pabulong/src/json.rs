//! Minimal JSON for settings, call metadata and diagnostics, so the app
//! needs no serialization dependency.

use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub fn object() -> Self {
        Value::Object(BTreeMap::new())
    }

    /// Adds a field to an object (no-op on other values). Builder style.
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        if let Value::Object(map) = &mut self {
            map.insert(key.to_string(), value.into());
        }
        self
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(map) => map.get(key),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    /// Pretty-printed with sorted keys (like `JSONEncoder` with
    /// `.prettyPrinted, .sortedKeys` on macOS).
    pub fn to_pretty(&self) -> String {
        let mut out = String::new();
        write_value(self, 0, &mut out);
        out.push('\n');
        out
    }

    pub fn parse(text: &str) -> Result<Value, ParseError> {
        let mut p = Parser { bytes: text.as_bytes(), pos: 0 };
        p.skip_ws();
        let v = p.value(0)?;
        p.skip_ws();
        if p.pos != p.bytes.len() {
            return Err(ParseError(p.pos));
        }
        Ok(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Number(v)
    }
}
impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Number(f64::from(v))
    }
}
impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Number(v as f64)
    }
}
impl From<u64> for Value {
    fn from(v: u64) -> Self {
        Value::Number(v as f64)
    }
}
impl From<u32> for Value {
    fn from(v: u32) -> Self {
        Value::Number(f64::from(v))
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_string())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}
impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Self {
        Value::Array(v)
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_value(v: &Value, indent: usize, out: &mut String) {
    let pad = |n: usize, out: &mut String| out.extend(std::iter::repeat_n(' ', n * 2));
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.is_finite() {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    let _ = write!(out, "{}", *n as i64);
                } else {
                    let _ = write!(out, "{n}");
                }
            } else {
                out.push_str("null");
            }
        }
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                pad(indent + 1, out);
                write_value(item, indent + 1, out);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            pad(indent, out);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            for (i, (k, item)) in map.iter().enumerate() {
                pad(indent + 1, out);
                write_string(k, out);
                out.push_str(" : ");
                write_value(item, indent + 1, out);
                if i + 1 < map.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            pad(indent, out);
            out.push('}');
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct ParseError(pub usize);

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && matches!(self.bytes[self.pos], b' ' | b'\n' | b'\r' | b'\t') {
            self.pos += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> Result<(), ParseError> {
        if self.bytes[self.pos..].starts_with(lit.as_bytes()) {
            self.pos += lit.len();
            Ok(())
        } else {
            Err(ParseError(self.pos))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > 64 {
            return Err(ParseError(self.pos));
        }
        match self.bytes.get(self.pos) {
            Some(b'n') => self.eat("null").map(|_| Value::Null),
            Some(b't') => self.eat("true").map(|_| Value::Bool(true)),
            Some(b'f') => self.eat("false").map(|_| Value::Bool(false)),
            Some(b'"') => self.string().map(Value::String),
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                self.skip_ws();
                if self.bytes.get(self.pos) == Some(&b']') {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                loop {
                    self.skip_ws();
                    items.push(self.value(depth + 1)?);
                    self.skip_ws();
                    match self.bytes.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(Value::Array(items));
                        }
                        _ => return Err(ParseError(self.pos)),
                    }
                }
            }
            Some(b'{') => {
                self.pos += 1;
                let mut map = BTreeMap::new();
                self.skip_ws();
                if self.bytes.get(self.pos) == Some(&b'}') {
                    self.pos += 1;
                    return Ok(Value::Object(map));
                }
                loop {
                    self.skip_ws();
                    let key = self.string()?;
                    self.skip_ws();
                    self.eat(":")?;
                    self.skip_ws();
                    let v = self.value(depth + 1)?;
                    map.insert(key, v);
                    self.skip_ws();
                    match self.bytes.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {
                            self.pos += 1;
                            return Ok(Value::Object(map));
                        }
                        _ => return Err(ParseError(self.pos)),
                    }
                }
            }
            Some(b'-' | b'0'..=b'9') => {
                let start = self.pos;
                while self.pos < self.bytes.len() && matches!(self.bytes[self.pos], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.pos += 1;
                }
                std::str::from_utf8(&self.bytes[start..self.pos])
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .map(Value::Number)
                    .ok_or(ParseError(start))
            }
            _ => Err(ParseError(self.pos)),
        }
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let s = self.bytes.get(self.pos..self.pos + 4).ok_or(ParseError(self.pos))?;
        let v = std::str::from_utf8(s).ok().and_then(|s| u32::from_str_radix(s, 16).ok()).ok_or(ParseError(self.pos))?;
        self.pos += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.eat("\"")?;
        let mut out = String::new();
        loop {
            let start = self.pos;
            while self.pos < self.bytes.len() && self.bytes[self.pos] != b'"' && self.bytes[self.pos] != b'\\' {
                self.pos += 1;
            }
            out.push_str(std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| ParseError(start))?);
            match self.bytes.get(self.pos) {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let c = *self.bytes.get(self.pos).ok_or(ParseError(self.pos))?;
                    self.pos += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xD800..0xDC00).contains(&code) {
                                self.eat("\\u")?;
                                let low = self.hex4()?;
                                code = 0x10000 + ((code - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(ParseError(self.pos - 1)),
                    }
                }
                _ => return Err(ParseError(self.pos)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let v = Value::object()
            .with("name", "Pabulong \"Mic\"\n")
            .with("on", true)
            .with("level", 2i64)
            .with("gain", 0.25f64)
            .with("none", Option::<String>::None)
            .with("list", vec![Value::from(1i64), Value::from("ä — ✓")]);
        let text = v.to_pretty();
        assert_eq!(Value::parse(&text).unwrap(), v);
    }

    #[test]
    fn parses_escapes_and_rejects_garbage() {
        assert_eq!(Value::parse(r#""é😀""#).unwrap(), Value::from("é😀"));
        assert!(Value::parse("{\"a\": }").is_err());
        assert!(Value::parse("[1,2] x").is_err());
    }
}
