# Use the official Rust image as a base
FROM rust:latest

# Install cargo watch for development
RUN cargo install cargo-watch

# Set the working directory to /app
WORKDIR /app

# Start the Rust application using cargo watch
CMD cargo watch -x run

