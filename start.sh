#!/usr/bin/env bash
# Arranque de un solo comando para uso personal/productivo local.
# Carga los secretos desde .env (nunca desde amatl.toml) y levanta el
# servidor (UI + API + MCP) con la configuración real del operador.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

if [ ! -f .env ]; then
  echo "Falta .env con AMATL_SERVER_TOKEN y MARGINALIA_API_KEY" >&2
  exit 1
fi

set -a
source .env
set +a

exec cargo run --release -p amatl-cli -- --config-file amatl.toml serve
