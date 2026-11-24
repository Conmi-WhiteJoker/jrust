//! 诊断信息：jrust 的编译错误不只要说「不行」，还要说「Rust 为什么这么要求」。

#[derive(Debug, Clone)]
pub struct Diag {
    /// 源码行号（从 1 开始）
    pub line: usize,
    pub message: String,
    pub hint: Option<String>,
}

impl Diag {
    pub fn error(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// 渲染成终端可读的两行格式。
    pub fn render(&self, path: &str) -> String {
        let mut s = format!("错误: {}:{} - {}", path, self.line, self.message);
        if let Some(h) = &self.hint {
            s.push_str(&format!("\n  提示: {}", h));
        }
        s
    }
}
