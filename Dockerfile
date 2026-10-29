# syntax=docker/dockerfile:1.27

# Set to true to build the FIPS image, relying on the AWS-LC FIPS module
ARG FIPS=false

FROM rust:1.96.0-alpine AS builder
ARG FIPS

# For layer optimisation, start by installing resources not dependant from source code
RUN <<EOT
  set -euxo pipefail

  apk --no-cache add musl-dev
  # Go, Perl, CMake, clang and the Linux headers are required to build the AWS-LC FIPS module
  if [ "$FIPS" = "true" ]; then
    apk --no-cache add build-base linux-headers cmake perl go clang
  fi
  rm /var/log/apk.log
EOT

WORKDIR /opt/xtm-composer
COPY . .

# Build the application in release mode
RUN <<EOT
  set -euxo pipefail

  if [ "$FIPS" = "true" ]; then
    # GCC 14 and later fail to build the AWS-LC-FIPS 3 module on x86_64 (aws/aws-lc#2010)
    CC=clang CXX=clang++ cargo build --release --features fips
  else
    cargo build --release
  fi
EOT

# Start a new build stage with Alpine
FROM alpine:latest AS runtime

# The binary is statically linked and uses rustls: no OpenSSL is needed at runtime.
# Keep the CA bundle, then purge every package that pulls libssl3/libcrypto3.
RUN <<EOT
  set -euxo pipefail

  apk --no-cache upgrade
  apk --no-cache add ca-certificates-bundle
  apk --no-cache --purge del apk-tools libapk ssl_client libssl3 libcrypto3
  rm /var/log/apk.log
EOT

# Copy the required files from the builder stage to the current stage
WORKDIR /opt/xtm-composer
COPY --from=builder /opt/xtm-composer/target/release/xtm-composer /usr/local/bin/xtm-composer
COPY --from=builder /opt/xtm-composer/config/default.yaml /opt/xtm-composer/config/default.yaml

# Define the env to production
ENV COMPOSER_ENV=production

# Expose and entrypoint
COPY entrypoint.sh /
RUN chmod +x /entrypoint.sh
ENTRYPOINT ["/entrypoint.sh"]

# Final image, selected by the FIPS build argument
FROM runtime AS runtime-false

FROM runtime AS runtime-true
# Deploy the FIPS connector images
ENV MANAGER__IMAGE_TAG_SUFFIX=-fips

FROM runtime-${FIPS}
