FROM rust:1.85.0-alpine AS builder

WORKDIR /opt/opencti-composer
COPY . .

RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static

# Build the application in release mode
RUN cargo build --release

# Start a new build stage with Alpine
FROM alpine:latest

RUN apk add --no-cache musl openssl openssl-libs-static

# Copy the required files from the builder stage to the current stage
WORKDIR /opt/opencti-composer
COPY --from=builder /opt/opencti-composer/target/release/opencti-composer /usr/local/bin/opencti-composer
COPY --from=builder /opt/opencti-composer/config/composer-default.yaml /opt/opencti-composer/config/default.yaml
COPY --from=builder /opt/opencti-composer/contracts /opt/opencti-composer/contracts

# Defiine the env to production
ENV COMPOSER_ENV=production

# Expose and entrypoint
COPY entrypoint.sh /
RUN chmod +x /entrypoint.sh
ENTRYPOINT ["/entrypoint.sh"]