#!/usr/bin/env bash
# Cloudflare Pages calls this path from its saved build configuration.
exec bash "$(dirname "$0")/../scripts/package-site.sh" "$@"
