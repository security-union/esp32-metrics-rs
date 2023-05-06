CREATE TABLE climate_metrics (
    device_id bytea NOT NULL,
    device_timestamp timestamp,
    temperature_celsius float,
    humidity float,
    co2_ppm int,
    created_at timestamp DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (device_id, device_timestamp)
);
