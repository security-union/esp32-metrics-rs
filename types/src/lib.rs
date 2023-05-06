use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct HttpResponseBody {
    /// String encoded to bytes
    pub message: Vec<u8>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MetricRequestBody {
    pub topic: Topic,
    /// Unix timestamp in seconds this will overflow in the year 2106
    pub timestamp: i64,
    pub device_id: Vec<u8>,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum Topic {
    Climate(Climate),
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Climate {
    pub temperature_celsius: f32,
    pub humidity: f32,
    pub co2_ppm: i32,
}
