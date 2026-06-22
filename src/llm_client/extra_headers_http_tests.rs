//! HTTP-layer tests: spin up a real local axum server, fire a real request,
//! assert the custom headers actually arrive at the wire level.

use super::{Message, create_client};
use axum::{Router, http::HeaderMap, routing::post};
use crate::config::{Config, OutputFormat, Provider as ConfigProvider, ProviderEntry};
use std::sync::{Arc, Mutex};

/// Bind a random local port, serve `app` in a background task, return the port.
async fn spawn_server(app: Router) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    // Yield once so the accept loop starts before we connect.
    tokio::task::yield_now().await;
    port
}

#[tokio::test]
async fn extra_headers_sent_on_every_openai_request() {
    let captured = Arc::new(Mutex::new(None::<String>));
    let cap2 = Arc::clone(&captured);

    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |headers: HeaderMap| {
            let cap = Arc::clone(&cap2);
            async move {
                *cap.lock().unwrap() = headers
                    .get("x-gomodel-user-path")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
                axum::Json(serde_json::json!({
                    "choices": [{"message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}]
                }))
            }
        }),
    );
    let port = spawn_server(app).await;

    let base_url = format!("http://127.0.0.1:{}", port);
    let mut extra = std::collections::HashMap::new();
    extra.insert("X-GoModel-User-Path".to_string(), "sanjeev/test-path".to_string());

    let mut config = Config {
        provider: ConfigProvider::OpenAi,
        provider_slug: "gomodel".to_string(),
        model: "gpt-4o".to_string(),
        api_key: "test-key".to_string(),
        base_url: Some(base_url.clone()),
        output_format: OutputFormat::Text,
        disable_stream: true,
        ..Default::default()
    };
    config.all_providers.insert("gomodel".to_string(), ProviderEntry {
        kind: Some("openai".to_string()),
        model: Some("gpt-4o".to_string()),
        api_key: Some("test-key".to_string()),
        base_url: Some(base_url),
        extra_headers: extra,
        ..Default::default()
    });

    let client = create_client(&config);
    let _ = client.send("test", &[Message::user_text("ping")], &[], None, 0).await;

    assert_eq!(
        captured.lock().unwrap().as_deref(),
        Some("sanjeev/test-path"),
        "X-GoModel-User-Path must arrive at the mock OpenAI server",
    );
}

#[tokio::test]
async fn extra_headers_sent_on_every_anthropic_request() {
    let captured = Arc::new(Mutex::new(None::<String>));
    let cap2 = Arc::clone(&captured);

    let app = Router::new().route(
        "/v1/messages",
        post(move |headers: HeaderMap| {
            let cap = Arc::clone(&cap2);
            async move {
                *cap.lock().unwrap() = headers
                    .get("x-tenant-id")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
                axum::Json(serde_json::json!({
                    "content": [{"type": "text", "text": "ok"}],
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 1, "output_tokens": 1}
                }))
            }
        }),
    );
    let port = spawn_server(app).await;

    let base_url = format!("http://127.0.0.1:{}", port);
    let mut extra = std::collections::HashMap::new();
    extra.insert("X-Tenant-ID".to_string(), "acme-corp".to_string());

    let mut config = Config {
        provider: ConfigProvider::Anthropic,
        provider_slug: "my-anthropic".to_string(),
        model: "claude-3-5-haiku-20241022".to_string(),
        api_key: "test-key".to_string(),
        base_url: Some(base_url.clone()),
        output_format: OutputFormat::Text,
        disable_stream: true,
        ..Default::default()
    };
    config.all_providers.insert("my-anthropic".to_string(), ProviderEntry {
        kind: Some("anthropic".to_string()),
        model: Some("claude-3-5-haiku-20241022".to_string()),
        api_key: Some("test-key".to_string()),
        base_url: Some(base_url),
        extra_headers: extra,
        ..Default::default()
    });

    let client = create_client(&config);
    let _ = client.send("test", &[Message::user_text("ping")], &[], None, 0).await;

    assert_eq!(
        captured.lock().unwrap().as_deref(),
        Some("acme-corp"),
        "X-Tenant-ID must arrive at the mock Anthropic server",
    );
}

#[tokio::test]
async fn no_extra_headers_sends_none() {
    // Control: without extra_headers, the custom header must not be present.
    let captured = Arc::new(Mutex::new(false));
    let cap2 = Arc::clone(&captured);

    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |headers: HeaderMap| {
            let cap = Arc::clone(&cap2);
            async move {
                *cap.lock().unwrap() = headers.get("x-gomodel-user-path").is_some();
                axum::Json(serde_json::json!({
                    "choices": [{"message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}]
                }))
            }
        }),
    );
    let port = spawn_server(app).await;

    let base_url = format!("http://127.0.0.1:{}", port);
    let mut config = Config {
        provider: ConfigProvider::OpenAi,
        provider_slug: "plain".to_string(),
        model: "gpt-4o".to_string(),
        api_key: "test-key".to_string(),
        base_url: Some(base_url.clone()),
        output_format: OutputFormat::Text,
        disable_stream: true,
        ..Default::default()
    };
    config.all_providers.insert("plain".to_string(), ProviderEntry {
        kind: Some("openai".to_string()),
        base_url: Some(base_url),
        ..Default::default()
    });

    let client = create_client(&config);
    let _ = client.send("test", &[Message::user_text("ping")], &[], None, 0).await;

    assert!(
        !*captured.lock().unwrap(),
        "X-GoModel-User-Path must NOT be sent when extra_headers is empty",
    );
}
