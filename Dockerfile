FROM docker.io/library/rust:1 AS builder

WORKDIR /app/

COPY ./src/ /app/src/
COPY ./Cargo.toml /app/Cargo.toml
COPY ./Cargo.lock /app/Cargo.lock
COPY ./LICENSE /app/LICENSE
COPY ./README.md /app/README.md

RUN cargo build --release

FROM gcr.io/distroless/cc-debian13

COPY --from=builder /app/target/release/rshs /usr/bin/rshs
COPY --from=builder /app/LICENSE /usr/share/doc/rshs/LICENSE
COPY --from=builder /app/README.md /usr/share/doc/rshs/README.md

WORKDIR /mnt/data/

ENV RSHS_LOG=info RSHS_ROOT_DIR=/mnt/data/

EXPOSE 8080/tcp
VOLUME /mnt/data/

ENTRYPOINT ["/usr/bin/rshs"]
