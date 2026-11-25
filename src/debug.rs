//! 调试器：命令行交互调试 + DAP（Debug Adapter Protocol）适配器。
//!
//! 两者共用同一个执行核心（`interp`），断点直接落在 `.jrust` 源码行上。
//! 命令行调试用于终端体验；DAP 让 VS Code / IntelliJ 这类 IDE 接进来。

use std::collections::BTreeSet;
use std::io::{self, BufRead, Read, Write};

use crate::interp::Snapshot;
use crate::json::{n, obj, s, Json};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 命令行调试器启动后等待用户输入（`run` 才开始）。
    Wait,
    Continue,
    StepIn,
    Next(usize),
    Out(usize),
}

pub enum DebugResult {
    Resume,
    Quit,
}

pub enum Debugger {
    Cli(CliDebugger),
    Dap(DapState),
}

impl Debugger {
    pub fn cli(path: &str, src: Vec<String>) -> Debugger {
        Debugger::Cli(CliDebugger {
            path: path.to_string(),
            src,
            bp: BTreeSet::new(),
            mode: Mode::Wait,
            last: None,
        })
    }

    pub fn on_line(&mut self, line: usize, snap: &Snapshot) -> DebugResult {
        match self {
            Debugger::Cli(d) => d.on_line(line, snap),
            Debugger::Dap(d) => d.on_line(line, snap),
        }
    }

    pub fn on_output(&mut self, text: &str) {
        match self {
            Debugger::Cli(_) => println!("{}", text),
            Debugger::Dap(d) => d.output(text, true),
        }
    }

    pub fn on_output_raw(&mut self, text: &str) {
        match self {
            Debugger::Cli(_) => {
                print!("{}", text);
                let _ = io::stdout().flush();
            }
            Debugger::Dap(d) => d.output(text, false),
        }
    }

    pub fn on_finish(&mut self, code: i32) {
        match self {
            Debugger::Cli(_) => {
                println!("== 程序结束（退出码 {}）==", code);
            }
            Debugger::Dap(d) => d.finish(code),
        }
    }
}

// ---------------- 命令行调试器 ----------------

pub struct CliDebugger {
    path: String,
    src: Vec<String>,
    bp: BTreeSet<usize>,
    mode: Mode,
    last: Option<String>,
}

impl CliDebugger {
    fn on_line(&mut self, line: usize, snap: &Snapshot) -> DebugResult {
        let depth = snap.frames.len();
        let stop = match self.mode {
            Mode::Wait => true,
            Mode::Continue => self.bp.contains(&line),
            Mode::StepIn => true,
            Mode::Next(d) => depth <= d,
            Mode::Out(d) => depth < d,
        };
        if !stop {
            return DebugResult::Resume;
        }
        let reason = if self.mode == Mode::Continue && self.bp.contains(&line) {
            "断点"
        } else {
            "单步"
        };
        self.show_stop(line, snap, reason);
        self.command_loop(line, depth, snap)
    }

    fn show_stop(&self, line: usize, snap: &Snapshot, reason: &str) {
        let frame = snap.frames.last().map(|f| f.name.as_str()).unwrap_or("?");
        println!("== 停在 {}:{}（{}，{}）==", self.path, line, frame, reason);
        if let Some(text) = self.src.get(line.saturating_sub(1)) {
            println!("{:>5} | {}", line, text);
        }
        self.print_vars(snap);
    }

    fn print_vars(&self, snap: &Snapshot) {
        let vars = match snap.frames.last() {
            Some(f) => &f.vars,
            None => return,
        };
        if vars.is_empty() {
            println!("  变量: （无）");
            return;
        }
        println!("  变量:");
        for v in vars.iter().take(20) {
            println!("    {} = {}  [{}]", v.name, v.value, v.ty);
        }
    }

    fn command_loop(&mut self, line: usize, depth: usize, snap: &Snapshot) -> DebugResult {
        loop {
            print!("(jrust-dbg) ");
            let _ = io::stdout().flush();
            let mut input = String::new();
            let stdin = io::stdin();
            if stdin.lock().read_line(&mut input).unwrap_or(0) == 0 {
                return DebugResult::Quit;
            }
            let text = input.trim().to_string();
            if text.is_empty() {
                if let Some(last) = self.last.clone() {
                    if let Some(r) = self.run_command(&last, line, depth, snap) {
                        return r;
                    }
                }
                continue;
            }
            self.last = Some(text.clone());
            if let Some(r) = self.run_command(&text, line, depth, snap) {
                return r;
            }
        }
    }

