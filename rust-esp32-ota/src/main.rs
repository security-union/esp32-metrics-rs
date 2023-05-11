#![allow(deprecated)]

use anyhow::{anyhow, Result};
use embedded_svc::{
    httpd::{registry::Registry, Body, Handler, Request, Response},
    ipv4,
    wifi::*,
};
use esp_idf_hal::{peripheral, prelude::*};
use esp_idf_svc::{
    eventloop::*,
    httpd::{Server, ServerRegistry},
    log::EspLogger,
    netif::*,
    ping,
    sntp::*,
    wifi::*,
};
use log::{error, info, warn};
use rust_esp32_ota::ota::OtaUpdate;
use std::{env, io::Read, net::Ipv4Addr, thread, time::*};

const SSID: &str = env!("WIFI_SSID");
const PASS: &str = env!("WIFI_PASSWORD");
const LOOP_DELAY_MS: &str = env!("ESP_LOOP_DELAY_MS");

fn main() -> Result<()> {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();
    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take()?;

    // Ntp Time Sync
    let sntp = EspSntp::new_default()?;
    let mut time_synced = false;

    // Connect to Wifi
    let mut wifi = wifi(peripherals.modem, sysloop)?;

    // Create OtaServer
    let _ota_server = OtaServer::new()?;

    info!("Starting loop with {}ms delay...", LOOP_DELAY_MS);
    loop {
        thread::sleep(Duration::from_millis(
            LOOP_DELAY_MS
                .parse()
                .expect("ESP_LOOP_DELAY_MS must be a number"),
        ));
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
    }
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

struct OtaServer {
    #[allow(dead_code)]
    server: Server,
}

impl OtaServer {
    fn new() -> Result<OtaServer> {
        let mut registry = ServerRegistry::new();
        let configuration = esp_idf_svc::httpd::Configuration::default();
        registry = registry.handler(Handler::new(
            "/",
            #[allow(deprecated)]
            embedded_svc::httpd::Method::Get,
            #[allow(deprecated)]
            |_request: Request| {
                let mut response = Response::new(200);
                response = response.header("Content-Type", "text/html; charset=UTF-8");
                response = response.body(embedded_svc::httpd::Body::Bytes(
                    br#"<!DOCTYPE html>
<html>
  <head>
    <meta charset="UTF-8">
    <title>File Upload Form</title>
  </head>
  <body>
    <h1>File Upload Form</h1>
    <form method="post" enctype="multipart/form-data" action="" id="file-upload-form">
      <input type="file" name="file">
      <input type="submit" value="Upload">
    </form>
    <script>
      // get the form element
      const form = document.getElementById('file-upload-form');

      // set the action attribute of the form to the current host
      form.action = `http://${window.location.host}/ota`;

      // add an event listener to the form's submit button to prevent the default form submission behavior
      form.addEventListener('submit', (event) => {
        event.preventDefault();
        // add your own code to handle the form submission and file upload here

        console.log("I will upload the file now");

        // get the file input element
        const fileInput = document.querySelector('input[type=file]');

        // get the file from the input element
        const file = fileInput.files[0];

        // create a FormData object to store the file data
        const formData = new FormData();
        formData.append('file', file);

        // create the fetch request with the form data
        fetch(form.action, {
          method: 'POST',
          body: formData
        })
        .then(response => {
            console.log(response);
        })
        .catch(error => {
          console.error(error);
        });
      });
    </script>
  </body>
</html>"#
                    .to_vec(),
                ));
                Ok(response)
            },
        ))?;
        registry = registry.handler(Handler::new(
            "/ota",
            embedded_svc::httpd::Method::Post,
            |request: Request| {
                info!(
                    "Got request for OTA size {}",
                    request.content_len().unwrap()
                );
                let mut n = 0;
                const BLOCK_SIZE: usize = 4096;
                let mut bytea = [0_u8; BLOCK_SIZE];
                let mut written_blocks = 0;
                let mut count = 0;
                let mut body_started = false;
                let mut ota = OtaUpdate::begin()?;

                for byte in request.bytes() {
                    let byte = byte.unwrap();
                    if n == 4 && !body_started {
                        info!("Body started");
                        body_started = true;
                    }
                    if byte == 10 || byte == 13 {
                        n += 1;
                    } else {
                        n = 0;
                    }
                    if body_started {
                        if count == BLOCK_SIZE {
                            ota.write(&bytea)?;
                            bytea = [0_u8; BLOCK_SIZE];
                            count = 0;
                            written_blocks += 1;
                            info!("Wrote {} bytes", written_blocks * BLOCK_SIZE);
                        }
                        bytea[count] = byte;
                        count += 1;
                    } else {
                        match std::str::from_utf8(&[byte]) {
                            Ok(s) => print!("{}", s),
                            Err(_e) => {
                                continue;
                            }
                        }
                    }
                }
                info!("Done writing");
                let mut response = Response::new(200);
                // Performs validation of the newly written app image and completes the OTA update.
                let mut completed_ota = match ota.finalize() {
                    Ok(ota) => ota,
                    Err(e) => {
                        error!("Error finalizing OTA: {:?}", e);
                        response.status = 500;
                        response = response.body(Body::Bytes(b"Error finalizing OTA".to_vec()));
                        return Ok(response);
                    }
                };

                // Sets the newly written to partition as the next partition to boot from.
                match completed_ota.set_as_boot_partition() {
                    Ok(_) => {}
                    Err(e) => {
                        error!("Error setting OTA as boot partition: {:?}", e);
                        response.status = 500;
                        response = response
                            .body(Body::Bytes(b"Error setting OTA as boot partition".to_vec()));
                        return Ok(response);
                    }
                };

                // Restarts the CPU, booting into the newly written app.
                completed_ota.restart();
                Ok(response)
            },
        ))?;
        let server = registry.start(&configuration)?;
        Ok(OtaServer { server })
    }
}
