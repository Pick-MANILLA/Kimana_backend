mod common;

use axum::http::StatusCode;
use common::TestApp;
use kimana_backend::domain::fx;
use serial_test::file_serial;

#[tokio::test]
#[file_serial]
async fn seeded_providers_do_not_diverge() {
    let app = TestApp::new().await;
    app.get("/rates/indicative?send=USD&receive=NGN").await;

    let events = fx::recent_divergence_events(&app.pool, 10).await.unwrap();
    assert!(events.is_empty());
}

#[tokio::test]
#[file_serial]
async fn diverging_secondary_provider_triggers_an_alert() {
    let app = TestApp::new().await;

    // Primary is seeded at 1645.2; push the secondary far enough away to
    // exceed the default 1% threshold.
    sqlx::query("update fx_secondary_rates set rate = 1700.0 where pair = 'USD/NGN'")
        .execute(&app.pool)
        .await
        .unwrap();

    let (status, _) = app.get("/rates/indicative?send=USD&receive=NGN").await;
    assert_eq!(status, StatusCode::OK);

    let events = fx::recent_divergence_events(&app.pool, 10).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].pair, "USD/NGN");
    assert_eq!(events[0].provider_a, "primary");
    assert_eq!(events[0].provider_b, "secondary");
    assert_eq!(events[0].rate_a, 1645.2);
    assert_eq!(events[0].rate_b, 1700.0);
    assert!(events[0].divergence_percent > events[0].threshold_percent);
}

#[tokio::test]
#[file_serial]
async fn divergence_check_ignores_pairs_the_secondary_does_not_quote() {
    let app = TestApp::new().await;
    sqlx::query("delete from fx_secondary_rates where pair = 'USD/NGN'")
        .execute(&app.pool)
        .await
        .unwrap();

    let (status, _) = app.get("/rates/indicative?send=USD&receive=NGN").await;
    assert_eq!(status, StatusCode::OK);

    let events = fx::recent_divergence_events(&app.pool, 10).await.unwrap();
    assert!(events.is_empty());
}

#[tokio::test]
#[file_serial]
async fn creating_a_quote_also_checks_divergence() {
    let app = TestApp::new().await;
    sqlx::query("update fx_secondary_rates set rate = 1700.0 where pair = 'USD/NGN'")
        .execute(&app.pool)
        .await
        .unwrap();

    let (status, _) = app
        .post(
            "/quotes",
            serde_json::json!({
                "sendCurrency": "USD", "receiveCurrency": "NGN",
                "amount": { "amountMinor": 100_000, "currency": "USD" }, "amountField": "send"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let events = fx::recent_divergence_events(&app.pool, 10).await.unwrap();
    assert_eq!(events.len(), 1);
}
