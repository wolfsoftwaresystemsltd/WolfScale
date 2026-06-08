# WolfDisk Docker Container — distributed FUSE filesystem
#
# Runs in --network host mode with /dev/fuse. Pre-built static musl binaries
# are placed under dist/<arch>/ by the publish workflow; this Dockerfile
# copies the right one based on TARGETARCH.
#
# Usage:
#   docker run -d --name wolfdisk --network host \
#     --cap-add SYS_ADMIN --device /dev/fuse:/dev/fuse \
#     --security-opt apparmor:unconfined \
#     -v wolfdisk-config:/etc/wolfdisk \
#     -v wolfdisk-data:/var/lib/wolfdisk \
#     -v /mnt/wolfdisk:/mnt/wolfdisk:rshared \
#     -e WOLFDISK_ROLE=client \
#     -e WOLFDISK_PEERS=<leader_ip:8650> \
#     ghcr.io/wolfsoftwaresystemsltd/wolfdisk:latest

FROM debian:bookworm-slim

ARG TARGETARCH

RUN apt-get update && apt-get install -y --no-install-recommends \
    fuse3 \
    procps \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /etc/wolfdisk /var/lib/wolfdisk /mnt/wolfdisk

COPY dist/${TARGETARCH}/wolfdisk    /usr/local/bin/wolfdisk
COPY dist/${TARGETARCH}/wolfdiskctl /usr/local/bin/wolfdiskctl
COPY docker/entrypoint.sh /entrypoint.sh

RUN chmod +x /usr/local/bin/wolfdisk /usr/local/bin/wolfdiskctl /entrypoint.sh

EXPOSE 8650/tcp
EXPOSE 8651/udp
EXPOSE 9878/tcp

VOLUME ["/etc/wolfdisk", "/var/lib/wolfdisk"]

ENTRYPOINT ["/entrypoint.sh"]
