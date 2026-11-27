//! 树遍历解释器：直接执行 AST，不经过 rustc。
//!
//! 这是 IDE 调试路线（方案 C）的核心：断点、单步、变量查看都作用在 `.jrust`
//! 源码行上，不需要任何行号映射。
//!
//! 解释器刻意保留 Rust 的「移动语义」：给 `String` / 对象 / `List` 变量赋值、
//! 传参、放进集合，都会把原变量标记为「已移动」，之后再读就报运行时错误。
//! 这样 `examples/Ownership.jrust` 这类教学示例在解释执行时也能体现所有权。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;
use crate::debug::{DebugResult, Debugger};

/// 单次执行允许的最大语句数，用来兜住死循环。
const MAX_STEPS: u64 = 5_000_000;

#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub line: usize,
    pub message: String,
    pub hint: Option<String>,
    /// 调试器里输入 `quit` 触发的正常中止：不打印错误。
    pub quit: bool,
}

impl RuntimeError {
    fn new(line: usize, message: impl Into<String>) -> Self {
        RuntimeError {
            line,
            message: message.into(),
            hint: None,
            quit: false,
        }
    }

    fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    fn quit() -> Self {
        RuntimeError {
            line: 0,
            message: String::new(),
            hint: None,
            quit: true,
        }
    }

    pub fn render(&self, path: &str) -> String {
        // 与 JR-G7 统一：`path:line:0: error: 消息`（运行时错误没有诊断码，故不带方括号）。
        let mut s = format!("{}:{}:0: error: {}", path, self.line, self.message);
        if let Some(h) = &self.hint {
            s.push_str(&format!("\n提示: {}", h));
        }
        s
    }
}

#[derive(Clone)]
enum Value {
    Int(i64),
    Double(f64),
    Bool(bool),
    Char(char),
    Str(String),
    List(Rc<RefCell<Vec<Value>>>),
    /// `Set<T>` / `HashSet<T>`：按插入顺序保存并去重（Rust 侧是 `HashSet`，顺序不承诺）。
    Set(Rc<RefCell<Vec<Value>>>),
    Map(Rc<RefCell<HashMap<String, Value>>>),
    Obj(Rc<RefCell<Instance>>),
    /// 带数据枚举的值：`Shape.Circle(2.0)` / `Shape.Empty`。
    Enum {
        enum_name: String,
        variant: String,
        /// 分支数据，按 enum 声明顺序。
        fields: Vec<Value>,
    },
    /// `T?`：`Some(v)` / `None`，与 Rust 的 `Option<T>` 对齐。
    Option(Option<Box<Value>>),
    Unit,
}

struct Instance {
    class: String,
    fields: Vec<(String, Slot)>,
}

#[derive(Clone)]
struct Slot {
    value: Value,
    /// Rust 的所有权只能转移一次：搬走之后原位置不可再用。
    moved: bool,
    ty: Type,
}

/// 按值语义复制一份：对象、列表、Map 递归复制，与 Rust 侧 `Clone` 对齐。
fn deep_clone(v: &Value) -> Value {
    match v {
        Value::List(rc) => Value::List(Rc::new(RefCell::new(
            rc.borrow().iter().map(deep_clone).collect(),
        ))),
        Value::Set(rc) => Value::Set(Rc::new(RefCell::new(
            rc.borrow().iter().map(deep_clone).collect(),
        ))),
        Value::Enum {
            enum_name,
            variant,
            fields,
        } => Value::Enum {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            fields: fields.iter().map(deep_clone).collect(),
        },
        Value::Map(rc) => Value::Map(Rc::new(RefCell::new(
            rc.borrow()
                .iter()
                .map(|(k, v)| (k.clone(), deep_clone(v)))
                .collect(),
        ))),
        Value::Option(inner) => Value::Option(inner.as_ref().map(|v| Box::new(deep_clone(v)))),
        Value::Obj(rc) => {
            let inst = rc.borrow();
            let fields = inst
                .fields
                .iter()
                .map(|(n, s)| {
                    (
                        n.clone(),
                        Slot {
                            value: deep_clone(&s.value),
                            moved: false,
                            ty: s.ty.clone(),
                        },
                    )
                })
                .collect();
            Value::Obj(Rc::new(RefCell::new(Instance {
                class: inst.class.clone(),
                fields,
            })))
        }
        other => other.clone(),
    }
}

/// 闭包参数省略类型标注时，用实参的动态类型补一个（只用于调试面板与赋值的可空包装）。
fn dynamic_type(v: &Value) -> Type {
    match v {
        Value::Int(_) => Type::Int,
        Value::Double(_) => Type::Double,
        Value::Bool(_) => Type::Boolean,
        Value::Char(_) => Type::Char,
        Value::Str(_) => Type::String,
        Value::List(_) => Type::List(Box::new(Type::Int)),
        Value::Set(_) => Type::Set(Box::new(Type::Int)),
        Value::Map(_) => Type::Map(Box::new(Type::String), Box::new(Type::Int)),
        Value::Obj(obj) => Type::Named(obj.borrow().class.clone()),
        Value::Enum { enum_name, .. } => Type::Named(enum_name.clone()),
        Value::Option(_) => Type::Nullable(Box::new(Type::Int)),
        Value::Unit => Type::Void,
    }
}

impl Value {
    /// Copy 类型直接复制；其余类型在普通赋值时走 `deep_clone`，显式 `move` 才转移所有权。
    fn is_copy(&self) -> bool {
        matches!(
            self,
            Value::Int(_) | Value::Double(_) | Value::Bool(_) | Value::Char(_) | Value::Unit
        )
    }

    /// `println` / 字符串拼接时使用的显示形式（与 Rust 的 Display 对齐）。
    fn display(&self) -> String {
        match self {
            Value::Int(n) => n.to_string(),
            Value::Double(f) => format!("{}", f),
            Value::Bool(b) => b.to_string(),
            Value::Char(c) => c.to_string(),
            Value::Str(s) => s.clone(),
            Value::List(_) | Value::Set(_) | Value::Obj(_) | Value::Map(_) | Value::Option(_) => {
                self.debug_repr()
            }
            Value::Enum { .. } => self.debug_repr(),
            Value::Unit => "()".to_string(),
        }
    }

    /// 调试器面板里的显示形式（字符串带引号，浮点保留 `.0`）。
    fn debug_repr(&self) -> String {
        match self {
            Value::Int(n) => n.to_string(),
            Value::Double(f) => format!("{:?}", f),
            Value::Bool(b) => b.to_string(),
            Value::Char(c) => format!("{:?}", c),
            Value::Str(s) => format!("{:?}", s),
            Value::Unit => "()".to_string(),
            Value::List(items) => {
                let items = items.borrow();
                let inner: Vec<String> = items.iter().map(|v| v.debug_repr()).collect();
                format!("[{}]", inner.join(", "))
            }
            Value::Set(items) => {
                let items = items.borrow();
                let inner: Vec<String> = items.iter().map(|v| v.debug_repr()).collect();
                format!("{{{}}}", inner.join(", "))
            }
            Value::Map(entries) => {
                let entries = entries.borrow();
                let mut pairs: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| {
                        let shown = k.get(2..).unwrap_or(k);
                        format!("{}: {}", shown, v.debug_repr())
                    })
                    .collect();
                pairs.sort();
                format!("{{{}}}", pairs.join(", "))
            }
            Value::Obj(obj) => {
                let inst = obj.borrow();
                if inst.fields.is_empty() {
                    format!("{}", inst.class)
                } else {
                    let inner: Vec<String> = inst
                        .fields
                        .iter()
                        .map(|(name, slot)| {
                            if slot.moved {
                                format!("{}: <已 move>", name)
                            } else {
                                format!("{}: {}", name, slot.value.debug_repr())
                            }
                        })
                        .collect();
                    format!("{} {{ {} }}", inst.class, inner.join(", "))
                }
            }
            Value::Option(Some(v)) => format!("Some({})", v.debug_repr()),
            Value::Option(None) => "None".to_string(),
            Value::Enum {
                variant, fields, ..
            } => {
                if fields.is_empty() {
                    variant.clone()
                } else {
                    let inner: Vec<String> = fields.iter().map(|v| v.debug_repr()).collect();
                    format!("{}({})", variant, inner.join(", "))
                }
            }
        }
    }

    fn type_name(&self) -> String {
        match self {
            Value::Int(_) => "int",
            Value::Double(_) => "double",
            Value::Bool(_) => "boolean",
            Value::Char(_) => "char",
            Value::Str(_) => "String",
            Value::List(_) => "List",
            Value::Set(_) => "Set",
            Value::Map(_) => "Map",
            Value::Obj(obj) => return obj.borrow().class.clone(),
            Value::Enum { enum_name, .. } => enum_name.as_str(),
            Value::Option(_) => "Option",
            Value::Unit => "void",
        }
        .to_string()
    }
}

