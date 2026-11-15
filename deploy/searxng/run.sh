#!/usr/bin/env bash
# Levanta una instancia SearXNG local para la fuente `searxng` de AMATL.
#
# Por qué existe: AMATL habla con SearXNG por su API JSON
# (GET /search?q=...&format=json&pageno=1), y la imagen oficial trae `json`
# DESHABILITADO. Sin el `settings.yml` de este directorio, la instancia
# responde 403 al adapter y AMATL reporta `provider_invalid_response`. Este
# script publica la instancia en loopback con el JSON ya habilitado, que es el
# único prerequisito que AMATL no puede suplir desde su configuración.
#
# No reemplaza la ficha de gobernanza: el provider sigue necesitando
# aprobación en amatl.toml (ver docs/gobernanza-providers.md).
#
# Uso:
#   deploy/searxng/run.sh [PUERTO]        # 8888 por defecto
#   deploy/searxng/run.sh stop
#   deploy/searxng/run.sh status
set -euo pipefail

NAME="amatl-searxng"
IMAGE="searxng/searxng:latest"
PORT="${1:-8888}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ "${1:-}" = "stop" ]; then
  docker rm -f "$NAME" >/dev/null 2>&1 && echo "Instancia $NAME detenida." || echo "$NAME no estaba corriendo."
  exit 0
fi

if [ "${1:-}" = "status" ]; then
  docker ps --filter "name=$NAME" --format '{{.Names}}\t{{.Status}}\t{{.Ports}}'
  exit 0
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "docker no está disponible; instala Docker o provee una instancia SearXNG propia." >&2
  echo "Luego exporta SEARXNG_INSTANCE_URL apuntando a ella." >&2
  exit 1
fi

if ! docker info >/dev/null 2>&1; then
  echo "El daemon de Docker no responde; arráncalo y reintenta." >&2
  exit 1
fi

# Idempotente: si ya existe, se reemplaza para no dejar una instancia vieja
# sirviendo una configuración distinta a la de este directorio.
docker rm -f "$NAME" >/dev/null 2>&1 || true

docker run -d --name "$NAME" \
  -p "127.0.0.1:${PORT}:8080" \
  -v "$SCRIPT_DIR/settings.yml:/etc/searxng/settings.yml:ro" \
  "$IMAGE" >/dev/null

# Espera activa a que responda, en lugar de un sleep fijo.
for _ in $(seq 1 30); do
  if [ "$(curl -s -m 3 -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/" || true)" = "200" ]; then
    break
  fi
  sleep 1
done

# Control positivo: la API JSON debe responder y traer resultados. Un 403 aquí
# significa que `format: json` no quedó habilitado.
probe="$(curl -s -m 20 "http://127.0.0.1:${PORT}/search?q=rust&format=json" || true)"
if printf '%s' "$probe" | grep -q '"results"'; then
  count="$(printf '%s' "$probe" | grep -o '"url"' | wc -l | tr -d ' ')"
  echo "SearXNG arriba en http://127.0.0.1:${PORT} (${count} resultados en la sonda)."
  echo "Exporta: export SEARXNG_INSTANCE_URL=http://127.0.0.1:${PORT}"
  echo
  # Un canario falla antes de conectar si la instancia no está aprobada o si
  # [data_policy] no permite egress. El mensaje por defecto garantiza además que
  # la sección esté presente: sin ella, egress queda en "deny" y el canario
  # responde provider_not_approved, que no dice "falta la sección".
  echo "Para validar el adapter contra esta instancia, amatl.toml necesita:"
  echo "  1. [data_policy] con egress = \"governed\" y un perfil distinto de \"isolated\"."
  echo "  2. providers.enabled = [\"searxng\"] y la ficha completa aprobada"
  echo "     (una tabla parcial reemplaza la incorporada, no la fusiona)."
  echo "Entonces: amatl --config-file amatl.toml provider-canary searxng \"rust\" --json"
  echo "Salir con error_code=provider_not_approved significa que falta uno de los dos."
else
  echo "La instancia arrancó pero la API JSON no respondió resultados." >&2
  echo "Revisa: docker logs $NAME   (¿está 'format: json' habilitado en settings.yml?)" >&2
  exit 1
fi