    /// 返回 Some 表示离开命令循环（继续执行或退出）。
    fn run_command(
        &mut self,
        text: &str,
        line: usize,
        depth: usize,
        snap: &Snapshot,
    ) -> Option<DebugResult> {
        let mut parts = text.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("").trim();
        match cmd {
            "b" | "break" => match arg.parse::<usize>() {
                Ok(n) if n > 0 => {
                    self.bp.insert(n);
                    println!("已在第 {} 行设置断点", n);
                }
                _ => println!("用法: break <行号>"),
            },
            "d" | "delete" => match arg.parse::<usize>() {
                Ok(n) => {
                    if self.bp.remove(&n) {
                        println!("已删除第 {} 行断点", n);
                    } else {
                        println!("第 {} 行没有断点", n);
                    }
                }
                _ => println!("用法: delete <行号>"),
            },
            "run" | "r" | "continue" | "c" => {
                self.mode = Mode::Continue;
                return Some(DebugResult::Resume);
            }
            "next" | "n" => {
                self.mode = Mode::Next(depth);
                return Some(DebugResult::Resume);
            }
            "step" | "s" => {
                self.mode = Mode::StepIn;
                return Some(DebugResult::Resume);
            }
            "out" | "o" => {
                self.mode = Mode::Out(depth);
                return Some(DebugResult::Resume);
            }
            "print" | "p" => {
                if arg.is_empty() {
                    println!("用法: print <变量名>，例如 print d.label");
                } else if let Some(v) = lookup_var(snap, arg) {
                    println!("{} = {}  [{}]", arg, v.value, v.ty);
                } else {
                    println!("当前作用域里看不到 `{}`", arg);
                }
            }
            "where" | "w" | "bt" => {
                if snap.frames.is_empty() {
                    println!("（没有栈帧）");
                }
                for (i, f) in snap.frames.iter().rev().enumerate() {
                    println!("  #{} {} at {}:{}", i, f.name, self.path, f.line);
                }
            }
            "list" | "l" => {
                let center = arg.parse::<usize>().unwrap_or(line);
                let start = center.saturating_sub(3).max(1);
                let end = (center + 3).min(self.src.len());
                for i in start..=end {
                    let mark = if i == line { "->" } else { "  " };
                    let text = self.src.get(i - 1).map(|s| s.as_str()).unwrap_or("");
                    println!("{} {:>5} | {}", mark, i, text);
                }
            }
            "quit" | "q" | "exit" => return Some(DebugResult::Quit),
            "help" | "h" => print_help(),
            _ => {
                println!("未知命令 `{}`；输入 help 查看命令列表", cmd);
            }
        }
        None
    }
}

fn lookup_var<'a>(snap: &'a Snapshot, name: &str) -> Option<&'a crate::interp::VarView> {
    for f in snap.frames.iter().rev() {
        if let Some(v) = f.vars.iter().find(|v| v.name == name) {
            return Some(v);
        }
    }
    None
}

fn print_help() {
    println!("命令:");
    println!("  break <行号> / b    设置断点");
    println!("  delete <行号> / d    删除断点");
    println!("  run / r / continue   继续执行到下一个断点");
    println!("  next / n             单步（不进入方法）");
    println!("  step / s             单步（进入方法）");
    println!("  out / o              执行到当前方法返回");
    println!("  print <名字> / p     查看变量，如 print d.label");
    println!("  where / w            查看调用栈");
    println!("  list [行号] / l      查看源码上下文");
    println!("  quit / q             退出调试");
}

// ---------------- DAP 适配器 ----------------

pub struct DapState {
    seq: i64,
    path: String,
    breakpoints: BTreeSet<usize>,
    mode: Mode,
    finished: bool,
}

impl DapState {
    pub fn new() -> Self {
        DapState {
            seq: 0,
            path: String::new(),
            breakpoints: BTreeSet::new(),
            mode: Mode::Continue,
            finished: false,
        }
    }