fn type_str(t: &Type) -> String {
    match t {
        Type::Int => "int".into(),
        Type::Long => "long".into(),
        Type::Double => "double".into(),
        Type::Float => "float".into(),
        Type::Boolean => "boolean".into(),
        Type::Char => "char".into(),
        Type::Void => "void".into(),
        Type::String => "String".into(),
        Type::RustPrim(name) => name.clone(),
        Type::List(inner) => format!("List<{}>", type_str(inner)),
        Type::Map(k, v) => format!("Map<{}, {}>", type_str(k), type_str(v)),
        Type::Set(inner) => format!("Set<{}>", type_str(inner)),
        Type::Array(inner) => format!("{}[]", type_str(inner)),
        Type::Named(name) => name.clone(),
        Type::Generic(name, args) => {
            let inner: Vec<String> = args.iter().map(type_str).collect();
            format!("{}<{}>", name, inner.join(", "))
        }
        Type::Nullable(inner) => format!("{}?", type_str(inner)),
    }
}

fn default_value(t: &Type) -> Value {
    match t {
        Type::Int | Type::Long => Value::Int(0),
        Type::RustPrim(name) if name == "bool" => Value::Bool(false),
        Type::RustPrim(name) if name.starts_with('f') => Value::Double(0.0),
        Type::RustPrim(_) => Value::Int(0),
        Type::Double | Type::Float => Value::Double(0.0),
        Type::Boolean => Value::Bool(false),
        Type::Char => Value::Char('\0'),
        Type::String => Value::Str(String::new()),
        Type::List(_) | Type::Array(_) => Value::List(Rc::new(RefCell::new(Vec::new()))),
        Type::Map(_, _) => Value::Map(Rc::new(RefCell::new(HashMap::new()))),
        Type::Set(_) => Value::Set(Rc::new(RefCell::new(Vec::new()))),
        Type::Named(_) | Type::Generic(_, _) | Type::Void => Value::Unit,
        Type::Nullable(_) => Value::Option(None),
    }
}

enum Flow {
    Normal,
    Return(Value),
    Break,
    Continue,
}

struct Frame {
    class: String,
    method: String,
    line: usize,
    scopes: Vec<Vec<(String, Slot)>>,
    this: Option<Value>,
    /// 返回类型：`return e;` 据此做可空包装 / 解包。
    ret_ty: Type,
}

/// 调试器看到的栈帧快照（变量已拍平，`d.label` 这类路径可直接查看）。
pub struct Snapshot {
    /// 外层在前、内层在后。
    pub frames: Vec<FrameView>,
}

pub struct FrameView {
    pub name: String,
    pub line: usize,
    pub vars: Vec<VarView>,
}

pub struct VarView {
    pub name: String,
    pub value: String,
    pub ty: String,
}

pub struct Interp<'a> {
    prog: &'a Program,
    classes: HashMap<&'a str, &'a ClassDecl>,
    interfaces: HashMap<&'a str, &'a InterfaceDecl>,
    enums: HashMap<&'a str, &'a EnumDecl>,
    frames: Vec<Frame>,
    pub debug: Option<Debugger>,
    steps: u64,
    cur_line: usize,
}

impl<'a> Interp<'a> {
    pub fn new(prog: &'a Program, _path: &str) -> Self {
        let mut classes = HashMap::new();
        for c in &prog.classes {
            classes.insert(c.name.as_str(), c);
        }
        let mut enums = HashMap::new();
        for e in &prog.enums {
            enums.insert(e.name.as_str(), e);
        }
        let mut interfaces = HashMap::new();
        for i in &prog.interfaces {
            interfaces.insert(i.name.as_str(), i);
        }
        Interp {
            prog,
            classes,
            interfaces,
            enums,
            frames: Vec::new(),
            debug: None,
            steps: 0,
            cur_line: 1,
        }
    }

    pub fn take_debug(&mut self) -> Option<Debugger> {
        self.debug.take()
    }

    /// 从 `public static void main` 开始执行。
    pub fn run_main(&mut self) -> Result<(), RuntimeError> {
        let (class, main) = match self.find_main() {
            Some(v) => v,
            None => {
                return Err(RuntimeError::new(
                    1,
                    "找不到 `public static void main(String[] args)`",
                )
                .with_hint("解释执行需要入口；在某个类里写 `public static void main(String[] args)`。"))
            }
        };
        let mut scope = Vec::new();
        for p in &main.params {
            scope.push((
                p.name.clone(),
                Slot {
                    value: default_value(&p.ty),
                    moved: false,
                    ty: p.ty.clone(),
                },
            ));
        }
        self.frames.push(Frame {
            class: class.to_string(),
            method: "main".to_string(),
            line: main.line,
            scopes: vec![scope],
            this: None,
            ret_ty: main.ret.clone(),
        });
        let result = self.exec_block(&main.body);
        self.frames.pop();
        result.map(|_| ())
    }

    fn find_main(&self) -> Option<(&'a str, &'a Method)> {
        for c in &self.prog.classes {
            for m in &c.methods {
                if m.name == "main" && m.is_static {
                    return Some((c.name.as_str(), m));
                }
            }
        }
        None
    }

    // ---------- 行钩子 / 输出 ----------

