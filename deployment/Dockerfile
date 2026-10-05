FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p remote-service

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home rdc \
    && mkdir /data && chown rdc:rdc /data
COPY --from=build /src/target/release/remote-service /usr/local/bin/remote-service
USER 10001:10001
ENV REMOTE_SERVICE_BIND=0.0.0.0:8787 REMOTE_SERVICE_DATABASE=/data/service.db
VOLUME /data
EXPOSE 8787
ENTRYPOINT ["remote-service"]
