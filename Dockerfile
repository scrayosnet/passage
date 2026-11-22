FROM rust:1.89-alpine@sha256:4b800f2e72e04be908e5f634c504c741bd943b763d1d8ad7b096cc340e1b5b46 AS builder

# specify rust features
ARG FEATURES="default"

# specify our build directory
WORKDIR /usr/src/passage

# copy the source files into the engine
COPY . .

# install dev dependencies and perform build process
RUN set -eux \
 && apk add --no-cache libressl-dev musl-dev protobuf-dev protoc \
 && cargo build --release --features "${FEATURES}"


FROM scratch

# declare our minecraft and metrics ports
EXPOSE 25565

# copy the ssl certificates
COPY --from=builder "/etc/ssl/certs/ca-certificates.crt" "/etc/ssl/certs/ca-certificates.crt"

# copy the raw binary into the new image
COPY --from=builder "/usr/src/passage/target/release/passage" "/passage"

# copy the users and groups for the nobody user and group
COPY --from=builder "/etc/passwd" "/etc/passwd"
COPY --from=builder "/etc/group" "/etc/group"

# we run with minimum permissions as the nobody user
USER nobody:nobody

# just execute the raw binary without any wrapper
ENTRYPOINT ["/passage"]
