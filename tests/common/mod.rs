#![allow(dead_code)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use kimana_backend::{build_app, config::Config, db, seed, state::AppState};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

pub struct TestApp {
    app: Router,
    pub pool: PgPool,
    pub state: AppState,
}

impl TestApp {
    pub async fn new() -> Self {
        let config = Config::test();
        let pool = db::connect(&config.database_url)
            .await
            .expect("connect to test database (is Postgres up?)");
        db::run_migrations(&pool).await.expect("migrations");
        seed::seed(&pool).await.expect("seed");
        let state = AppState {
            pool: pool.clone(),
            config: Arc::new(config),
        };
        TestApp {
            app: build_app(state.clone()),
            pool,
            state,
        }
    }

    pub async fn reseed(&self) {
        seed::seed(&self.pool).await.expect("reseed");
    }

    pub fn router_clone(&self) -> Router {
        self.app.clone()
    }

    async fn send(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let builder = Request::builder().method(method).uri(uri);
        let request = match body {
            Some(b) => builder
                .header("content-type", "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let response = self.app.clone().oneshot(request).await.unwrap();
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

        let request = Request::builder()
            .method("POST")
            .uri(uri)
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
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
