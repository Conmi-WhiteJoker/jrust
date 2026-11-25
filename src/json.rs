//! 最小 JSON 实现：只服务于 DAP（调试适配器协议）。
//!
//! 项目坚持零外部依赖，所以这里手写一个够用的 JSON 解析/序列化：
//! 支持 null / bool / 数字 / 字符串 / 数组 / 对象，够处理 DAP 的请求与响应。

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn parse(src: &str) -> Result<Json, String> {
        let mut p = Parser {
            b: src.as_bytes(),
            i: 0,
        };
        p.skip_ws();
        let v = p.value()?;
        p.skip_ws();
        if p.i != p.b.len() {
            return Err(format!("JSON 末尾有多余内容（位置 {}）", p.i));
        }
        Ok(v)
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Num(n) => Some(*n as i64),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(v) => Some(v),
            _ => None,
        }
    }

    /// 紧凑序列化（DAP 不需要缩进）。
    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    out.push_str(&format!("{}", *n as i64));
                } else {
                    out.push_str(&format!("{}", n));
                }
            }
            Json::Str(s) => write_string(s, out),
            Json::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Json::Obj(items) => {
                out.push('{');
                for (i, (k, v)) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// 便捷构造：`obj(vec![("id", Json::Num(1.0))])`
pub fn obj(items: Vec<(&str, Json)>) -> Json {
    Json::Obj(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

pub fn s(v: impl Into<String>) -> Json {
    Json::Str(v.into())
}

pub fn n(v: i64) -> Json {
    Json::Num(v as f64)
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
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\r' | b'\n') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn value(&mut self) -> Result<Json, String> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => {
                self.lit("true")?;
                Ok(Json::Bool(true))
            }
            Some(b'f') => {
                self.lit("false")?;
                Ok(Json::Bool(false))
            }
            Some(b'n') => {
                self.lit("null")?;
                Ok(Json::Null)
            }
            Some(b'-') | Some(b'0'..=b'9') => self.number(),
            other => Err(format!(
                "JSON 无法识别的字符 {:?}（位置 {}）",
                other.map(|b| b as char),
                self.i
            )),
        }
    }

    fn lit(&mut self, text: &str) -> Result<(), String> {
        if self.b[self.i..].starts_with(text.as_bytes()) {
            self.i += text.len();
            Ok(())
        } else {
            Err(format!("JSON 期望 `{}`（位置 {}）", text, self.i))
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        let text = std::str::from_utf8(&self.b[start..self.i])
            .map_err(|_| "JSON 数字不是合法 UTF-8".to_string())?;
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| format!("JSON 数字非法: {}", text))
    }

    fn string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return Err(format!("JSON 期望字符串（位置 {}）", self.i));
        }
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = self
                .peek()
                .ok_or_else(|| "JSON 字符串未闭合".to_string())?;
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.i += 1;
                    let esc = self
                        .peek()
                        .ok_or_else(|| "JSON 转义未完成".to_string())?;
                    self.i += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = self.take(4)?;
                            let code = u32::from_str_radix(&hex, 16)
                                .map_err(|_| format!("JSON \\u 转义非法: {}", hex))?;
                            // 代理对：高位后面必须跟 \uDC00-\uDFFF
                            if (0xD800..=0xDBFF).contains(&code) {
                                if self.take(2)? != "\\u" {
                                    return Err("JSON 代理对缺少低位".to_string());
                                }
                                let low_hex = self.take(4)?;
                                let low = u32::from_str_radix(&low_hex, 16)
                                    .map_err(|_| format!("JSON \\u 转义非法: {}", low_hex))?;
                                let c = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                out.push(char::from_u32(c).unwrap_or('\u{fffd}'));
                            } else {
                                out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                            }
                        }
                        other => {
                            return Err(format!("JSON 未知转义 \\{}", other as char));
                        }
                    }
                }
                _ => {
                    // 逐字节读取 UTF-8 字符
                    let rest = std::str::from_utf8(&self.b[self.i..])
                        .map_err(|_| "JSON 字符串不是合法 UTF-8".to_string())?;
                    let ch = rest.chars().next().unwrap();
                    out.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
    }

    fn take(&mut self, n: usize) -> Result<String, String> {
        if self.i + n > self.b.len() {
            return Err("JSON 输入意外结束".to_string());
        }
        let text = String::from_utf8_lossy(&self.b[self.i..self.i + n]).into_owned();
        self.i += n;
        Ok(text)
    }

    fn array(&mut self) -> Result<Json, String> {
        self.i += 1; // [
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.i += 1;
                }
                Some(b']') => {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                _ => return Err(format!("JSON 数组期望 `,` 或 `]`（位置 {}）", self.i)),
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.i += 1; // {
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Json::Obj(items));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(format!("JSON 对象期望 `:`（位置 {}）", self.i));
            }
            self.i += 1;
            self.skip_ws();
            let value = self.value()?;
            items.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.i += 1;
                }
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Json::Obj(items));
                }
                _ => return Err(format!("JSON 对象期望 `,` 或 `}}`（位置 {}）", self.i)),
            }
        }
    }
}
