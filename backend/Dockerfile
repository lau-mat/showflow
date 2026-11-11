# Stage 1: Build binary using the latest stable Rust
FROM rust:latest AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

# Stage 2: Minimal runtime image
FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /app/target/release/showflow-backend /app/showflow-backend

EXPOSE 8080
CMD ["./showflow-backend"]