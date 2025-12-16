FROM rust:1.92-alpine AS builder

WORKDIR /opt/xtm-composer
COPY . .

RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static

# Build the application in release mode
RUN cargo build --release

# Start a new build stage with Alpine
FROM alpine:latest

RUN apk add --no-cache musl openssl openssl-libs-static

# Copy the required files from the builder stage to the current stage
WORKDIR /opt/xtm-composer
COPY --from=builder /opt/xtm-composer/target/release/xtm-composer /usr/local/bin/xtm-composer
COPY --from=builder /opt/xtm-composer/config/default.yaml /opt/xtm-composer/config/default.yaml

# Defiine the env to production
ENV COMPOSER_ENV=production

# Expose and entrypoint
COPY entrypoint.sh /
RUN chmod +x /entrypoint.sh
ENTRYPOINT ["/entrypoint.sh"]