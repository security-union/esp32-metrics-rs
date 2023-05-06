# Rust ESP32 Metrics Platform

## Esp Rust Dependencies

Check [esp-rs/rust-build](https://github.com/esp-rs/rust-build#espup-installation) for the most up-to-date setup guide

1. Install espup
    ```sh
    cargo install espup
    ```
1. Add this to your `.zshrc` or `.bashrc`
    ```sh
    . $HOME/export-esp.sh
    ```
1. Open a new terminal

## Running the axum server with docker-compose

```sh
docker-compose up --build
```