    fn tick(&mut self, line: usize) -> Result<(), RuntimeError> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Err(RuntimeError::new(
                line,
                format!("执行步数超过上限（{} 步）", MAX_STEPS),
            )
            .with_hint("很可能是死循环；检查 while 条件或计数器是否推进。"));
        }
        if let Some(f) = self.frames.last_mut() {
            f.line = line;
        }
        self.cur_line = line;
        if self.debug.is_some() {
            let snap = self.snapshot();
            let mut dbg = self.debug.take().unwrap();
            let result = dbg.on_line(line, &snap);
            self.debug = Some(dbg);
            if matches!(result, DebugResult::Quit) {
                return Err(RuntimeError::quit());
            }
        }
        Ok(())
    }

    fn emit(&mut self, text: String) {
        if let Some(dbg) = self.debug.as_mut() {
            dbg.on_output(&text);
        } else {
            println!("{}", text);
        }
    }

    fn emit_no_newline(&mut self, text: String) {
        if let Some(dbg) = self.debug.as_mut() {
            dbg.on_output_raw(&text);
        } else {
            print!("{}", text);
        }
    }

    // ---------- 语句 ----------

    fn exec_block(&mut self, stmts: &[Stmt]) -> Result<Flow, RuntimeError> {
        if let Some(f) = self.frames.last_mut() {
            f.scopes.push(Vec::new());
        }
        let mut flow = Flow::Normal;
        for st in stmts {
            // while / foreach 在循环体每轮自己打点，避免同一行连续停两次。
            if !matches!(
                st,
                Stmt::While { .. } | Stmt::ForEach { .. } | Stmt::ForRange { .. }
            ) {
                if let Err(e) = self.tick(stmt_line(st)) {
                    self.pop_scope();
                    return Err(e);
                }
            }
            match self.exec_stmt(st) {
                Ok(Flow::Normal) => {}
                Ok(other) => {
                    flow = other;
                    break;
                }
                Err(e) => {
                    self.pop_scope();
                    return Err(e);
                }
            }
        }
        self.pop_scope();
        Ok(flow)
    }

    fn pop_scope(&mut self) {
        if let Some(f) = self.frames.last_mut() {
            f.scopes.pop();
        }
    }

    fn exec_stmt(&mut self, st: &Stmt) -> Result<Flow, RuntimeError> {
        match st {
            Stmt::Let {
                ty,
                name,
                init,
                line,
            } => {
                let v = self.eval_assign_source(init)?;
                let v = self.coerce_value(v, ty, *line)?;
                self.declare(name, ty.clone(), v);
                Ok(Flow::Normal)
            }
            Stmt::Assign {
                target,
                value,
                line,
            } => {
                let v = self.eval_assign_source(value)?;
                let v = match self.target_type(target)? {
                    Some(tt) => self.coerce_value(v, &tt, *line)?,
                    None => v,
                };
                self.assign(target, v)?;
                Ok(Flow::Normal)
            }
            Stmt::Expr { expr, .. } => {
                self.eval(expr)?;
                Ok(Flow::Normal)
            }
            Stmt::Return { value, line } => {
                let rty = self
                    .frames
                    .last()
                    .map(|f| f.ret_ty.clone())
                    .unwrap_or(Type::Void);
                let v = match value {
                    Some(e) => {
                        let v = self.eval(e)?;
                        self.coerce_value(v, &rty, *line)?
                    }
                    None => Value::Unit,
                };
                Ok(Flow::Return(v))
            }
            Stmt::If {
                cond, then, els, ..
            } => {
                let c = self.eval(cond)?;
                if self.as_bool(&c)? {
                    self.exec_block(then)
                } else if let Some(else_body) = els {
                    self.exec_block(else_body)
                } else {
                    Ok(Flow::Normal)
                }
            }
            Stmt::While { cond, body, line } => {
                loop {
                    self.tick(*line)?;
                    let c = self.eval(cond)?;
                    if !self.as_bool(&c)? {
                        break;
                    }
                    match self.exec_block(body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::ForEach {
                ty,
                name,
                iter,
                body,
                line,
            } => {
                let v = self.eval(iter)?;
                let items = match v {
                    Value::List(rc) => rc.borrow().clone(),
                    other => {
                        return Err(RuntimeError::new(
                            *line,
                            format!("for-each 需要集合，实际是 {}", other.type_name()),
                        )
                        .with_hint("先声明 `List<T> xs = new ArrayList<T>();` 再遍历。"))
                    }
                };
                for item in items {
                    self.tick(*line)?;
                    if let Some(f) = self.frames.last_mut() {
                        f.scopes.push(vec![(
                            name.clone(),
                            Slot {
                                value: item,
                                moved: false,
                                ty: ty.clone(),
                            },
                        )]);
                    }
                    let r = self.exec_block(body);
                    self.pop_scope();
                    match r? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::ForRange {
                ty,
                name,
                start,
                end,
                inclusive,
                descending,
                body,
                line,
            } => {
                let sv = self.eval(start)?;
                let s = self.int_of(sv, *line)?;
                let ev = self.eval(end)?;
                let e = self.int_of(ev, *line)?;
                let step: i64 = if *descending { -1 } else { 1 };
                let mut i = s;
                loop {
                    let go = if *descending {
                        if *inclusive {
                            i >= e
                        } else {
                            i > e
                        }
                    } else if *inclusive {
                        i <= e
                    } else {
                        i < e
                    };
                    if !go {
                        break;
                    }
                    self.tick(*line)?;
                    if let Some(f) = self.frames.last_mut() {
                        f.scopes.push(vec![(
                            name.clone(),
                            Slot {
                                value: Value::Int(i),
                                moved: false,
                                ty: ty.clone(),
                            },
                        )]);
                    }
                    let r = self.exec_block(body);
                    self.pop_scope();
                    match r? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                    i += step;
                }
                Ok(Flow::Normal)
            }
            Stmt::Break { .. } => Ok(Flow::Break),
            Stmt::Continue { .. } => Ok(Flow::Continue),
            Stmt::Switch {
                subject,
                cases,
                default,
                line,
            } => self.exec_switch(subject, cases, default, *line),
            // 解释器无并发：spawn 块同步执行一次（JR-C11）。
            Stmt::Spawn { body, .. } => {
                let _ = self.exec_block(body)?;
                Ok(Flow::Normal)
            }
            // 解释器无并发：synchronized 块直接顺序执行（JR-C12）。
            Stmt::Sync { body, .. } => {
                let f = self.exec_block(body)?;
                Ok(f)
            }
            Stmt::Throw { expr, line } => Err(self.throw_error(expr, *line)?),
            Stmt::Raw { line, .. } => Err(RuntimeError::new(
                *line,
                "解释器无法执行 raw 块",
            )
            .with_hint(
                "raw 是原样内联的 Rust 源码；用 `jrust build` 生成 Rust 后交给 cargo 运行。",
            )),
        }
    }

    // ---------- 异常 ----------

    /// `throw new E("msg")`：v1 没有 try / catch，异常只能一路向上传播（与 `?` 一致）。
    /// 因此把 throw 表示成运行时错误，由调用链自然带回 `main`。
    fn throw_error(&mut self, expr: &Expr, line: usize) -> Result<RuntimeError, RuntimeError> {
        let hint = "v1 只支持 `throws` + `?` 向上传播，暂不支持 try / catch / finally。";
        if let Expr::New { class, args, .. } = strip_paren(expr) {
            let msg = match args.first() {
                Some(a) => {
                    let v = self.eval(a)?;
                    Some(v.display())
                }
                None => None,
            };
            let mut text = format!("未捕获异常: {}", class);
            if let Some(m) = msg {
                text.push_str(": ");
                text.push_str(&m);
            }
            return Ok(RuntimeError::new(line, text).with_hint(hint));
        }
        let v = self.eval(expr)?;
        Ok(RuntimeError::new(line, format!("未捕获异常: {}", v.display())).with_hint(hint))
    }

    // ---------- switch / enum ----------

    /// `switch` 按 Java 语义顺序匹配：首个命中且 guard 为真的 case 执行其分支体；
    /// 分支体里的 `break` 只结束 switch，`return` / `continue` 继续向外传播。
    fn exec_switch(
        &mut self,
        subject: &Expr,
        cases: &[SwitchCase],
        default: &Option<Vec<Stmt>>,
        line: usize,
    ) -> Result<Flow, RuntimeError> {
        let subj = self.eval(subject)?;
        for case in cases {
            let mut bindings = Vec::new();
            if !self.case_matches(case, &subj, &mut bindings, line)? {
                continue;
            }
            if let Some(f) = self.frames.last_mut() {
                f.scopes.push(bindings);
            }
            let guard = match &case.guard {
                Some(g) => match self.eval(g) {
                    Ok(v) => self.as_bool(&v),
                    Err(e) => Err(e),
                },
                None => Ok(true),
            };
            let guard = match guard {
                Ok(b) => b,
                Err(e) => {
                    self.pop_scope();
                    return Err(e);
                }
            };
            if !guard {
                self.pop_scope();
                continue;
            }
            let r = self.exec_block(&case.body);
            self.pop_scope();
            return match r? {
                Flow::Normal | Flow::Break => Ok(Flow::Normal),
                Flow::Return(v) => Ok(Flow::Return(v)),
                Flow::Continue => Ok(Flow::Continue),
            };
        }
        if let Some(body) = default {
            return match self.exec_block(body)? {
                Flow::Normal | Flow::Break => Ok(Flow::Normal),
                Flow::Return(v) => Ok(Flow::Return(v)),
                Flow::Continue => Ok(Flow::Continue),
            };
        }
        Ok(Flow::Normal)
    }

    /// 判断 case 是否命中；`Variant` 命中时把分支数据按声明顺序绑定进 `bindings_out`。
    fn case_matches(
        &mut self,
        case: &SwitchCase,
        subj: &Value,
        bindings_out: &mut Vec<(String, Slot)>,
        line: usize,
    ) -> Result<bool, RuntimeError> {
        match &case.pattern {
            CasePattern::Bool(b) => Ok(matches!(subj, Value::Bool(v) if v == b)),
            CasePattern::Variant {
                enum_name,
                name,
                bindings,
            } => {
                let Value::Enum {
                    enum_name: en,
                    variant,
                    fields,
                } = subj
                else {
                    return Ok(false);
                };
                if let Some(q) = enum_name {
                    if q != en {
                        return Ok(false);
                    }
                }
                if variant != name {
                    return Ok(false);
                }
                if bindings.len() != fields.len() {
                    return Err(RuntimeError::new(
                        line,
                        format!(
                            "分支 `{}` 需要 {} 个绑定，实际写了 {} 个",
                            name,
                            fields.len(),
                            bindings.len()
                        ),
                    ));
                }
                let params = self
                    .enums
                    .get(en.as_str())
                    .and_then(|ed| ed.variants.iter().find(|v| &v.name == name))
                    .map(|v| v.params.clone())
                    .unwrap_or_default();
                for (i, b) in bindings.iter().enumerate() {
                    let ty = params
                        .get(i)
                        .map(|p| p.ty.clone())
                        .unwrap_or_else(|| dynamic_type(&fields[i]));
                    bindings_out.push((
                        b.clone(),
                        Slot {
                            value: fields[i].clone(),
                            moved: false,
                            ty,
                        },
                    ));
                }
                Ok(true)
            }
            CasePattern::Other(expr) => {
                let v = self.eval(expr)?;
                Ok(values_equal(subj, &v))
            }
        }
    }

    /// `Shape.Circle(2.0)` / `Shape.Empty` 构造枚举值。
    fn eval_enum_variant(
        &mut self,
        enum_name: &str,
        variant: &str,
        args: &[Expr],
    ) -> Result<Value, RuntimeError> {
        let line = self.cur_line;
        let params = self
            .enums
            .get(enum_name)
            .and_then(|ed| ed.variants.iter().find(|v| v.name == variant))
            .map(|v| v.params.clone());
        let params = match params {
            Some(p) => p,
            None => {
                return Err(RuntimeError::new(
                    line,
                    format!("枚举 `{}` 没有分支 `{}`", enum_name, variant),
                ))
            }
        };
        let argv = self.eval_args(args)?;
        if argv.len() != params.len() {
            return Err(RuntimeError::new(
                line,
                format!(
                    "枚举分支 `{}::{}` 需要 {} 个参数，实际给了 {} 个",
                    enum_name,
                    variant,
                    params.len(),
                    argv.len()
                ),
            ));
        }
        Ok(Value::Enum {
            enum_name: enum_name.to_string(),
            variant: variant.to_string(),
            fields: argv,
        })
    }

    // ---------- 变量 ----------

    /// 把值调整成目标类型：`T?` 补 `Some(...)`，非可空类型从 `Some` 解包（`None` 报错）。
    fn coerce_value(&self, v: Value, ty: &Type, line: usize) -> Result<Value, RuntimeError> {
        match ty {
            Type::Nullable(_) => match v {
                Value::Option(_) => Ok(v),
                other => Ok(Value::Option(Some(Box::new(other)))),
            },
            Type::Void => Ok(v),
            _ => match v {
                Value::Option(Some(inner)) => Ok(*inner),
                Value::Option(None) => Err(RuntimeError::new(line, "不能把 `null` 赋给非可空类型")
                    .with_hint(
                        "把目标类型写成 `T?`，或先判空再取出来：`if (x != null) { T v = x; ... }`。",
                    )),
                other => Ok(other),
            },
        }
    }

    /// 赋值目标的声明类型（用于可空包装 / 解包）。
    fn target_type(&mut self, target: &Expr) -> Result<Option<Type>, RuntimeError> {
        match strip_paren(target) {
            Expr::Ident(name) => Ok(self.find_slot(name).ok().map(|s| s.ty.clone())),
            Expr::Field(recv, name) => {
                let rv = self.eval(recv)?;
                match rv {
                    Value::Obj(obj) => Ok(obj
                        .borrow()
                        .fields
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, s)| s.ty.clone())),
                    _ => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }

    fn declare(&mut self, name: &str, ty: Type, value: Value) {
        if let Some(f) = self.frames.last_mut() {
            if let Some(scope) = f.scopes.last_mut() {
                scope.push((
                    name.to_string(),
                    Slot {
                        value,
                        moved: false,
                        ty,
                    },
                ));
            }
        }
    }

    fn var_exists(&self, name: &str) -> bool {
        self.frames.last().is_some_and(|f| {
            f.scopes
                .iter()
                .rev()
                .any(|s| s.iter().any(|(n, _)| n == name))
        })
    }

    /// 读取 `static final` 常量（JR-D8）；`None` 表示该名字不是静态常量。
    fn eval_static_field(
        &mut self,
        class_name: &str,
        field: &str,
    ) -> Result<Option<Value>, RuntimeError> {
        let decl = match self.classes.get(class_name) {
            Some(d) => *d,
            None => return Ok(None),
        };
        let init = match decl
            .fields
            .iter()
            .find(|f| f.is_static && f.name == field)
            .and_then(|f| f.init.as_ref())
        {
            Some(e) => e,
            None => return Ok(None),
        };
        Ok(Some(self.eval(init)?))
    }

    /// 当前类内的裸静态常量引用：`x * N` 里的 `N`。
    fn eval_current_static(&mut self, field: &str) -> Result<Option<Value>, RuntimeError> {
        let class = match self.frames.last() {
            Some(f) => f.class.clone(),
            None => return Ok(None),
        };
        self.eval_static_field(&class, field)
    }

    fn find_slot(&mut self, name: &str) -> Result<&mut Slot, RuntimeError> {
        let line = self.cur_line;
        if let Some(f) = self.frames.last_mut() {
            for scope in f.scopes.iter_mut().rev() {
                if let Some(pos) = scope.iter().position(|(n, _)| n == name) {
                    return Ok(&mut scope[pos].1);
                }
            }
        }
        Err(RuntimeError::new(line, format!("未声明的变量 `{}`", name))
            .with_hint("变量离开作用域后就不可见了；确认它是在当前位置之前声明的。"))
    }

    fn read_var(&mut self, name: &str) -> Result<Value, RuntimeError> {
        let line = self.cur_line;
        let slot = self.find_slot(name)?;
        if slot.moved {
            return Err(use_after_move(line, name));
        }
        Ok(slot.value.clone())
    }

    fn set_var(&mut self, name: &str, value: Value) -> Result<(), RuntimeError> {
        let slot = self.find_slot(name)?;
        slot.value = value;
        slot.moved = false;
        Ok(())
    }

    fn read_this(&mut self) -> Result<Value, RuntimeError> {
        let line = self.cur_line;
        match self.frames.last() {
            Some(Frame { this: Some(v), .. }) => Ok(v.clone()),
            _ => Err(RuntimeError::new(line, "静态方法里没有 `this`")
                .with_hint("`this` 只在实例方法中存在；静态方法要显式传参。")),
        }
    }

    fn eval_place_take(&mut self, e: &Expr) -> Result<Value, RuntimeError> {
        match strip_paren(e) {
            Expr::Ident(_) | Expr::Field(..) | Expr::Index(..) => self.take(e),
            _ => self.eval(e),
        }
    }

    /// 普通赋值 / 声明：与 codegen 的 `.clone()` 对齐——复制一份，源变量仍然可用；
    /// 只有显式 `move` 才搬走所有权。
    fn eval_assign_source(&mut self, e: &Expr) -> Result<Value, RuntimeError> {
        match strip_paren(e) {
            Expr::Unary {
                op: UnOp::Move,
                expr,
            } => self.take(expr),
            Expr::Ident(_) | Expr::Field(..) | Expr::Index(..) => {
                let v = self.eval(e)?;
                Ok(deep_clone(&v))
            }
            _ => self.eval(e),
        }
    }

    /// 按 Rust 的 move 语义读取一个「位置」：非 Copy 值会被标记为已移动。
    fn take(&mut self, e: &Expr) -> Result<Value, RuntimeError> {
        match strip_paren(e) {
            Expr::Ident(name) => {
                let line = self.cur_line;
                let slot = self.find_slot(name)?;
                if slot.moved {
                    return Err(use_after_move(line, name));
                }
                if slot.value.is_copy() {
                    return Ok(slot.value.clone());
                }
                let v = slot.value.clone();
                slot.moved = true;
                Ok(v)
            }
            Expr::Field(recv, name) => {
                // `Limits.NAME` 是静态常量，按值读取，不搬所有权。
                if let Expr::Ident(en) = strip_paren(recv) {
                    if let Some(v) = self.eval_static_field(en, name)? {
                        return Ok(v);
                    }
                }
                // `Shape.Empty` 是无数据枚举常量，不是字段访问：交给普通求值。
                if let Expr::Ident(en) = strip_paren(recv) {
                    if let Some(ed) = self.enums.get(en.as_str()) {
                        if ed
                            .variants
                            .iter()
                            .any(|v| &v.name == name && v.params.is_empty())
                        {
                            return self.eval(e);
                        }
                    }
                }
                let line = self.cur_line;
                let rv = self.eval(recv)?;
                match rv {
                    Value::Obj(obj) => {
                        let mut inst = obj.borrow_mut();
                        match inst.fields.iter_mut().find(|(n, _)| n == name) {
                            Some((_, slot)) => {
                                if slot.moved {
                                    return Err(use_after_move(line, name));
                                }
                                if slot.value.is_copy() {
                                    Ok(slot.value.clone())
                                } else {
                                    let v = slot.value.clone();
                                    slot.moved = true;
                                    Ok(v)
                                }
                            }
                            None => Err(RuntimeError::new(
                                line,
                                format!("{} 没有字段 `{}`", inst.class, name),
                            )),
                        }
                    }
                    _ => Err(RuntimeError::new(line, "只有对象才有字段")),
                }
            }
            // 从集合下标搬所有权在 Rust 里也不行（E0507），解释器按借用处理。
            _ => self.eval(e),
        }
    }

    fn assign(&mut self, target: &Expr, value: Value) -> Result<(), RuntimeError> {
        match strip_paren(target) {
            Expr::Ident(name) => self.set_var(name, value),
            Expr::Field(recv, name) => {
                let line = self.cur_line;
                let rv = self.eval(recv)?;
                match rv {
                    Value::Obj(obj) => {
                        let mut inst = obj.borrow_mut();
                        match inst.fields.iter_mut().find(|(n, _)| n == name) {
                            Some((_, slot)) => {
                                slot.value = value;
                                slot.moved = false;
                                Ok(())
                            }
                            None => Err(RuntimeError::new(
                                line,
                                format!("{} 没有字段 `{}`", inst.class, name),
                            )),
                        }
                    }
                    _ => Err(RuntimeError::new(line, "只有对象才有字段")),
                }
            }
            Expr::Index(base, idx) => {
                let line = self.cur_line;
                let b = self.eval(base)?;
                let iv = self.eval(idx)?;
                let i = self.as_usize(&iv)?;
                match b {
                    Value::List(rc) => {
                        let mut items = rc.borrow_mut();
                        if i >= items.len() {
                            return Err(index_error(line, i, items.len()));
                        }
                        items[i] = value;
                        Ok(())
                    }
                    other => Err(RuntimeError::new(
                        line,
                        format!("{} 不支持下标赋值", other.type_name()),
                    )),
                }
            }
            _ => Err(RuntimeError::new(
                self.cur_line,
                "赋值目标必须是变量、字段或下标",
            )),
        }
    }

    // ---------- 表达式 ----------

    fn eval(&mut self, e: &Expr) -> Result<Value, RuntimeError> {
        match e {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Float(f) => Ok(Value::Double(*f)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Char(c) => Ok(Value::Char(*c)),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Null => Ok(Value::Option(None)),
            Expr::Ident(name) => match self.read_var(name) {
                Ok(v) => Ok(v),
                // 类内裸引用静态常量：`x * N` 里的 `N` 是 `Limits.N`（JR-D8）。
                Err(e) => match self.eval_current_static(name)? {
                    Some(v) => Ok(v),
                    None => Err(e),
                },
            },
            Expr::This(_) => self.read_this(),
            Expr::Field(recv, name) => {
                // `Limits.N`：类名 + static final 常量（JR-D8）。
                if let Expr::Ident(en) = strip_paren(recv) {
                    if let Some(v) = self.eval_static_field(en, name)? {
                        return Ok(v);
                    }
                }
                // `Shape.Empty`：不带数据的枚举分支当常量用。
                if let Expr::Ident(en) = strip_paren(recv) {
                    if let Some(ed) = self.enums.get(en.as_str()) {
                        if ed
                            .variants
                            .iter()
                            .any(|v| &v.name == name && v.params.is_empty())
                        {
                            return Ok(Value::Enum {
                                enum_name: en.clone(),
                                variant: name.clone(),
                                fields: Vec::new(),
                            });
                        }
                    }
                }
                let v = self.eval(recv)?;
                self.read_field(&v, name)
            }
            Expr::Paren(inner) => self.eval(inner),
            Expr::Unary { op, expr } => match op {
                UnOp::Move => self.take(expr),
                UnOp::Neg => {
                    let line = self.cur_line;
                    match self.eval(expr)? {
                        Value::Int(n) => Ok(Value::Int(-n)),
                        Value::Double(f) => Ok(Value::Double(-f)),
                        other => Err(RuntimeError::new(
                            line,
                            format!("`-` 不能作用于 {}", other.type_name()),
                        )),
                    }
                }
                UnOp::Not => {
                    let line = self.cur_line;
                    match self.eval(expr)? {
                        Value::Bool(b) => Ok(Value::Bool(!b)),
                        other => Err(RuntimeError::new(
                            line,
                            format!("`!` 不能作用于 {}", other.type_name()),
                        )),
                    }
                }
            },
            Expr::Binary { op, lhs, rhs } => self.eval_binary(*op, lhs, rhs),
            Expr::Index(base, idx) => {
                let line = self.cur_line;
                let b = self.eval(base)?;
                let iv = self.eval(idx)?;
                let i = self.as_usize(&iv)?;
                match b {
                    Value::List(rc) => {
                        let items = rc.borrow();
                        match items.get(i) {
                            Some(v) => Ok(v.clone()),
                            None => Err(index_error(line, i, items.len())),
                        }
                    }
                    other => Err(RuntimeError::new(
                        line,
                        format!("{} 不支持下标访问", other.type_name()),
                    )),
                }
            }
            Expr::New { class, args, .. } => self.eval_new(class, args),
            Expr::NewArray { elem, len } => {
                let line = self.cur_line;
                let n = match self.eval(len)? {
                    Value::Int(k) if k >= 0 => k as usize,
                    Value::Int(k) => {
                        return Err(RuntimeError::new(
                            line,
                            format!("数组长度不能为负数: {}", k),
                        )
                        .with_hint("Java 会抛 NegativeArraySizeException；先保证长度 >= 0。"))
                    }
                    other => {
                        return Err(RuntimeError::new(
                            line,
                            format!("数组长度需要 int，实际是 {}", other.type_name()),
                        ))
                    }
                };
                Ok(Value::List(Rc::new(RefCell::new(vec![
                    default_value(elem);
                    n
                ]))))
            }
            Expr::Call { recv, name, args } => self.eval_call(recv.as_deref(), name, args),
            Expr::Closure { .. } => {
                let line = self.cur_line;
                Err(RuntimeError::new(line, "解释器暂不支持闭包").with_hint(
                    "闭包目前只在转译链路实现；用 `jrust build` 生成 Rust 后交给 cargo 运行。",
                ))
            }
            // 解释器同步执行：`await e` 就是 `e`（JR-C10）。
            Expr::Await(inner) => self.eval(inner),
            // `?` 在解释器里就是「异常继续向上抛」：成功时原样返回，失败时错误自然外传。
            Expr::Try(inner) => self.eval(inner),
            Expr::Raw { line, .. } => Err(RuntimeError::new(*line, "解释器无法执行 raw 块")
                .with_hint(
                    "raw 是原样内联的 Rust 源码；用 `jrust build` 生成 Rust 后交给 cargo 运行。",
                )),
        }
    }

    fn read_field(&mut self, recv: &Value, name: &str) -> Result<Value, RuntimeError> {
        let line = self.cur_line;
        match recv {
            Value::Obj(obj) => {
                let inst = obj.borrow();
                match inst.fields.iter().find(|(n, _)| n == name) {
                    Some((_, slot)) if slot.moved => Err(use_after_move(line, name)),
                    Some((_, slot)) => Ok(slot.value.clone()),
                    None => Err(RuntimeError::new(
                        line,
                        format!("{} 没有字段 `{}`", inst.class, name),
                    )),
                }
            }
            other => Err(RuntimeError::new(
                line,
                format!("{} 没有字段", other.type_name()),
            )),
        }
    }

    fn eval_binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Result<Value, RuntimeError> {
        let line = self.cur_line;
        match op {
            BinOp::And => {
                let l = self.eval(lhs)?;
                if !self.as_bool(&l)? {
                    return Ok(Value::Bool(false));
                }
                let r = self.eval(rhs)?;
                Ok(Value::Bool(self.as_bool(&r)?))
            }
            BinOp::Or => {
                let l = self.eval(lhs)?;
                if self.as_bool(&l)? {
                    return Ok(Value::Bool(true));
                }
                let r = self.eval(rhs)?;
                Ok(Value::Bool(self.as_bool(&r)?))
            }
            _ => {
                let a = self.eval(lhs)?;
                let b = self.eval(rhs)?;
                self.apply_binary(op, &a, &b, line)
            }
        }
    }

    fn apply_binary(
        &mut self,
        op: BinOp,
        a: &Value,
        b: &Value,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        match op {
            BinOp::Add => {
                if matches!(a, Value::Str(_)) || matches!(b, Value::Str(_)) {
                    return Ok(Value::Str(format!("{}{}", a.display(), b.display())));
                }
                self.numeric(op, a, b, line)
            }
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => self.numeric(op, a, b, line),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let (x, y) = self.numeric_pair(a, b, line)?;
                let r = match op {
                    BinOp::Lt => x < y,
                    BinOp::Le => x <= y,
                    BinOp::Gt => x > y,
                    BinOp::Ge => x >= y,
                    _ => unreachable!(),
                };
                Ok(Value::Bool(r))
            }
            BinOp::Eq | BinOp::Ne => {
                let eq = values_equal(a, b);
                Ok(Value::Bool(if op == BinOp::Eq { eq } else { !eq }))
            }
            BinOp::And | BinOp::Or => unreachable!(),
        }
    }

    fn numeric(
        &self,
        op: BinOp,
        a: &Value,
        b: &Value,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => {
                let r = match op {
                    BinOp::Add => x.checked_add(*y),
                    BinOp::Sub => x.checked_sub(*y),
                    BinOp::Mul => x.checked_mul(*y),
                    BinOp::Div => {
                        if *y == 0 {
                            return Err(RuntimeError::new(line, "整数除零").with_hint("先判断除数非零。"));
                        }
                        x.checked_div(*y)
                    }
                    BinOp::Rem => {
                        if *y == 0 {
                            return Err(RuntimeError::new(line, "整数取余除数为零"));
                        }
                        x.checked_rem(*y)
                    }
                    _ => None,
                };
                match r {
                    Some(v) => Ok(Value::Int(v)),
                    None => Err(RuntimeError::new(line, "整数运算溢出")),
                }
            }
            _ => {
                let (x, y) = self.numeric_pair(a, b, line)?;
                let v = match op {
                    BinOp::Add => x + y,
                    BinOp::Sub => x - y,
                    BinOp::Mul => x * y,
                    BinOp::Div => {
                        if y == 0.0 {
                            return Err(RuntimeError::new(line, "浮点除零").with_hint("先判断除数非零。"));
                        }
                        x / y
                    }
                    BinOp::Rem => x % y,
                    _ => return Err(RuntimeError::new(line, "不支持的数值运算")),
                };
                Ok(Value::Double(v))
            }
        }
    }

    fn numeric_pair(
        &self,
        a: &Value,
        b: &Value,
        line: usize,
    ) -> Result<(f64, f64), RuntimeError> {
        match (a.as_number(), b.as_number()) {
            (Some(x), Some(y)) => Ok((x, y)),
            _ => Err(RuntimeError::new(
                line,
                format!(
                    "运算需要数字，实际是 {} 和 {}",
                    a.type_name(),
                    b.type_name()
                ),
            )),
        }
    }

    fn eval_args(&mut self, args: &[Expr]) -> Result<Vec<Value>, RuntimeError> {
        let mut out = Vec::new();
        for a in args {
            // 实参默认按借用传递：普通变量不会因「传进方法」被标记为已移动（JR-O11 / @ref）。
            // 想搬走所有权必须显式写 `move x`，`eval` 的 Unary::Move 分支仍会调用 take。
            out.push(self.eval(a)?);
        }
        Ok(out)
    }

    fn eval_new(&mut self, class: &str, args: &[Expr]) -> Result<Value, RuntimeError> {
        let line = self.cur_line;
        if matches!(
            class,
            "ArrayList" | "LinkedList" | "List" | "Collection" | "Vec"
        ) {
            return Ok(Value::List(Rc::new(RefCell::new(Vec::new()))));
        }
        if matches!(class, "HashSet" | "Set") {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    line,
                    "jrust MVP 只支持无参的 new HashSet<>()",
                )
                .with_hint("先 `new HashSet<>()`，再用 `add` 逐个插入。"));
            }
            return Ok(Value::Set(Rc::new(RefCell::new(Vec::new()))));
        }
        if matches!(class, "HashMap" | "Map") {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    line,
                    "jrust MVP 只支持无参的 new HashMap<>()",
                )
                .with_hint("先 `new HashMap<>()`，再用 `put` 逐个插入。"));
            }
            return Ok(Value::Map(Rc::new(RefCell::new(HashMap::new()))));
        }
        let decl: &'a ClassDecl = match self.classes.get(class) {
            Some(d) => *d,
            None => {
                return Err(RuntimeError::new(line, format!("未知的类 `{}`", class))
                    .with_hint("解释执行只认识当前文件里声明的类。"))
            }
        };
        let argv = self.eval_args(args)?;
        let fields: Vec<(String, Slot)> = decl
            .fields
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    Slot {
                        value: default_value(&f.ty),
                        moved: false,
                        ty: f.ty.clone(),
                    },
                )
            })
            .collect();
        let obj = Rc::new(RefCell::new(Instance {
            class: class.to_string(),
            fields,
        }));
        if let Some(ctor) = &decl.ctor {
            if argv.len() != ctor.params.len() {
                return Err(RuntimeError::new(
                    line,
                    format!(
                        "构造器 {} 需要 {} 个参数，实际给了 {} 个",
                        class,
                        ctor.params.len(),
                        argv.len()
                    ),
                ));
            }
            self.call_ctor(class, ctor, obj.clone(), argv)?;
        } else if !argv.is_empty() {
            return Err(RuntimeError::new(
                line,
                format!("类 `{}` 没有构造器，却传了参数", class),
            ));
        }
        Ok(Value::Obj(obj))
    }

    fn call_ctor(
        &mut self,
        class: &str,
        ctor: &Constructor,
        obj: Rc<RefCell<Instance>>,
        args: Vec<Value>,
    ) -> Result<(), RuntimeError> {
        let mut scope = Vec::new();
        for (p, v) in ctor.params.iter().zip(args.into_iter()) {
            let v = self.coerce_value(v, &p.ty, ctor.line)?;
            scope.push((
                p.name.clone(),
                Slot {
                    value: v,
                    moved: false,
                    ty: p.ty.clone(),
                },
            ));
        }
        self.frames.push(Frame {
            class: class.to_string(),
            method: "new".to_string(),
            line: ctor.line,
            scopes: vec![scope],
            this: Some(Value::Obj(obj)),
            ret_ty: Type::Void,
        });
        let r = self.exec_block(&ctor.body);
        self.frames.pop();
        r.map(|_| ())
    }

    fn eval_call(
        &mut self,
        recv: Option<&Expr>,
        name: &str,
        args: &[Expr],
    ) -> Result<Value, RuntimeError> {
        let line = self.cur_line;
        if (name == "println" || name == "print") && recv.map_or(true, is_system_out) {
            let text = if args.is_empty() {
                String::new()
            } else if args.len() == 1 {
                let v = self.eval(&args[0])?;
                if matches!(v, Value::Option(_)) {
                    return Err(RuntimeError::new(line, "不能直接打印可空类型 `T?`")
                        .with_hint(
                            "先判空并取出非空值：`if (x != null) { T v = x; println(v); }`。",
                        ));
                }
                v.display()
            } else {
                let fmt = match &args[0] {
                    Expr::Str(s) => s.clone(),
                    _ => {
                        return Err(RuntimeError::new(
                            line,
                            "多参数打印要求第一个参数是格式串字面量，例如 println(\"{} {}\", a, b)",
                        ))
                    }
                };
                let mut vals = Vec::new();
                for a in &args[1..] {
                    let v = self.eval(a)?;
                    if matches!(v, Value::Option(_)) {
                        return Err(RuntimeError::new(line, "不能直接打印可空类型 `T?`")
                            .with_hint(
                                "先判空并取出非空值：`if (x != null) { T v = x; println(v); }`。",
                            ));
                    }
                    vals.push(v.display());
                }
                match format_with(&fmt, &vals) {
                    Some(t) => t,
                    None => {
                        return Err(RuntimeError::new(
                            line,
                            "格式串里的 `{}` 个数与参数个数不一致（字面花括号写 `{{` / `}}`）",
                        ))
                    }
                }
            };
            if name == "println" {
                self.emit(text);
            } else {
                self.emit_no_newline(text);
            }
            return Ok(Value::Unit);
        }
        // 内置 `panic` / `assert`（JR-C7）。
        if recv.is_none() && (name == "panic" || name == "assert") {
            if name == "panic" {
                let msg = if args.len() == 1 {
                    self.eval(&args[0])?.display()
                } else {
                    String::new()
                };
                return Err(RuntimeError::new(line, format!("panic: {}", msg)));
            }
            let Some(cond) = args.first() else {
                return Err(RuntimeError::new(line, "assert 需要条件参数"));
            };
            if !matches!(self.eval(cond)?, Value::Bool(true)) {
                let msg = if args.len() > 1 {
                    self.eval(&args[1])?.display()
                } else {
                    "断言失败".to_string()
                };
                return Err(RuntimeError::new(line, format!("assert 失败: {}", msg)));
            }
            return Ok(Value::Unit);
        }
        // `Shape.Circle(2.0)`：枚举分支构造（在求值 recv 之前拦截，避免把类型名当变量）。
        if let Some(recv_expr) = recv {
            if let Expr::Ident(en) = strip_paren(recv_expr) {
                if self.enums.contains_key(en.as_str()) {
                    return self.eval_enum_variant(en, name, args);
                }
            }
        }
        // `Runner.run(1)`：类名作接收者 → 静态方法调用（同名局部变量优先当对象）。
        if let Some(recv_expr) = recv {
            if let Expr::Ident(cname) = strip_paren(recv_expr) {
                if !self.var_exists(cname) {
                    if let Some(decl) = self.classes.get(cname.as_str()).copied() {
                        if let Some(m) =
                            decl.methods.iter().find(|m| m.name == name && m.is_static)
                        {
                            let argv = self.eval_args(args)?;
                            self.check_arity(name, m.params.len(), argv.len(), line)?;
                            return self.call_method(cname, m, None, argv);
                        }
                    } else if let Some(iface) = self.interfaces.get(cname.as_str()).copied() {
                        // 接口 static 方法（JR-D10）：`Greeter.tag()`。
                        if let Some(sig) = iface
                            .methods
                            .iter()
                            .find(|m| m.name == name && m.is_static)
                        {
                            let argv = self.eval_args(args)?;
                            self.check_arity(name, sig.params.len(), argv.len(), line)?;
                            return self.call_iface_method(cname, sig, None, argv);
                        }
                    }
                }
            }
        }
        if let Some(r) = recv {
            let rv = self.eval(r)?;
            match &rv {
                Value::List(rc) => match name {
                    "add" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(line, "add 需要一个参数"));
                        }
                        let v = self.eval_place_take(&args[0])?;
                        rc.borrow_mut().push(v);
                        Ok(Value::Unit)
                    }
                    "size" | "length" => Ok(Value::Int(rc.borrow().len() as i64)),
                    "isEmpty" => Ok(Value::Bool(rc.borrow().is_empty())),
                    "get" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(line, "get 需要一个下标"));
                        }
                        let iv = self.eval(&args[0])?;
                        let i = self.as_usize(&iv)?;
                        let items = rc.borrow();
                        match items.get(i) {
                            Some(v) => Ok(v.clone()),
                            None => Err(index_error(line, i, items.len())),
                        }
                    }
                    // `xs.stream()` 取一份快照，后续 map / filter 不改动原列表（JR-E18）。
                    "stream" => Ok(Value::List(Rc::new(RefCell::new(
                        rc.borrow().iter().map(deep_clone).collect(),
                    )))),
                    // `for (T x : c.mutable())`：元素按引用共享，循环里改字段会写回原对象（JR-E4）。
                    "mutable" => Ok(rv.clone()),
                    "map" | "filter" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(
                                line,
                                format!("`{}` 需要一个闭包参数，例如 `x -> x * 2`", name),
                            ));
                        }
                        let items = rc.borrow().clone();
                        let mut out = Vec::new();
                        for item in items {
                            let r = self.call_closure(&args[0], vec![item.clone()])?;
                            if name == "map" {
                                out.push(r);
                            } else if self.as_bool(&r)? {
                                out.push(item);
                            }
                        }
                        Ok(Value::List(Rc::new(RefCell::new(out))))
                    }
                    "collectList" => Ok(Value::List(Rc::new(RefCell::new(
                        rc.borrow().iter().map(deep_clone).collect(),
                    )))),
                    other => Err(RuntimeError::new(
                        line,
                        format!("List 没有方法 `{}`", other),
                    )),
                },
                Value::Set(rc) => match name {
                    "add" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(line, "add 需要一个参数"));
                        }
                        let v = self.eval_place_take(&args[0])?;
                        let mut items = rc.borrow_mut();
                        if !items.iter().any(|x| values_equal(x, &v)) {
                            items.push(v);
                        }
                        Ok(Value::Bool(true))
                    }
                    "contains" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(line, "contains 需要一个参数"));
                        }
                        let v = self.eval(&args[0])?;
                        Ok(Value::Bool(
                            rc.borrow().iter().any(|x| values_equal(x, &v)),
                        ))
                    }
                    "remove" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(line, "remove 需要一个参数"));
                        }
                        let v = self.eval(&args[0])?;
                        let mut items = rc.borrow_mut();
                        match items.iter().position(|x| values_equal(x, &v)) {
                            Some(pos) => {
                                items.remove(pos);
                                Ok(Value::Bool(true))
                            }
                            None => Ok(Value::Bool(false)),
                        }
                    }
                    "size" | "length" => Ok(Value::Int(rc.borrow().len() as i64)),
                    "isEmpty" => Ok(Value::Bool(rc.borrow().is_empty())),
                    other => Err(RuntimeError::new(
                        line,
                        format!("Set 没有方法 `{}`", other),
                    )),
                },
                Value::Str(s) => match name {
                    // Java 的 `length()` 数的是字符个数；Rust 的 `len()` 是字节数。
                    // 非 ASCII 时两者不同——产物侧补 `chars().count()` 对齐（见 codegen）。
                    "length" => Ok(Value::Int(s.chars().count() as i64)),
                    "isEmpty" => Ok(Value::Bool(s.is_empty())),
                    // 阶段 D / JR-E21：与 codegen 的映射表保持一致。
                    "trim" => Ok(Value::Str(s.trim().to_string())),
                    "toLowerCase" => Ok(Value::Str(s.to_lowercase())),
                    "toUpperCase" => Ok(Value::Str(s.to_uppercase())),
                    "toString" => Ok(Value::Str(s.clone())),
                    "startsWith" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(line, "startsWith 需要一个字符串参数"));
                        }
                        match self.eval(&args[0])? {
                            Value::Str(p) => Ok(Value::Bool(s.starts_with(p.as_str()))),
                            _ => Err(RuntimeError::new(line, "startsWith 需要一个字符串参数")),
                        }
                    }
                    "endsWith" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(line, "endsWith 需要一个字符串参数"));
                        }
                        match self.eval(&args[0])? {
                            Value::Str(p) => Ok(Value::Bool(s.ends_with(p.as_str()))),
                            _ => Err(RuntimeError::new(line, "endsWith 需要一个字符串参数")),
                        }
                    }
                    "contains" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(line, "contains 需要一个字符串参数"));
                        }
                        match self.eval(&args[0])? {
                            Value::Str(p) => Ok(Value::Bool(s.contains(p.as_str()))),
                            _ => Err(RuntimeError::new(line, "contains 需要一个字符串参数")),
                        }
                    }
                    "equals" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(line, "equals 需要一个字符串参数"));
                        }
                        match self.eval(&args[0])? {
                            Value::Str(p) => Ok(Value::Bool(*s == p)),
                            _ => Err(RuntimeError::new(line, "equals 需要一个字符串参数")),
                        }
                    }
                    other => Err(RuntimeError::new(
                        line,
                        format!("String 没有方法 `{}`", other),
                    )),
                },
                Value::Map(rc) => match name {
                    "put" => {
                        if args.len() != 2 {
                            return Err(RuntimeError::new(line, "put 需要键和值两个参数"));
                        }
                        let k = self.eval(&args[0])?;
                        let v = self.eval_place_take(&args[1])?;
                        rc.borrow_mut().insert(map_key(&k), v);
                        Ok(Value::Unit)
                    }
                    "size" => Ok(Value::Int(rc.borrow().len() as i64)),
                    "isEmpty" => Ok(Value::Bool(rc.borrow().is_empty())),
                    "containsKey" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(line, "containsKey 需要一个键"));
                        }
                        let k = self.eval(&args[0])?;
                        Ok(Value::Bool(rc.borrow().contains_key(&map_key(&k))))
                    }
                    "get" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(line, "get 需要一个键"));
                        }
                        let k = self.eval(&args[0])?;
                        // 2.0 语义（SPEC-v2 §3-1）：`Map.get` 返回 `Option<V>`，
                        // 键缺失得到 `None`，**不再 panic**。与 `getOrNull` 完全等价。
                        let found = rc.borrow().get(&map_key(&k)).cloned();
                        Ok(Value::Option(found.map(Box::new)))
                    }
                    "getOrNull" => {
                        if args.len() != 1 {
                            return Err(RuntimeError::new(line, "getOrNull 需要一个键"));
                        }
                        let k = self.eval(&args[0])?;
                        let found = rc.borrow().get(&map_key(&k)).cloned();
                        Ok(Value::Option(found.map(Box::new)))
                    }
                    other => Err(RuntimeError::new(
                        line,
                        format!("Map 没有方法 `{}`", other),
                    )),
                },
                Value::Option(_) => Err(RuntimeError::new(
                    line,
                    format!("可空值不能直接调用 `{}`", name),
                )
                .with_hint(
                    "先判空再取出来：`if (x != null) { T v = x; v.method() }`（→ `if let Some(v) = x`）。",
                )),
                Value::Obj(_) => {
                    let class = match &rv {
                        Value::Obj(obj) => obj.borrow().class.clone(),
                        _ => unreachable!(),
                    };
                    let decl: &'a ClassDecl = match self.classes.get(class.as_str()) {
                        Some(d) => *d,
                        None => return Err(RuntimeError::new(line, format!("未知的类 `{}`", class))),
                    };
                    if let Some(method) = decl.methods.iter().find(|m| m.name == name) {
                        let argv = self.eval_args(args)?;
                        self.check_arity(name, method.params.len(), argv.len(), line)?;
                        return self.call_method(&class, method, Some(rv.clone()), argv);
                    }
                    // 接口默认方法（JR-D10）：类里没实现时回退到接口的默认实现。
                    if let Some(sig) = self.find_iface_method(decl, name) {
                        let argv = self.eval_args(args)?;
                        self.check_arity(name, sig.params.len(), argv.len(), line)?;
                        return self.call_iface_method(&class, sig, Some(rv.clone()), argv);
                    }
                    Err(RuntimeError::new(
                        line,
                        format!("{} 没有方法 `{}`", class, name),
                    ))
                }
                other => Err(RuntimeError::new(
                    line,
                    format!("{} 没有方法 `{}`", other.type_name(), name),
                )),
            }
        } else {
            // 裸函数调用：当前类的静态方法。
            let class = match self.frames.last() {
                Some(f) => f.class.clone(),
                None => return Err(RuntimeError::new(line, format!("未知函数 `{}`", name))),
            };
            let decl: &'a ClassDecl = match self.classes.get(class.as_str()) {
                Some(d) => *d,
                None => return Err(RuntimeError::new(line, format!("未知的类 `{}`", class))),
            };
            let method = match decl.methods.iter().find(|m| m.name == name && m.is_static) {
                Some(m) => m,
                None => {
                    return Err(RuntimeError::new(
                        line,
                        format!("找不到静态方法 `{}.{}`", class, name),
                    ))
                }
            };
            let argv = self.eval_args(args)?;
            self.check_arity(name, method.params.len(), argv.len(), line)?;
            self.call_method(&class, method, None, argv)
        }
    }

    fn check_arity(
        &self,
        name: &str,
        expected: usize,
        actual: usize,
        line: usize,
    ) -> Result<(), RuntimeError> {
        if expected == actual {
            Ok(())
        } else {
            Err(RuntimeError::new(
                line,
                format!("`{}` 需要 {} 个参数，实际给了 {} 个", name, expected, actual),
            ))
        }
    }

    /// 在迭代器链（`map` / `filter`）里按元素调用闭包：参数绑定到新作用域，
    /// 闭包体可以照常读取 `this` 和外层局部变量（按引用捕获）。
    fn call_closure(&mut self, e: &Expr, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let line = self.cur_line;
        let (params, body) = match strip_paren(e) {
            Expr::Closure { params, body } => (params, body),
            _ => {
                return Err(RuntimeError::new(
                    line,
                    "迭代器链的 map / filter 需要一个闭包参数，例如 `x -> x * 2`",
                ))
            }
        };
        if params.len() != args.len() {
            return Err(RuntimeError::new(
                line,
                format!(
                    "闭包需要 {} 个参数，实际给了 {} 个",
                    params.len(),
                    args.len()
                ),
            ));
        }
        let mut scope = Vec::new();
        for (p, v) in params.iter().zip(args.into_iter()) {
            let ty = p.ty.clone().unwrap_or_else(|| dynamic_type(&v));
            let v = self.coerce_value(v, &ty, line)?;
            scope.push((
                p.name.clone(),
                Slot {
                    value: v,
                    moved: false,
                    ty,
                },
            ));
        }
        if let Some(f) = self.frames.last_mut() {
            f.scopes.push(scope);
        }
        let r = self.eval(body);
        self.pop_scope();
        r
    }

    fn call_method(
        &mut self,
        class: &str,
        method: &Method,
        this: Option<Value>,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let mut scope = Vec::new();
        for (p, v) in method.params.iter().zip(args.into_iter()) {
            let v = self.coerce_value(v, &p.ty, method.line)?;
            scope.push((
                p.name.clone(),
                Slot {
                    value: v,
                    moved: false,
                    ty: p.ty.clone(),
                },
            ));
        }
        self.frames.push(Frame {
            class: class.to_string(),
            method: method.name.clone(),
            line: method.line,
            scopes: vec![scope],
            this,
            ret_ty: method.ret.clone(),
        });
        let r = self.exec_block(&method.body);
        self.frames.pop();
        match r? {
            Flow::Return(v) => Ok(v),
            _ => Ok(Value::Unit),
        }
    }

    /// 接口方法（含父接口）的默认实现：类里没写该方法时回退到这里。
    fn find_iface_method(&self, class: &'a ClassDecl, name: &str) -> Option<&'a MethodSig> {
        let mut stack: Vec<&'a InterfaceDecl> = Vec::new();
        for t in &class.implements {
            if let Some(i) = self.interface_decl(t) {
                stack.push(i);
            }
        }
        let mut seen: Vec<&str> = Vec::new();
        while let Some(i) = stack.pop() {
            if seen.contains(&i.name.as_str()) {
                continue;
            }
            seen.push(i.name.as_str());
            if let Some(sig) = i
                .methods
                .iter()
                .find(|m| m.name == name && !m.is_static && m.body.is_some())
            {
                return Some(sig);
            }
            for t in &i.extends {
                if let Some(p) = self.interface_decl(t) {
                    stack.push(p);
                }
            }
        }
        None
    }

    fn interface_decl(&self, t: &Type) -> Option<&'a InterfaceDecl> {
        let name = match t {
            Type::Named(n) => n.as_str(),
            Type::Generic(n, _) => n.as_str(),
            _ => return None,
        };
        self.interfaces.get(name).copied()
    }

    fn call_iface_method(
        &mut self,
        class: &str,
        sig: &MethodSig,
        this: Option<Value>,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let body = match &sig.body {
            Some(b) => b,
            None => {
                return Err(RuntimeError::new(
                    sig.line,
                    format!("接口方法 `{}` 只有签名，没有默认实现", sig.name),
                ))
            }
        };
        let mut scope = Vec::new();
        for (p, v) in sig.params.iter().zip(args.into_iter()) {
            let v = self.coerce_value(v, &p.ty, sig.line)?;
            scope.push((
                p.name.clone(),
                Slot {
                    value: v,
                    moved: false,
                    ty: p.ty.clone(),
                },
            ));
        }
        self.frames.push(Frame {
            class: class.to_string(),
            method: sig.name.clone(),
            line: sig.line,
            scopes: vec![scope],
            this,
            ret_ty: sig.ret.clone(),
        });
        let r = self.exec_block(body);
        self.frames.pop();
        match r? {
            Flow::Return(v) => Ok(v),
            _ => Ok(Value::Unit),
        }
    }

    // ---------- 值转换 ----------

    fn as_bool(&self, v: &Value) -> Result<bool, RuntimeError> {
        match v {
            Value::Bool(b) => Ok(*b),
            other => Err(RuntimeError::new(
                self.cur_line,
                format!("条件需要 boolean，实际是 {}", other.type_name()),
            )),
        }
    }

    fn int_of(&self, v: Value, line: usize) -> Result<i64, RuntimeError> {
        match v {
            Value::Int(n) => Ok(n),
            other => Err(RuntimeError::new(
                line,
                format!("for 的边界需要整数，实际是 {}", other.type_name()),
            )
            .with_hint("C 风格 for 只支持 int / long 计数。")),
        }
    }

    fn as_usize(&self, v: &Value) -> Result<usize, RuntimeError> {
        match v {
            Value::Int(n) if *n >= 0 => Ok(*n as usize),
            Value::Int(n) => Err(RuntimeError::new(
                self.cur_line,
                format!("下标不能为负数: {}", n),
            )),
            other => Err(RuntimeError::new(
                self.cur_line,
                format!("下标需要 int，实际是 {}", other.type_name()),
            )),
        }
    }

    // ---------- 调试快照 ----------

    pub fn snapshot(&self) -> Snapshot {
        let mut frames = Vec::new();
        for f in &self.frames {
            let mut vars = Vec::new();
            if let Some(this) = &f.this {
                collect_vars("this", this, None, 2, &mut vars);
            }
            for scope in &f.scopes {
                for (name, slot) in scope {
                    if slot.moved {
                        vars.push(VarView {
                            name: name.clone(),
                            value: "<已 move>".to_string(),
                            ty: type_str(&slot.ty),
                        });
                    } else {
                        collect_vars(name, &slot.value, Some(&slot.ty), 2, &mut vars);
                    }
                }
            }
            frames.push(FrameView {
                name: format!("{}.{}", f.class, f.method),
                line: f.line,
                vars,
            });
        }
        Snapshot { frames }
    }
}

