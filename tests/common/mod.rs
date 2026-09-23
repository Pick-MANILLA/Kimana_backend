#![allow(dead_code)]

use axum::body::Body;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use kimana_backend::{
    build_app, config::Config, db, http::SESSION_COOKIE_NAME, seed, state::AppState,
};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Mutex;
use tower::ServiceExt;

pub struct TestApp {
    app: Router,
    pub pool: PgPool,
    pub state: AppState,
    session_cookie: Mutex<Option<String>>,
}

impl TestApp {
    pub async fn new() -> Self {
        Self::with_config(Config::test()).await
    }

    pub async fn with_config(config: Config) -> Self {
        let pool = db::connect(&config.database_url)
            .await
            .expect("connect to test database (is Postgres up?)");
        db::run_migrations(&pool).await.expect("migrations");
        seed::seed(&pool).await.expect("seed");
        let state = AppState::new(pool.clone(), config);
        let app = TestApp {
            app: build_app(state.clone()),
            pool,
            state,
            session_cookie: Mutex::new(None),
        };
        let (status, _) = app
            .post(
                "/login",
                serde_json::json!({ "email": seed::DEMO_EMAIL, "password": seed::DEMO_PASSWORD }),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "demo login must succeed after seeding"
        );
        app
    }

    /// Reseeds and logs back in as the (re-created) demo user — reseeding
    /// truncates `sessions` along with `users`, invalidating any prior cookie.
    pub async fn reseed(&self) {
        seed::seed(&self.pool).await.expect("reseed");
        self.clear_cookie();
        let (status, _) = self
            .post(
                "/login",
                serde_json::json!({ "email": seed::DEMO_EMAIL, "password": seed::DEMO_PASSWORD }),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "demo login must succeed after reseeding"
        );
    }

    pub fn router_clone(&self) -> Router {
        self.app.clone()
    }

    /// Drops the stored session cookie, simulating a fresh, unauthenticated client.
    pub fn clear_cookie(&self) {
        *self.session_cookie.lock().unwrap() = None;
    }

    /// The current session's `Cookie` request-header value, for tests that
    /// build a raw `Request` directly instead of going through `send`.
    pub fn cookie_header(&self) -> Option<String> {
        self.session_cookie
            .lock()
            .unwrap()
            .clone()
            .map(|c| format!("{SESSION_COOKIE_NAME}={c}"))
    }

    pub async fn login(&self, email: &str, password: &str) -> (StatusCode, Value) {
        self.post(
            "/login",
            serde_json::json!({ "email": email, "password": password }),
        )
        .await
    }

    pub async fn logout(&self) -> (StatusCode, Value) {
        self.post("/logout", Value::Null).await
    }

    fn capture_cookie(&self, response: &axum::http::Response<Body>) {
        for value in response.headers().get_all(SET_COOKIE) {
            let Ok(value) = value.to_str() else { continue };
            let Some(rest) = value.strip_prefix(&format!("{SESSION_COOKIE_NAME}=")) else {
                continue;
            };
            let raw_value = rest.split(';').next().unwrap_or("");
            let mut guard = self.session_cookie.lock().unwrap();
            *guard = if raw_value.is_empty() {
                None
            } else {
                Some(raw_value.to_string())
            };
        }
    }

    async fn send(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(cookie) = self.session_cookie.lock().unwrap().clone() {
            builder = builder.header(COOKIE, format!("{SESSION_COOKIE_NAME}={cookie}"));
        }
        let request = match body {
            Some(b) => builder
                .header("content-type", "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let response = self.app.clone().oneshot(request).await.unwrap();
        self.capture_cookie(&response);
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }

    pub async fn get(&self, uri: &str) -> (StatusCode, Value) {
        self.send("GET", uri, None).await
    }
    pub async fn post(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send("POST", uri, Some(body)).await
    }
    pub async fn put(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send("PUT", uri, Some(body)).await
    }
    pub async fn delete(&self, uri: &str) -> (StatusCode, Value) {
        self.send("DELETE", uri, None).await
    }

    /// Multipart document upload (single file + fields).
    pub async fn upload(
        &self,
        uri: &str,
        fields: &[(&str, &str)],
        file: (&str, &str, &[u8]),
    ) -> (StatusCode, Value) {
        let boundary = "----kimanatestboundary";
        let mut body: Vec<u8> = Vec::new();
        for (name, value) in fields {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
                )
                .as_bytes(),
            );
        }
        let (field, filename, bytes) = file;
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\nContent-Type: {}\r\n\r\n",
                mime_for(filename)
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let mut builder = Request::builder().method("POST").uri(uri).header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        );
        if let Some(cookie) = self.session_cookie.lock().unwrap().clone() {
            builder = builder.header(COOKIE, format!("{SESSION_COOKIE_NAME}={cookie}"));
        }
        let request = builder.body(Body::from(body)).unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        self.capture_cookie(&response);
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }

    pub async fn scalar_i64(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(&self.pool).await.unwrap()
    }
}

fn mime_for(filename: &str) -> &'static str {
    if filename.ends_with(".pdf") {
        "application/pdf"
    } else if filename.ends_with(".png") {
        "image/png"
    } else if filename.ends_with(".jpg") || filename.ends_with(".jpeg") {
        "image/jpeg"
    } else {
        "text/plain"
    }
}
