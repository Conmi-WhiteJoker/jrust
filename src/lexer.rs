//! 手写词法分析器：把 UTF-8 源码切成 token 流。
//!
//! 刻意不引入 pest 之类的解析框架——词法/语法分析本身也是这个项目的教学素材。

use crate::diag::Diag;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),
    CharLit(char),
    Kw(Kw),
    Punct(&'static str),
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kw {
    Class,
    Interface,
    Implements,
    Extends,
    Public,
    Private,
    Protected,
    Static,
    Final,
    Void,
    Return,
    If,
    Else,
    While,
    For,
    New,
    This,
    /// jrust 自有扩展关键字：显式移交所有权
    Move,
    True,
    False,
    Null,
    Import,
    Package,
    Int,
    Long,
    Double,
    Float,
    Boolean,
    Char,
    Break,
    Continue,
    Synchronized,
    Try,
    Catch,
    Finally,
    Throw,
    Throws,
    Instanceof,
}

pub fn keyword(s: &str) -> Option<Kw> {
    Some(match s {
        "class" => Kw::Class,
        "interface" => Kw::Interface,
        "implements" => Kw::Implements,
        "extends" => Kw::Extends,
        "public" => Kw::Public,
        "private" => Kw::Private,
        "protected" => Kw::Protected,
        "static" => Kw::Static,
        "final" => Kw::Final,
        "void" => Kw::Void,
        "return" => Kw::Return,
        "if" => Kw::If,
        "else" => Kw::Else,
        "while" => Kw::While,
        "for" => Kw::For,
        "new" => Kw::New,
        "this" => Kw::This,
        "move" => Kw::Move,
        "true" => Kw::True,
        "false" => Kw::False,
        "null" => Kw::Null,
        "import" => Kw::Import,
        "package" => Kw::Package,
        "int" => Kw::Int,
        "long" => Kw::Long,
        "double" => Kw::Double,
        "float" => Kw::Float,
        "boolean" => Kw::Boolean,
        "char" => Kw::Char,
        "break" => Kw::Break,
        "continue" => Kw::Continue,
        "synchronized" => Kw::Synchronized,
        "try" => Kw::Try,
        "catch" => Kw::Catch,
        "finally" => Kw::Finally,
        "throw" => Kw::Throw,
        "throws" => Kw::Throws,
        "instanceof" => Kw::Instanceof,
        _ => return None,
    })
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
}

/// 多字符符号必须排在单字符前面，否则 `==` 会被切成两个 `=`。
const PUNCTS: &[&str] = &[
    "->", "==", "!=", "<=", ">=", "&&", "||", "++", "--", "+=", "-=", "*=", "/=", "%=", "{", "}",
    "(", ")", "[", "]", ";", ",", ".", "=", "<", ">", "+", "-", "*", "/", "%", "!", ":", "?", "@",
    "&", "|",
];

pub fn lex(src: &str) -> Result<Vec<Token>, Vec<Diag>> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut toks: Vec<Token> = Vec::new();
    let mut diags: Vec<Diag> = Vec::new();

    while i < chars.len() {
        let c = chars[i];

        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        // 行注释
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // 块注释
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let start_line = line;
            i += 2;
            let mut closed = false;
            while i < chars.len() {
                if chars[i] == '\n' {
                    line += 1;
                }
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    i += 2;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                diags.push(Diag::error(start_line, "块注释未闭合（缺少 `*/`）"));
            }
            continue;
        }

        // 字符串字面量
        if c == '"' {
            let start_line = line;
            i += 1;
            let mut s = String::new();
            let mut closed = false;
            while i < chars.len() {
                let ch = chars[i];
                if ch == '"' {
                    i += 1;
                    closed = true;
                    break;
                }
                if ch == '\n' {
                    break;
                }
                if ch == '\\' {
                    i += 1;
                    if let Some(e) = chars.get(i).copied() {
                        match e {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            'r' => s.push('\r'),
                            '"' => s.push('"'),
                            '\\' => s.push('\\'),
                            '\'' => s.push('\''),
                            other => {
                                s.push('\\');
                                s.push(other);
                            }
                        }
                        i += 1;
                    }
                    continue;
                }
                s.push(ch);
                i += 1;
            }
            if !closed {
                diags.push(Diag::error(start_line, "字符串字面量未闭合（缺少 `\"`）"));
            }
            toks.push(Token {
                tok: Tok::Str(s),
                line: start_line,
            });
            continue;
        }

        // 字符字面量
        if c == '\'' {
            let start_line = line;
            i += 1;
            let mut value: Option<char> = None;
            if let Some(ch) = chars.get(i).copied() {
                if ch == '\\' {
                    i += 1;
                    if let Some(e) = chars.get(i).copied() {
                        value = Some(match e {
                            'n' => '\n',
                            't' => '\t',
                            'r' => '\r',
                            '\'' => '\'',
                            '\\' => '\\',
                            other => other,
                        });
                        i += 1;
                    }
                } else {
                    value = Some(ch);
                    i += 1;
                }
                if chars.get(i) == Some(&'\'') {
                    i += 1;
                } else {
                    diags.push(Diag::error(start_line, "字符字面量未闭合（缺少 `'`）"));
                }
            } else {
                diags.push(Diag::error(start_line, "字符字面量不完整"));
            }
            toks.push(Token {
                tok: Tok::CharLit(value.unwrap_or('\0')),
                line: start_line,
            });
            continue;
        }

        // 数字
        if c.is_ascii_digit() {
            let start_line = line;
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let mut is_float = false;
            if i < chars.len() && chars[i] == '.' && chars.get(i + 1).map_or(false, |d| d.is_ascii_digit())
            {
                is_float = true;
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = chars[start..i].iter().collect();
            if is_float {
                match text.parse::<f64>() {
                    Ok(v) => toks.push(Token {
                        tok: Tok::Float(v),
                        line: start_line,
                    }),
                    Err(_) => diags.push(Diag::error(start_line, format!("无法解析浮点数 `{}`", text))),
                }
            } else {
                match text.parse::<i64>() {
                    Ok(v) => toks.push(Token {
                        tok: Tok::Int(v),
                        line: start_line,
                    }),
                    Err(_) => diags.push(Diag::error(start_line, format!("整数超出范围 `{}`", text))),
                }
            }
            continue;
        }

        // 标识符 / 关键字
        if c.is_alphabetic() || c == '_' || c == '$' {
            let start_line = line;
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$')
            {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let tok = match keyword(&text) {
                Some(k) => Tok::Kw(k),
                None => Tok::Ident(text),
            };
            toks.push(Token {
                tok,
                line: start_line,
            });
            continue;
        }

        // 符号
        let mut matched = false;
        for p in PUNCTS {
            let pc: Vec<char> = p.chars().collect();
            if chars[i..].starts_with(&pc[..]) {
                toks.push(Token {
                    tok: Tok::Punct(p),
                    line,
                });
                i += pc.len();
                matched = true;
                break;
            }
        }
        if matched {
            continue;
        }

        diags.push(Diag::error(
            line,
            format!("无法识别的字符 `{}`", c),
        ));
        i += 1;
    }

    toks.push(Token {
        tok: Tok::Eof,
        line,
    });

    if diags.is_empty() {
        Ok(toks)
    } else {
        Err(diags)
    }
}