impl Value {
    fn as_number(&self) -> Option<f64> {
        match self {
            Value::Int(n) => Some(*n as f64),
            Value::Double(f) => Some(*f),
            _ => None,
        }
    }
}

fn use_after_move(line: usize, name: &str) -> RuntimeError {
    RuntimeError::new(line, format!("使用已移动的值 `{}`", name)).with_hint(
        "Rust 的所有权只能转移一次：`String b = a;` 之后 `a` 就失效了。需要两份值时，让原变量重新赋值，或调整代码只搬一次。",
    )
}

fn index_error(line: usize, index: usize, len: usize) -> RuntimeError {
    RuntimeError::new(
        line,
        format!("下标越界: 访问 [{}]，但长度是 {}", index, len),
    )
}

/// Map 的键在解释器里统一存成带类型标签的字符串，避免 `1` 与 `"1"` 混淆。
fn map_key(v: &Value) -> String {
    match v {
        Value::Int(n) => format!("i:{}", n),
        Value::Str(s) => format!("s:{}", s),
        Value::Bool(b) => format!("b:{}", b),
        Value::Double(f) => format!("d:{}", f),
        Value::Char(c) => format!("c:{}", c),
        other => format!("o:{}", other.debug_repr()),
    }
}

fn stmt_line(s: &Stmt) -> usize {
    match s {
        Stmt::Let { line, .. }
        | Stmt::Assign { line, .. }
        | Stmt::Expr { line, .. }
        | Stmt::Return { line, .. }
        | Stmt::If { line, .. }
        | Stmt::While { line, .. }
        | Stmt::ForEach { line, .. }
        | Stmt::ForRange { line, .. }
        | Stmt::Break { line }
        | Stmt::Continue { line }
        | Stmt::Switch { line, .. }
        | Stmt::Spawn { line, .. }
        | Stmt::Raw { line, .. }
        | Stmt::Sync { line, .. }
        | Stmt::Throw { line, .. } => *line,
    }
}

