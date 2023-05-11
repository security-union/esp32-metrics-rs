#![allow(deprecated)]

use anyhow::{anyhow, Result};
use embedded_svc::{ipv4, wifi::*};
use esp_idf_hal::{
    delay::FreeRtos,
    i2c::{config::Config, I2cDriver, I2C0},
    peripheral,
    prelude::*,
};
use esp_idf_svc::{eventloop::*, log::EspLogger, netif::*, ping, sntp::*, wifi::*};
use esp_idf_sys::MACSTR;
use log::{debug, error, info, warn};
use scd4x::scd4x::Scd4x;
use std::{
    env,
    io::{Read, Write},
    net::{Ipv4Addr, TcpStream},
    thread,
    time::*,
};
use types::{Climate, MetricRequestBody, Topic};

const SSID: &str = env!("WIFI_SSID");
const PASS: &str = env!("WIFI_PASSWORD");
const TCP_SERVER: &str = env!("ESP_TCP_SERVER");
const LOOP_DELAY_MS: &str = env!("ESP_LOOP_DELAY_MS");

type HttpResponse = (u16, Vec<String>, Vec<String>);

fn post_metric(body: MetricRequestBody) -> Result<HttpResponse> {
    info!("Posting metric to {TCP_SERVER}...", TCP_SERVER = TCP_SERVER);
    let mut stream = TcpStream::connect(TCP_SERVER)?;
    let body = serde_json::to_string(&body)?;
    let request = format!(
        "POST /metric HTTP/1.1\r\n\
        Host: {server}\r\n\
        User-Agent: esp32\r\n\
        Accept: */*\r\n\
        Content-Type: application/json\r\n\
        Content-Length: {size}\r\n\
        \r\n\
        {body}\r\n",
        server = TCP_SERVER,
        size = body.len(),
        body = body
    );
    stream.write_all(request.as_bytes())?;
    let mut result = Vec::new();
    let mut num_bytes_read;
    let mut iterations = 0;
    loop {
        result.resize(1024, 0);
        num_bytes_read = stream.read(&mut result)?;
        std::thread::sleep(Duration::from_millis(100));
        if num_bytes_read > 0 || iterations > 10 {
            break;
        } else {
            iterations += 1;
        }
    }
    drop(stream);
    let result = std::str::from_utf8(&result)?;
    if num_bytes_read == 0 {
        return Err(anyhow!(
            "No bytes read from {TCP_SERVER}",
            TCP_SERVER = TCP_SERVER
        ));
    }
    let mut lines = result.lines();
    let status_line = lines.next().unwrap();
    let status_line = status_line.split(' ').collect::<Vec<_>>();
    let status_code = status_line[1].parse::<u16>()?;
    let mut headers = Vec::new();
    for line in lines.clone() {
        if line.is_empty() {
            break;
        }
        headers.push(line.to_string());
    }
    let mut body = Vec::new();
    let mut start_body = false;
    for line in lines {
        if line.is_empty() {
            start_body = true;
            continue;
        }
        if start_body {
            body.push(line.to_string());
        }
    }
    Ok((status_code, headers, body))
}

fn unix_now() -> Duration {
    let start = SystemTime::now();
    start
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
}

fn main() -> Result<()> {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();
    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;
    let sysloop = EspSystemEventLoop::take()?;

    // Ntp Time Sync
    let sntp = EspSntp::new_default()?;
    let mut time_synced = false;

    // Connect to Wifi
    let mut wifi = wifi(peripherals.modem, sysloop)?;

    // Create OtaServer
    let _ota_server = OtaServer::new()?;

    let i2c_config = Config {
        baudrate: Hertz(115200),
        scl_pullup_enabled: false,
        sda_pullup_enabled: false,
    };
    let sda_pin = pins.gpio21;
    let scl_pin = pins.gpio22;
    let i2c_driver = unsafe { I2cDriver::new(I2C0::new(), sda_pin, scl_pin, &i2c_config).unwrap() };
    let mut scd4x_sensor = scd4x_sensor(i2c_driver)?;

    info!("Starting SCD4x low power periodic measurements...");
    scd4x_sensor
        .start_low_power_periodic_measurements()
        .map_err(|e| anyhow!("Failed to start low power periodic measurements: {:?}", e))?;

    info!("Starting loop with {}ms delay...", LOOP_DELAY_MS);
    loop {
        thread::sleep(Duration::from_millis(
            LOOP_DELAY_MS
                .parse()
                .expect("ESP_LOOP_DELAY_MS must be a number"),
        ));
        if !scd4x_sensor
            .data_ready_status()
            .map_err(|e| anyhow!("Failed to get data ready status: {:?}", e))?
        {
            debug!("No data ready yet, skipping loop iteration");
            continue;
        }
        let time_sync_status = sntp.get_sync_status();
        if time_sync_status != SyncStatus::Completed && !time_synced {
            warn!("NTP sync not completed yet, skipping loop iteration");
            continue;
        } else {
            time_synced = true;
        }
        if !wifi.is_connected()? {
            warn!("Wifi not connected yet, skipping loop iteration");
            wifi.connect()?;
            continue;
        }

        let mut climate = Climate::default();
        let data = scd4x_sensor
            .measurement()
            .map_err(|e| anyhow!("Failed to read measurement: {:?}", e))?;
        climate.temperature_celsius = data.temperature;
        climate.co2_ppm = data.co2.into();
        climate.humidity = data.humidity;
        let payload = MetricRequestBody {
            topic: Topic::Climate(climate),
            timestamp: unix_now().as_secs() as i64,
            device_id: MACSTR.to_vec(),
        };
        match post_metric(payload) {
            Ok((status_code, _headers, _body)) => {
                info!("POST /metric returned status code {}", status_code);
            }
            Err(e) => {
                error!("POST /metric failed: {:?}", e);
            }
        }
    }
}

