// L4 手写参考实现：与 `jrust build tests/l4/src` 的产物做逐字节行为对拍
// （SPEC §15 L4 / JR-G13）。域逻辑、状态码、响应体、计数规则都与 jrust 版本一致，
// 区别只在实现手法：这里用 AtomicI32 计数，jrust 用 `@shared` + `synchronized`。
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

struct Gate {
    password: String,
    hits: AtomicI32,
    ticks: AtomicI32,
}

impl Gate {
    fn new(password: &str) -> Self {
        Self {
            password: password.to_string(),
            hits: AtomicI32::new(0),
            ticks: AtomicI32::new(0),
        }
    }

    fn ok(&self, given: &str) -> bool {
        given == self.password
    }

    fn token(&self) -> String {
        "token:dev-1".to_string()
    }

    fn deny(&self) -> String {
        "deny:bad-password".to_string()
    }

    fn known(&self, id: &str) -> bool {
        id == "dev-1" || id == "dev-2"
    }

    fn device(&self, id: &str) -> String {
        format!("device:{}:online", id)
    }

    fn touch(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    fn beat(&self) {
        self.ticks.fetch_add(1, Ordering::Relaxed);
    }

    fn stats(&self) -> String {
        format!(
            "hits={} ticks={}",
            self.hits.load(Ordering::Relaxed),
            self.ticks.load(Ordering::Relaxed)
        )
    }
}

struct Store {
    config: HashMap<String, String>,
}

impl Store {
    fn demo() -> Self {
        let mut config = HashMap::new();
        config.insert("region".to_string(), "cn-north-1".to_string());
        config.insert("plan".to_string(), "team".to_string());
        Self { config }
    }

    fn get(&self, key: &str) -> String {
        self.config.get(key).cloned().unwrap()
    }
}

#[derive(Clone)]
pub struct AppState {
    gate: Arc<Gate>,
    store: Arc<Store>,
}

fn build_app(state: AppState) -> axum::Router {
    axum::Router::new()
        .route("/health", axum::routing::get(health))
        .route("/login", axum::routing::post(login))
        .route("/devices/{id}", axum::routing::get(device))
        .route("/config/{key}", axum::routing::get(config))
        .route("/stats", axum::routing::get(stats))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

async fn login(
    axum::extract::State(st): axum::extract::State<AppState>,
    body: String,
) -> (axum::http::StatusCode, String) {
    let pwd = body
        .trim()
        .strip_prefix("password=")
        .unwrap_or_default()
        .to_string();
    st.gate.touch();
    if st.gate.ok(pwd.as_str()) {
        (axum::http::StatusCode::OK, st.gate.token())
    } else {
        (
            axum::http::StatusCode::UNAUTHORIZED,
            st.gate.deny(),
        )
    }
}

async fn device(
    axum::extract::State(st): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> (axum::http::StatusCode, String) {
    st.gate.touch();
    if st.gate.known(id.as_str()) {
        (axum::http::StatusCode::OK, st.gate.device(id.as_str()))
    } else {
        (
            axum::http::StatusCode::NOT_FOUND,
            "deny:unknown-device".to_string(),
        )
    }
}

async fn config(
    axum::extract::State(st): axum::extract::State<AppState>,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> (axum::http::StatusCode, String) {
    st.gate.touch();
    let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        st.store.get(key.as_str())
    }));
    match got {
        Ok(body) => (axum::http::StatusCode::OK, body),
        Err(_) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "panic:missing-config-key".to_string(),
        ),
    }
}

async fn stats(axum::extract::State(st): axum::extract::State<AppState>) -> String {
    st.gate.stats()
}

#[tokio::main]
async fn main() {
    let port = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "18082".to_string());
    let addr = format!("127.0.0.1:{}", port);

    let gate = Arc::new(Gate::new("s3cret"));
    let store = Arc::new(Store::demo());
    println!("l4 service binding {}", addr);

    let bg = gate.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            bg.beat();
        }
    });

    let listener = tokio::net::TcpListener::bind(addr.as_str()).await.unwrap();
    println!("l4 service listening on {}", addr);
    let state = AppState {
        gate: gate.clone(),
        store: store.clone(),
    };
    axum::serve(listener, build_app(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
    println!("l4 service shutdown clean");
}