/// `==` / `!=` 的比较语义；`null` 用 `Option(None)` 表示，因此 `x == null` 天然成立。
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Double(x), Value::Double(y)) => x == y,
        (Value::Int(x), Value::Double(y)) => (*x as f64) == *y,
        (Value::Double(x), Value::Int(y)) => *x == (*y as f64),
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Char(x), Value::Char(y)) => x == y,
        (Value::Unit, Value::Unit) => true,
        (Value::Option(x), Value::Option(y)) => match (x, y) {
            (None, None) => true,
            (Some(p), Some(q)) => values_equal(p, q),
            _ => false,
        },
        (
            Value::Enum {
                enum_name: en1,
                variant: v1,
                fields: f1,
            },
            Value::Enum {
                enum_name: en2,
                variant: v2,
                fields: f2,
            },
        ) => {
            en1 == en2
                && v1 == v2
                && f1.len() == f2.len()
                && f1.iter().zip(f2.iter()).all(|(a, b)| values_equal(a, b))
        }
        _ => false,
    }
}

fn strip_paren(e: &Expr) -> &Expr {
    match e {
        Expr::Paren(inner) => strip_paren(inner),
        other => other,
    }
}

fn is_system_out(e: &Expr) -> bool {
    match e {
        Expr::Field(inner, name) if name == "out" => {
            matches!(&**inner, Expr::Ident(n) if n == "System")
        }
        _ => false,
    }
}

