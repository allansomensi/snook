# Snookered game server.
#   docker build -t snookered-server .
#   docker run -p 7878:7878 -e DATABASE_URL=postgres://… snookered-server

FROM rust:1.95-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p snookered_server \
 && cp target/release/snookered-server /snookered-server

# Distroless: no shell or package manager, runs as an unprivileged user. TLS
# roots are built into the binary (webpki), so no CA package is needed.
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /snookered-server /usr/local/bin/snookered-server
ENV PORT=7878 RUST_LOG=info
EXPOSE 7878
ENTRYPOINT ["/usr/local/bin/snookered-server"]
