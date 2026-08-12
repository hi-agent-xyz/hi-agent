# Stage 1: build the SPA
FROM node:22-alpine AS web
WORKDIR /web
COPY src/appearance/web/package.json src/appearance/web/package-lock.json ./
RUN npm ci
COPY src/appearance/web ./
RUN npm run build

# Stage 2: build the Rust binary (embeds SPA)
FROM rust:1-trixie AS rust
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends cmake libclang-dev \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock build.rs .env.example ./
COPY src ./src
COPY --from=web /web/dist ./src/appearance/web/dist
RUN cargo build --release

# Stage 3: minimal runtime
FROM debian:trixie-slim
# `unzip`: Chrome for Testing publishes the headless browser the view renderer
# drives as a `.zip`, and GNU tar (unlike the bsdtar macOS/Windows ship as `tar`)
# cannot read one. Without this the browser provisioner has no extractor here.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates unzip \
    && rm -rf /var/lib/apt/lists/*
COPY --from=rust /build/target/release/hi-agent /usr/local/bin/hi-agent
# A core in a container is reached from off its machine by definition — a
# published port is not loopback — so it accepts on the gated acceptor and
# nothing here is answered without a credential. The loopback listener keeps the
# default port for the codex subprocesses and the renderer, which is why the two
# cannot be the same number. The first-boot credential is printed to the log
# once; pair with it, then revoke it.
ENV HI_AGENT_OFF_BOX=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/hi-agent"]
