# The Internet meeting-place server. Build from the repository root:
#   docker build -f crates/ra2-server/Dockerfile -t ra2-server .
#   docker run -p 47100:47100 ra2-server
# Only the two networking crates are copied in: no game code, no game files.
FROM rust:1-slim AS build
WORKDIR /src
COPY crates/ra2-net crates/ra2-net
COPY crates/ra2-server crates/ra2-server
RUN printf '[workspace]\nmembers = ["crates/ra2-net", "crates/ra2-server"]\nresolver = "3"\n' > Cargo.toml \
 && cargo build --release -p ra2-server

FROM debian:stable-slim
COPY --from=build /src/target/release/ra2-server /usr/local/bin/ra2-server
ENV PORT=47100
EXPOSE 47100
USER nobody
ENTRYPOINT ["ra2-server"]