    /// 处理 initialize / launch / setBreakpoints，直到 configurationDone。
    /// 返回要执行的 `.jrust` 路径。
    pub fn handshake(&mut self) -> Result<String, String> {
        loop {
            let raw = match read_raw() {
                Some(r) => r,
                None => return Err("DAP 输入结束（客户端已关闭）".to_string()),
            };
            let req = match Json::parse(&raw) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("DAP 请求 JSON 解析失败: {}", e);
                    continue;
                }
            };
            let command = req
                .get("command")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            match command.as_str() {
                "initialize" => {
                    self.respond(
                        &req,
                        Some(obj(vec![(
                            "supportsConfigurationDoneRequest",
                            Json::Bool(true),
                        )])),
                    );
                    self.event("initialized", obj(vec![]));
                }
                "launch" | "attach" => {
                    if let Some(p) = req
                        .get("arguments")
                        .and_then(|a| a.get("program"))
                        .and_then(|v| v.as_str())
                    {
                        self.path = p.to_string();
                    }
                    self.respond(&req, None);
                }
                "setBreakpoints" => {
                    let lines = self.apply_breakpoints(&req);
                    self.respond(&req, Some(self.breakpoints_body(&lines)));
                }
                "configurationDone" => {
                    self.respond(&req, None);
                    if self.path.is_empty() {
                        return Err("launch 请求没有提供 program 路径".to_string());
                    }
                    return Ok(self.path.clone());
                }
                "disconnect" | "terminate" => {
                    self.respond(&req, None);
                    return Err("客户端已断开".to_string());
                }
                _ => self.respond(&req, None),
            }
        }
    }

    fn apply_breakpoints(&mut self, req: &Json) -> Vec<i64> {
        self.breakpoints.clear();
        let mut lines = Vec::new();
        if let Some(arr) = req
            .get("arguments")
            .and_then(|a| a.get("breakpoints"))
            .and_then(|v| v.as_arr())
        {
            for bp in arr {
                if let Some(l) = bp.get("line").and_then(|v| v.as_i64()) {
                    if l > 0 {
                        self.breakpoints.insert(l as usize);
                        lines.push(l);
                    }
                }
            }
        }
        lines
    }

    fn breakpoints_body(&self, lines: &[i64]) -> Json {
        let breakpoints: Vec<Json> = lines
            .iter()
            .map(|l| {
                obj(vec![
                    ("verified", Json::Bool(true)),
                    ("line", n(*l)),
                    ("source", obj(vec![("path", s(self.path.clone()))])),
                ])
            })
            .collect();
        obj(vec![("breakpoints", Json::Arr(breakpoints))])
    }

    fn on_line(&mut self, line: usize, snap: &Snapshot) -> DebugResult {
        let depth = snap.frames.len();
        let stop = match self.mode {
            Mode::Wait => true,
            Mode::Continue => self.breakpoints.contains(&line),
            Mode::StepIn => true,
            Mode::Next(d) => depth <= d,
            Mode::Out(d) => depth < d,
        };
        if !stop {
            return DebugResult::Resume;
        }
        let reason = if self.mode == Mode::Continue && self.breakpoints.contains(&line) {
            "breakpoint"
        } else {
            "step"
        };
        self.event(
            "stopped",
            obj(vec![
                ("reason", s(reason)),
                ("threadId", n(1)),
                ("allThreadsStopped", Json::Bool(true)),
            ]),
        );

        loop {
            let raw = match read_raw() {
                Some(r) => r,
                None => return DebugResult::Quit,
            };
            let req = match Json::parse(&raw) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("DAP 请求 JSON 解析失败: {}", e);
                    continue;
                }
            };
            let command = req
                .get("command")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            match command.as_str() {
                "continue" => {
                    self.respond(
                        &req,
                        Some(obj(vec![("allThreadsContinued", Json::Bool(true))])),
                    );
                    self.mode = Mode::Continue;
                    return DebugResult::Resume;
                }
                "next" => {
                    self.respond(&req, None);
                    self.mode = Mode::Next(depth);
                    return DebugResult::Resume;
                }
                "stepIn" => {
                    self.respond(&req, None);
                    self.mode = Mode::StepIn;
                    return DebugResult::Resume;
                }
                "stepOut" => {
                    self.respond(&req, None);
                    self.mode = Mode::Out(depth);
                    return DebugResult::Resume;
                }
                "threads" => self.respond(
                    &req,
                    Some(obj(vec![(
                        "threads",
                        Json::Arr(vec![obj(vec![
                            ("id", n(1)),
                            ("name", s("main")),
                        ])]),
                    )])),
                ),
                "stackTrace" => self.respond(&req, Some(self.stack_body(snap))),
                "scopes" => {
                    let frame = req
                        .get("arguments")
                        .and_then(|a| a.get("frameId"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let frame = frame.clamp(0, snap.frames.len().saturating_sub(1) as i64);
                    self.respond(
                        &req,
                        Some(obj(vec![(
                            "scopes",
                            Json::Arr(vec![obj(vec![
                                ("name", s("Locals")),
                                ("variablesReference", n(1000 + frame)),
                                ("expensive", Json::Bool(false)),
                            ])]),
                        )])),
                    );
                }
                "variables" => {
                    let reference = req
                        .get("arguments")
                        .and_then(|a| a.get("variablesReference"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    self.respond(&req, Some(self.variables_body(snap, reference)));
                }
                "evaluate" => {
                    let expr = req
                        .get("arguments")
                        .and_then(|a| a.get("expression"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    match lookup_var(snap, expr) {
                        Some(v) => self.respond(
                            &req,
                            Some(obj(vec![
                                ("result", s(v.value.clone())),
                                ("type", s(v.ty.clone())),
                                ("variablesReference", n(0)),
                            ])),
                        ),
                        None => self.respond_err(&req, &format!("看不到 `{}`", expr)),
                    }
                }
                "setBreakpoints" => {
                    let lines = self.apply_breakpoints(&req);
                    self.respond(&req, Some(self.breakpoints_body(&lines)));
                }
                "pause" => self.respond(&req, None),
                "disconnect" | "terminate" => {
                    self.respond(&req, None);
                    return DebugResult::Quit;
                }
                _ => self.respond(&req, None),
            }
        }
    }

    fn stack_body(&self, snap: &Snapshot) -> Json {
        let mut frames = Vec::new();
        for (id, f) in snap.frames.iter().enumerate().rev() {
            frames.push(obj(vec![
                ("id", n(id as i64)),
                ("name", s(f.name.clone())),
                (
                    "source",
                    obj(vec![
                        ("name", s(source_name(&self.path))),
                        ("path", s(self.path.clone())),
                    ]),
                ),
                ("line", n(f.line as i64)),
                ("column", n(1)),
            ]));
        }
        obj(vec![
            ("stackFrames", Json::Arr(frames)),
            ("totalFrames", n(snap.frames.len() as i64)),
        ])
    }

    fn variables_body(&self, snap: &Snapshot, reference: i64) -> Json {
        let index = if reference >= 1000 {
            (reference - 1000) as usize
        } else {
            0
        };
        let vars = snap.frames.get(index).map(|f| &f.vars);
        let items: Vec<Json> = match vars {
            Some(vars) => vars
                .iter()
                .map(|v| {
                    obj(vec![
                        ("name", s(v.name.clone())),
                        ("value", s(v.value.clone())),
                        ("type", s(v.ty.clone())),
                        ("variablesReference", n(0)),
                    ])
                })
                .collect(),
            None => Vec::new(),
        };
        obj(vec![("variables", Json::Arr(items))])
    }

    fn output(&mut self, text: &str, newline: bool) {
        if self.finished {
            return;
        }
        let mut text = text.to_string();
        if newline {
            text.push('\n');
        }
        self.event(
            "output",
            obj(vec![
                ("category", s("stdout")),
                ("output", s(text)),
            ]),
        );
    }

    pub fn finish(&mut self, code: i32) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.event(
            "exited",
            obj(vec![("exitCode", n(code as i64))]),
        );
        self.event("terminated", obj(vec![]));
    }

    fn respond(&mut self, req: &Json, body: Option<Json>) {
        let command = req
            .get("command")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        let request_seq = req.get("seq").and_then(|v| v.as_i64()).unwrap_or(0);
        let mut items: Vec<(&str, Json)> = vec![
            ("type", s("response")),
            ("request_seq", n(request_seq)),
            ("success", Json::Bool(true)),
            ("command", s(command)),
        ];
        if let Some(b) = body {
            items.push(("body", b));
        }
        self.send(obj(items));
    }

    fn respond_err(&mut self, req: &Json, message: &str) {
        let command = req
            .get("command")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        let request_seq = req.get("seq").and_then(|v| v.as_i64()).unwrap_or(0);
        self.send(obj(vec![
            ("type", s("response")),
            ("request_seq", n(request_seq)),
            ("success", Json::Bool(false)),
            ("command", s(command)),
            ("message", s(message.to_string())),
        ]));
    }

    fn event(&mut self, name: &str, body: Json) {
        self.send(obj(vec![
            ("type", s("event")),
            ("event", s(name.to_string())),
            ("body", body),
        ]));
    }

    fn send(&mut self, message: Json) {
        self.seq += 1;
        let message = match message {
            Json::Obj(mut items) => {
                items.insert(0, ("seq".to_string(), n(self.seq)));
                Json::Obj(items)
            }
            other => other,
        };
        let text = message.dump();
        let out = io::stdout();
        let mut lock = out.lock();
        let _ = write!(lock, "Content-Length: {}\r\n\r\n{}", text.len(), text);
        let _ = lock.flush();
    }
}

fn source_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// 读取一条 DAP 消息：`Content-Length: N\r\n\r\n<body>`。
fn read_raw() -> Option<String> {
    let stdin = io::stdin();
    let mut lock = stdin.lock();
    let mut len: Option<usize> = None;
    loop {
        let mut line = String::new();
        if lock.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let text = line.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            break;
        }
        if let Some(v) = text.strip_prefix("Content-Length:") {
            len = v.trim().parse().ok();
        }
    }
    let n = len?;
    let mut buf = vec![0u8; n];
    lock.read_exact(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}
