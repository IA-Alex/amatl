#!/usr/bin/env bash
# Arranque de un solo comando para uso personal/local.
#
# Secretos: siguen entrando *solo* como variables de entorno, igual que en
# docs/security/secrets.md -- amatl.toml nunca contiene un valor, solo el
# nombre de la variable (server.token_env, providers.*.credential_env). Este
# script no reemplaza ese mecanismo, solo automatiza el `export` que
# README.md/DEVELOPMENT.md piden hacer a mano: si existe un .env local
# (gitignored, uso exclusivamente local) lo carga en el entorno del proceso;
# si no existe, asume que ya exportaste las variables tú mismo y continúa.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

CONFIG_FILE="${AMATL_CONFIG_FILE:-amatl.toml}"

if [ -f .env ]; then
  set -a
  source .env
  set +a
fi

# El nombre real lo declara amatl.toml (server.token_env), no está
# hardcodeado: si el operador lo renombra, este chequeo se ajusta solo en
# vez de quedar obsoleto. El token de servidor es el único secreto siempre
# obligatorio para arrancar.
token_var="AMATL_SERVER_TOKEN"
if [ -f "$CONFIG_FILE" ]; then
  found="$(grep -oE '^\s*token_env\s*=\s*"[^"]+"' "$CONFIG_FILE" \
    | head -n1 | sed -E 's/.*"(.+)"/\1/')"
  [ -n "$found" ] && token_var="$found"
fi

# Bootstrap de primer arranque: si no hay token en el entorno NI ya
# guardado en .env, se genera uno y se persiste una sola vez -- nunca se
# regenera en arranques posteriores (eso invalidaría clientes ya
# autorizados y contradice la política de "un único token activo, sin
# rotación automática" de docs/security/secrets.md). Rotarlo sigue siendo
# un acto manual y deliberado: borra la línea de .env o cambia el valor
# tú mismo y sigue el runbook de esa misma página.
if [ -z "${!token_var:-}" ]; then
  if ! command -v openssl >/dev/null 2>&1; then
    echo "Falta la variable de entorno $token_var (server.token_env en $CONFIG_FILE)." >&2
    echo "openssl no está disponible para generarla automáticamente; expórtala tú mismo." >&2
    exit 1
  fi
  generated="$(openssl rand -hex 32)"
  env_existed=false
  [ -f .env ] && env_existed=true
  touch .env
  "$env_existed" || chmod 600 .env
  printf '%s=%s\n' "$token_var" "$generated" >> .env
  export "${token_var?}=${generated}"
  echo "Generado un $token_var nuevo (primer arranque) y guardado en .env (permisos 600)." >&2
  echo "$token_var=$generated" >&2
  echo "Guárdalo ahora: no se volverá a mostrar. Para rotarlo, edita o borra esa línea en .env (ver docs/security/secrets.md)." >&2
fi

# credential_env de cada provider *habilitado* solo se advierte, no bloquea:
# algunos (p.ej. searxng) tienen valor por defecto en el propio adapter.
if [ -f "$CONFIG_FILE" ]; then
  enabled_providers="$(sed -n 's/^enabled = \[\(.*\)\]/\1/p' "$CONFIG_FILE" | tr -d '"' | tr ',' '\n' | tr -d ' ')"
  for provider in $enabled_providers; do
    cred_var="$(awk -v p="[providers.$provider]" '
      $0 == p { infile=1; next }
      /^\[/ { infile=0 }
      infile && /credential_env/ { print; exit }
    ' "$CONFIG_FILE" | sed -E 's/.*"(.+)"/\1/')"
    if [ -n "$cred_var" ] && [ -z "${!cred_var:-}" ]; then
      echo "Aviso: provider '$provider' habilitado sin $cred_var en el entorno (puede tener default propio)." >&2
    fi
  done
fi

exec cargo run --release -p amatl-cli -- --config-file "$CONFIG_FILE" serve
