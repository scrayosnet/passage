FROM rust:alpine@sha256:126df0f2a57e675f9306fe180b833982ffb996e90a92a793bb75253cfeed5475 AS builder

# specify our build directory
WORKDIR /usr/src/passage

# copy the source files into the engine
COPY . .

# install dev dependencies and perform build process
RUN set -eux \
 && apk add --no-cache libressl-dev musl-dev protobuf-dev protoc \
 && cargo build --release


FROM scratch

# declare our minecraft and metrics ports
EXPOSE 25565 8080

# copy the raw binary into the new image
COPY --from=builder "/usr/src/passage/target/release/passage" "/passage"

# copy the users and groups for the nobody user and group
COPY --from=builder "/etc/passwd" "/etc/passwd"
COPY --from=builder "/etc/group" "/etc/group"

# we run with minimum permissions as the nobody user
USER nobody:nobody

# just execute the raw binary without any wrapper
ENTRYPOINT ["/passage"]
