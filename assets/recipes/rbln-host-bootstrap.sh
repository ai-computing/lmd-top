#!/bin/sh
# Prepare a bare ubuntu image to run the RBLN compile recipe against the node's own
# rebel-compiler install (hostPath), then hand over to it. Mounted from a ConfigMap.
#
# Only used when no LMD_COMPILE_IMAGE_RBLN is configured. tzdata is needed because
# pandas -> pytz reads /usr/share/zoneinfo, which minimal images do not ship.
set -e

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null 2>&1
apt-get install -y -qq --no-install-recommends \
  python3.10 libnuma1 libgomp1 ca-certificates tzdata >/dev/null 2>&1
ln -sf /usr/bin/python3.10 /usr/local/bin/python3

exec python3 /scripts/compile.py
