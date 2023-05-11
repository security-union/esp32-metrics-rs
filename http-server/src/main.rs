use std::{fs::OpenOptions, io::Read, os::fd::IntoRawFd, path::PathBuf};

use axum::{
    extract::{Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use chrono::{DateTime, NaiveDateTime, Utc};
use log::info;
use serde::{Deserialize, Serialize};
use sqlx::types::time::OffsetDateTime;
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, Row};
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
        .route("/index.html", get(root))
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

async fn root() -> Html<String> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("web")
        .join("index.html");
    let mut file = OpenOptions::new().read(true).open(d).unwrap();
    let mut buf = String::new();
    file.read_to_string(&mut buf).unwrap();
    Html(buf)
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
    created_at: NaiveDateTime,
}

#[derive(Serialize, Deserialize)]
struct ClimateMetric {
    device_id: Vec<u8>,
    device_timestamp: i64,
    temperature_celsius: f64,
    humidity: f64,
    co2_ppm: i32,
}

#[derive(Deserialize)]
struct SelectMetricsQuery {
    start_timestamp: Option<i64>,
    end_timestamp: Option<i64>,
}

async fn select_metrics(
    State(pool): State<PgPool>,
    query: Query<SelectMetricsQuery>,
) -> Result<(StatusCode, Json<Vec<ClimateMetric>>), (StatusCode, Json<HttpResponseBody>)> {
    let query = query.0;
    let query_str = "select * from climate_metrics where device_timestamp between $1 and $2 order by device_timestamp desc";
    let rows = sqlx::query_as::<Postgres, ClimateMetricRow>(query_str)
        .bind(OffsetDateTime::from_unix_timestamp(query.start_timestamp.unwrap_or(0)).unwrap())
        .bind(
            OffsetDateTime::from_unix_timestamp(
                query.end_timestamp.unwrap_or(Utc::now().timestamp()),
            )
            .unwrap(),
        )
        .fetch_all(&pool)
        .await
        .map_err(internal_error)?;
    let metrics = rows
        .into_iter()
        .map(|row| ClimateMetric {
            device_id: row.device_id,
            device_timestamp: row.device_timestamp.timestamp(),
            temperature_celsius: row.temperature_celsius,
            humidity: row.humidity,
            co2_ppm: row.co2_ppm,
        })
        .collect();
    Ok((StatusCode::OK, Json(metrics)))
}

async fn favicon() -> impl IntoResponse {
    // one pixel favicon generated from https://png-pixel.com/
    let one_pixel_favicon = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mPk+89QDwADvgGOSHzRgAAAAABJRU5ErkJggg==";
    let pixel_favicon = base64::prelude::BASE64_STANDARD
        .decode(one_pixel_favicon)
        .unwrap();
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))],
        pixel_favicon,
    )
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