fn scd4x_sensor(i2c: I2cDriver) -> Result<Scd4x<I2cDriver, FreeRtos>> {
    info!("Initializing SCD4x...");
    let mut sensor = Scd4x::new(i2c, FreeRtos);
    sensor.wake_up();
    sensor
        .stop_periodic_measurement()
        .map_err(|e| anyhow!("Failed to stop periodic measurement: {:?}", e))?;
    sensor
        .reinit()
        .map_err(|e| anyhow!("Failed to reinit sensor: {:?}", e))?;

    let serial = sensor
        .serial_number()
        .map_err(|e| anyhow!("Failed to read serial number: {:?}", e))?;
    info!("Initialized SCD4x with serial: {:#04x}", serial);

    sensor
        .self_test_is_ok()
        .map_err(|e| anyhow!("Failed to run self-test: {:?}", e))?;
    info!("SCD4x self-test passed");

    Ok(sensor)
}

fn wifi(
    modem: impl peripheral::Peripheral<P = esp_idf_hal::modem::Modem> + 'static,
    sysloop: EspSystemEventLoop,
) -> Result<Box<EspWifi<'static>>> {
    let mut wifi = Box::new(EspWifi::new(modem, sysloop.clone(), None)?);
    info!("Wifi created, about to scan");
    let ap_printlns = wifi.scan()?;
    let ours = ap_printlns.into_iter().find(|a| a.ssid == SSID);
    let channel = if let Some(ours) = ours {
        info!(
            "Found configured access point {} on channel {}",
            SSID, ours.channel
        );
        Some(ours.channel)
    } else {
        info!(
            "Configured access point {} not found during scanning, will go with unknown channel",
            SSID
        );
        None
    };
    wifi.set_configuration(&Configuration::Mixed(
        ClientConfiguration {
            ssid: SSID.into(),
            password: PASS.into(),
            channel,
            ..Default::default()
        },
        AccessPointConfiguration {
            ssid: "aptest".into(),
            channel: channel.unwrap_or(1),
            ..Default::default()
        },
    ))?;
    wifi.start()?;
    info!("Starting wifi...");
    if !WifiWait::new(&sysloop)?
        .wait_with_timeout(Duration::from_secs(20), || wifi.is_started().unwrap())
    {
        return Err(anyhow!("Wifi did not start"));
    }
    info!("Connecting wifi...");
    wifi.connect()?;
    if !EspNetifWait::new::<EspNetif>(wifi.sta_netif(), &sysloop)?.wait_with_timeout(
        Duration::from_secs(20),
        || {
            wifi.is_connected().unwrap()
                && wifi.sta_netif().get_ip_info().unwrap().ip != Ipv4Addr::new(0, 0, 0, 0)
        },
    ) {
        return Err(anyhow!(
            "Wifi did not connect or did not receive a DHCP lease"
        ));
    }
    let ip_println = wifi.sta_netif().get_ip_info()?;
    info!("Wifi DHCP println: {:?}", ip_println);
    ping(ip_println.subnet.gateway)?;
    Ok(wifi)
}

fn ping(ip: ipv4::Ipv4Addr) -> Result<()> {
    info!("About to do some pings for {:?}", ip);
    let ping_summary = ping::EspPing::default().ping(ip, &Default::default())?;
    if ping_summary.transmitted != ping_summary.received {
        return Err(anyhow!("Pinging IP {} resulted in timeouts", ip));
    }
    info!("Pinging done");
    Ok(())
}
