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
        let mut s = format!("运行时错误: {}:{} - {}", path, self.line, self.message);
        if let Some(h) = &self.hint {
            s.push_str(&format!("\n  提示: {}", h));
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
    Obj(Rc<RefCell<Instance>>),
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

impl Value {
    /// Copy 类型赋值不移动；其余类型赋值即 move。
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
            Value::List(_) | Value::Obj(_) => self.debug_repr(),
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
            Value::Obj(obj) => return obj.borrow().class.clone(),
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
        Type::Array(inner) => format!("{}[]", type_str(inner)),
        Type::Named(name) => name.clone(),
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
        Type::Named(_) | Type::Void => Value::Unit,
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
        Interp {
            prog,
            classes,
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
            if !matches!(st, Stmt::While { .. } | Stmt::ForEach { .. }) {
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
            Stmt::Let { ty, name, init, .. } => {
                let v = self.eval_place_take(init)?;
                self.declare(name, ty.clone(), v);
                Ok(Flow::Normal)
            }
            Stmt::Assign { target, value, .. } => {
                let v = self.eval_place_take(value)?;
                self.assign(target, v)?;
                Ok(Flow::Normal)
            }
            Stmt::Expr { expr, .. } => {
                self.eval(expr)?;
                Ok(Flow::Normal)
            }
            Stmt::Return { value, .. } => {
                let v = match value {
                    Some(e) => self.eval(e)?,
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
            Stmt::Break { .. } => Ok(Flow::Break),
            Stmt::Continue { .. } => Ok(Flow::Continue),
        }
    }

    // ---------- 变量 ----------

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
            Expr::Ident(name) => self.read_var(name),
            Expr::This => self.read_this(),
            Expr::Field(recv, name) => {
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
            Expr::New { class, args } => self.eval_new(class, args),
            Expr::Call { recv, name, args } => self.eval_call(recv.as_deref(), name, args),
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
                let eq = match (a, b) {
                    (Value::Int(x), Value::Int(y)) => x == y,
                    (Value::Double(x), Value::Double(y)) => x == y,
                    (Value::Int(x), Value::Double(y)) => (*x as f64) == *y,
                    (Value::Double(x), Value::Int(y)) => *x == (*y as f64),
                    (Value::Str(x), Value::Str(y)) => x == y,
                    (Value::Bool(x), Value::Bool(y)) => x == y,
                    (Value::Char(x), Value::Char(y)) => x == y,
                    (Value::Unit, Value::Unit) => true,
                    _ => false,
                };
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
            out.push(self.eval_place_take(a)?);
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
                self.eval(&args[0])?.display()
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
                    vals.push(self.eval(a)?.display());
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
                    other => Err(RuntimeError::new(
                        line,
                        format!("List 没有方法 `{}`", other),
                    )),
                },
                Value::Obj(_) => {
                    let class = match &rv {
                        Value::Obj(obj) => obj.borrow().class.clone(),
                        _ => unreachable!(),
                    };
                    let decl: &'a ClassDecl = match self.classes.get(class.as_str()) {
                        Some(d) => *d,
                        None => return Err(RuntimeError::new(line, format!("未知的类 `{}`", class))),
                    };
                    let method = match decl.methods.iter().find(|m| m.name == name) {
                        Some(m) => m,
                        None => {
                            return Err(RuntimeError::new(
                                line,
                                format!("{} 没有方法 `{}`", class, name),
                            ))
                        }
                    };
                    let argv = self.eval_args(args)?;
                    self.check_arity(name, method.params.len(), argv.len(), line)?;
                    self.call_method(&class, method, Some(rv.clone()), argv)
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

    fn call_method(
        &mut self,
        class: &str,
        method: &Method,
        this: Option<Value>,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let mut scope = Vec::new();
        for (p, v) in method.params.iter().zip(args.into_iter()) {
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
        });
        let r = self.exec_block(&method.body);
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

fn stmt_line(s: &Stmt) -> usize {
    match s {
        Stmt::Let { line, .. }
        | Stmt::Assign { line, .. }
        | Stmt::Expr { line, .. }
        | Stmt::Return { line, .. }
        | Stmt::If { line, .. }
        | Stmt::While { line, .. }
        | Stmt::ForEach { line, .. }
        | Stmt::Break { line }
        | Stmt::Continue { line } => *line,
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
