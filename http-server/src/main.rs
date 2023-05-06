use std::os::fd::IntoRawFd;

use base64::Engine;
use axum::{extract::State, http::{StatusCode, HeaderValue, header}, routing::{get, post}, Json, Router, response::{Html, IntoResponse}};
use chrono::{DateTime, Utc, NaiveDateTime};
use log::info;
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, Row};
use sqlx::types::time::OffsetDateTime;
use types::{HttpResponseBody, MetricRequestBody, Topic};

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let db_connection_str = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");


    // setup connection pool
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_connection_str)
        .await
        .expect("can't connect to database");

    // run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("can't run migrations");

    // build our application with a route
    let app = Router::new()
        .route("/favicon.ico", get(favicon))
        .route("/", get(root))
        .route("/metrics", get(select_metrics))
        .route("/metric", post(insert_metric))
        .with_state(pool);

    info!("Listening on 0.0.0.0:3000");
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

type Resp = Result<(StatusCode, Json<HttpResponseBody>), (StatusCode, Json<HttpResponseBody>)>;

async fn root() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html>
<head>
    <title>ESP32 Metric Frontend</title>
</head>
<body>
    <h1>ESP32 Metric Frontend</h1>
    <button onclick="makeRequest()">Get Metrics</button>
    <div id="metrics"></div>
    <script>
    function makeRequest() {
        fetch("http://" + window.location.host + "/metrics").then((response) => {
            if (response.status != 200) {
                console.log("Error: " + response.status);
            } else {
                console.log("Success");
            }
            return response.json();
        }).then((data) => {
            if ('message' in data) {
                let decoder = new TextDecoder('UTF-8');
                let array = new Uint8Array(data.message);
                let message = decoder.decode(array);
                console.log("Error: " + message);
                return;
            }
            updateMetrics(data);
        }).catch((error) => {
            console.log(error);
        });
    }
    function updateMetrics(metrics) {
        if (metrics === undefined) {
            return;
        }
        var metricsDiv = document.getElementById("metrics");
        metricsDiv.innerHTML = "<p>Humidity: " + metrics.humidity + "</p>" +
                            "<p>Temperature: " + metrics.temperature_celsius + "°C</p>" +
                            "<p>CO2 PPM: " + metrics.co2_ppm + "</p>" +
                            "<p>Device ID: " + metrics.device_id + "</p>" +
                            "<p>Device Timestamp: " + new Date(metrics.device_timestamp * 1000).toLocaleString() + "</p>";
    }
    </script>
</body>
</html>"#)
}

// NOTE: State must be the first argument
async fn insert_metric(State(pool): State<PgPool>, Json(payload): Json<MetricRequestBody>) -> Resp {
    info!("Received metric: {:?}", payload);
    match payload.topic {
        Topic::Climate(data) => {
            sqlx::query("INSERT INTO climate_metrics (device_id, device_timestamp, temperature_celsius, humidity, co2_ppm) VALUES ($1, $2, $3, $4, $5)")
                .bind(payload.device_id)
                .bind(OffsetDateTime::from_unix_timestamp(payload.timestamp).unwrap())
                .bind(data.temperature_celsius)
                .bind(data.humidity)
                .bind(data.co2_ppm)
                .execute(&pool)
                .await
                .map_err(internal_error)?;
            let response = HttpResponseBody {
                message: "Climate data inserted".to_string().into_bytes(),
            };
            Ok((StatusCode::CREATED, Json(response)))
        }
    }
}

#[derive(sqlx::FromRow)]
struct ClimateMetricRow {
    device_id: Vec<u8>,
    device_timestamp: NaiveDateTime,
    temperature_celsius: f64,
    humidity: f64,
    co2_ppm: i32,
    created_at: NaiveDateTime
}

#[derive(Serialize, Deserialize)]
struct ClimateMetric {
    device_id: Vec<u8>,
    device_timestamp: i64,
    temperature_celsius: f64,
    humidity: f64,
    co2_ppm: i32,
}

async fn select_metrics(State(pool): State<PgPool>) -> Result<(StatusCode, Json<ClimateMetric>), (StatusCode, Json<HttpResponseBody>)> {
    let row = sqlx::query_as::<Postgres, ClimateMetricRow>("select distinct on (device_id) * from climate_metrics order by device_id, device_timestamp desc")
        .fetch_one(&pool)
        .await
        .map_err(internal_error)?;
    let metric = ClimateMetric {
        device_id: row.device_id,
        device_timestamp: row.device_timestamp.timestamp(),
        temperature_celsius: row.temperature_celsius,
        humidity: row.humidity,
        co2_ppm: row.co2_ppm,
    };
    Ok((StatusCode::OK, Json(metric)))
}

async fn favicon() -> impl IntoResponse {
    // one pixel favicon generated from https://png-pixel.com/
    let one_pixel_favicon = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mPk+89QDwADvgGOSHzRgAAAAABJRU5ErkJggg==";
    let pixel_favicon = base64::prelude::BASE64_STANDARD.decode(one_pixel_favicon).unwrap();
    ([(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))], pixel_favicon)
}

/// Utility function for mapping any error into a `500 Internal Server Error`
/// response.
fn internal_error<E>(err: E) -> (StatusCode, Json<HttpResponseBody>)
where
    E: std::error::Error,
{
    let response = HttpResponseBody {
        message: err.to_string().into_bytes(),
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(response))
}
