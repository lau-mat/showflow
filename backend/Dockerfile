# Stage 1: Build only the API, using the same distribution as the runtime.
FROM rust:1-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --locked -p showflow-backend

# Stage 2: Minimal runtime image
FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /app/target/release/showflow-backend /app/showflow-backend

EXPOSE 8080
CMD ["./showflow-backend"]