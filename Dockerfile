FROM rust:alpine@sha256:bea885d2711087e67a9f7a7cd1a164976f4c35389478512af170730014d2452a AS builder

# specify our build directory
WORKDIR /usr/src/passage

# copy the source files into the engine
COPY . .

# install dev dependencies and perform build process
RUN set -eux \
 && apk add --no-cache musl-dev \
 && cargo build --release


FROM scratch

# declare our metrics port
EXPOSE 8080

# declare our minecraft port
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
