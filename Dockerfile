FROM rust:1.88-alpine@sha256:9dfaae478ecd298b6b5a039e1f2cc4fc040fc818a2de9aa78fa714dea036574d AS builder

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

# copy the raw binary into the new image
COPY --from=builder "/usr/src/passage/target/release/passage" "/passage"

# copy the users and groups for the nobody user and group
COPY --from=builder "/etc/passwd" "/etc/passwd"
COPY --from=builder "/etc/group" "/etc/group"

# we run with minimum permissions as the nobody user
USER nobody:nobody

# just execute the raw binary without any wrapper
ENTRYPOINT ["/passage"]
