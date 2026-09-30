# Moteur Linkx : compilé sur la machine qui construit l'image (target-cpu=native),
# donc à construire sur le serveur qui le fait tourner.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --bins \
 && mkdir /out \
 && cp target/release/server target/release/play /out/

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/ /app/bin/
COPY data/ /app/data/
WORKDIR /app
ENV PATH=/app/bin:$PATH PORT=21345
CMD ["server"]
