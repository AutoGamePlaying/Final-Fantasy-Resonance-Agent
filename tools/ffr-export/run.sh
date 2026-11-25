#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

export DOTNET_ROOT="$ROOT/.deps/dotnet"
export DOTNET_CLI_HOME="$ROOT/.deps/dotnet-home"
export NUGET_PACKAGES="$ROOT/.deps/nuget"
export NUGET_HTTP_CACHE_PATH="$ROOT/.deps/nuget-http"
export DOTNET_CLI_TELEMETRY_OPTOUT=1
export DOTNET_NOLOGO=1

exec "$ROOT/.deps/dotnet/dotnet" run -c Release --project tools/ffr-export -- "$@"