/// 按 Rust 风格格式串拼接：`{}` 依次取 vals，`{{` / `}}` 输出字面花括号。
/// 占位符与 vals 个数不一致、或花括号不成对时返回 `None`。
fn format_with(fmt: &str, vals: &[String]) -> Option<String> {
    let mut out = String::new();
    let mut vals = vals.iter();
    let mut it = fmt.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' => match it.peek() {
                Some('{') => {
                    it.next();
                    out.push('{');
                }
                Some('}') => {
                    it.next();
                    out.push_str(vals.next()?);
                }
                _ => return None,
            },
            '}' => match it.peek() {
                Some('}') => {
                    it.next();
                    out.push('}');
                }
                _ => return None,
            },
            _ => out.push(c),
        }
    }
    if vals.next().is_some() {
        return None;
    }
    Some(out)
}

fn collect_vars(name: &str, value: &Value, ty: Option<&Type>, depth: usize, out: &mut Vec<VarView>) {
    if out.len() > 200 {
        return;
    }
    let ty_text = match ty {
        Some(t) => type_str(t),
        None => value.type_name(),
    };
    out.push(VarView {
        name: name.to_string(),
        value: value.debug_repr(),
        ty: ty_text,
    });
    if depth == 0 {
        return;
    }
    match value {
        Value::Obj(obj) => {
            let inst = obj.borrow();
            for (field, slot) in &inst.fields {
                let path = format!("{}.{}", name, field);
                if slot.moved {
                    out.push(VarView {
                        name: path,
                        value: "<已 move>".to_string(),
                        ty: type_str(&slot.ty),
                    });
                } else {
                    collect_vars(&path, &slot.value, Some(&slot.ty), depth - 1, out);
                }
            }
        }
        Value::List(items) => {
            let items = items.borrow();
            for (i, item) in items.iter().enumerate().take(10) {
                collect_vars(&format!("{}[{}]", name, i), item, None, depth - 1, out);
            }
        }
        _ => {}
    }
}
