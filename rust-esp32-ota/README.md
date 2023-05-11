# Rust ESP32 OTA Server
> WIP - Not ready for use

This project is a simple http server that serves an html page on `/` and accepts an esp32 image for flashing on `POST /ota`.
The server attempts to write the bytes and flash the esp32 with the image, but the magic byte is incorrect at the moment, that is why it is labeled a WIP.
