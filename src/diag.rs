//! 诊断信息：jrust 的编译错误不只要说「不行」，还要说「Rust 为什么这么要求」。

/// 诊断级别：`Error` 阻断生成，`Warning` 只提示（如 `JR4101`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diag {
    pub level: Level,
    /// 源码行号（从 1 开始）
    pub line: usize,
    /// 规范诊断码（如 `JR3018`）；旧诊断可以为空。
    pub code: Option<&'static str>,
    pub message: String,
    pub hint: Option<String>,
}

impl Diag {
    pub fn error(line: usize, message: impl Into<String>) -> Self {
        Self {
            level: Level::Error,
            line,
            code: None,
            message: message.into(),
            hint: None,
        }
    }

    pub fn error_code(line: usize, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            level: Level::Error,
            line,
            code: Some(code),
            message: message.into(),
            hint: None,
        }
    }

    /// warning 级诊断：带规范码，不阻断生成。
    pub fn warning_code(line: usize, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            level: Level::Warning,
            line,
            code: Some(code),
            message: message.into(),
            hint: None,
        }
    }

    pub fn is_warning(&self) -> bool {
        self.level == Level::Warning
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// 渲染成终端可读的两行格式。
    pub fn render(&self, path: &str) -> String {
        let (label, tag) = match self.level {
            Level::Error => ("错误", "error"),
            Level::Warning => ("警告", "warning"),
        };
        let mut s = match self.code {
            Some(code) => format!(
                "{}: {}:{} - {}[{}]: {}",
                label, path, self.line, tag, code, self.message
            ),
            None => format!("{}: {}:{} - {}", label, path, self.line, self.message),
        };
        if let Some(h) = &self.hint {
            s.push_str(&format!("\n  提示: {}", h));
        }
        s
    }
}
